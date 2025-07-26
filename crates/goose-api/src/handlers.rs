use warp::{http::HeaderValue, Filter, Rejection, reject::custom};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use futures_util::TryStreamExt;
use tracing::{info, warn, error};
use mcp_core::tool::Tool;
use goose::agents::{extension::Envs, extension_manager::ExtensionManager, Agent, SessionConfig, AgentEvent};
use goose::message::{Message, MessageContent};
use goose::session::{self, Identifier};
use goose::config::Config;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex; // Explicitly add this import
use crate::api_sessions::{ApiSession, SESSIONS, cleanup_expired_sessions};
use std::collections::HashMap;
// Custom rejection type for anyhow::Error
#[derive(Debug)]
struct AnyhowRejection(#[allow(dead_code)] anyhow::Error);

impl warp::reject::Reject for AnyhowRejection {}

pub static EXTENSION_MANAGER: LazyLock<ExtensionManager> = LazyLock::new(|| ExtensionManager::default());
pub static AGENT: LazyLock<tokio::sync::Mutex<Agent>> = LazyLock::new(|| tokio::sync::Mutex::new(Agent::new()));

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionRequest {
    pub prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub message: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartSessionResponse {
    pub message: String,
    pub status: String,
    pub session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionReplyRequest {
    pub session_id: Uuid,
    pub prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EndSessionRequest {
    pub session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SummarizeSessionRequest {
    pub session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionsResponse {
    pub extensions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionResponse {
    pub error: bool,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub active_sessions: usize,
    pub pending_requests: HashMap<String, usize>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ExtensionConfigRequest {
    #[serde(rename = "sse")]
    Sse {
        name: String,
        uri: String,
        #[serde(default)]
        envs: Envs,
        #[serde(default)]
        env_keys: Vec<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "stdio")]
    Stdio {
        name: String,
        cmd: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        envs: Envs,
        #[serde(default)]
        env_keys: Vec<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "builtin")]
    Builtin {
        name: String,
        display_name: Option<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "frontend")]
    Frontend {
        name: String,
        tools: Vec<Tool>,
        instructions: Option<String>,
    },
}

pub async fn start_session_handler(
    req: SessionRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Starting session with prompt: {}", req.prompt);

    cleanup_expired_sessions().await;

    // create fresh agent using provider from the template agent
    let template = AGENT.lock().await;
    let new_agent = Agent::new();
    if let Ok(provider) = template.provider().await {
        let _ = new_agent.update_provider(provider).await;
    }
    drop(template);

    let mut messages = vec![Message::user().with_text(&req.prompt)];
    let session_id = Uuid::new_v4();
    let session_name = session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    let session = ApiSession::new(new_agent);
    let agent_ref = session.agent.clone();
    SESSIONS.insert(session_id, session);

    let provider = agent_ref.lock().await.provider().await.ok();

    let agent_locked = agent_ref.lock().await;
    let result = agent_locked
        .reply(
            &messages,
            Some(SessionConfig {
                id: Identifier::Name(session_name.clone()),
                working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                schedule_id: None,
            }),
        )
        .await;

    match result {
        Ok(mut stream) => {
            let mut full_response_text = String::new();
            let mut final_status = "success".to_string();

            while let Some(agent_event) = stream.try_next().await.map_err(|e| custom(AnyhowRejection(e)))? {
                let response = match agent_event {
                    AgentEvent::Message(msg) => msg,
                    _ => {
                        continue;
                    }
                };
                if matches!(response.content.first(), Some(MessageContent::ContextLengthExceeded(_))) {
                    // This block needs to be handled carefully.
                    // The `agent` here refers to the global AGENT, not the session-specific agent_ref.
                    // This might be a bug in the original code.
                    // For now, I'll keep the existing logic but note this potential issue.
                    let session_agent = agent_ref.lock().await; // Use session-specific agent
                    match session_agent.summarize_context(&messages).await {
                        Ok((summarized, _)) => {
                            messages = summarized;
                            final_status = "warning".to_string();
                            full_response_text = "Conversation summarized to fit context window".to_string();
                            // Persist summarized messages immediately
                            if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                                warn!("Failed to persist session {}: {}", session_name, e);
                            }
                            break; // Exit loop after summarization
                        }
                        Err(e) => {
                            warn!("Failed to summarize context: {}", e);
                            final_status = "error".to_string();
                            full_response_text = format!("Failed to summarize context: {}", e);
                            break; // Exit loop on summarization error
                        }
                    }
                } else {
                    let response_text = response.as_concat_text();
                    full_response_text.push_str(&response_text);
                    messages.push(response);
                }
            }

            if full_response_text.is_empty() && final_status == "success" {
                final_status = "warning".to_string();
                full_response_text = "Session started but no response generated".to_string();
            }

            // Persist all messages after the stream is fully consumed
            if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                warn!("Failed to persist session {}: {}", session_name, e);
            }

            let api_response = StartSessionResponse {
                message: full_response_text,
                status: final_status,
                session_id,
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&api_response),
                warp::http::StatusCode::OK,
            ))
        }
        Err(e) => {
            error!("Failed to start session: {}", e);
            let response = ApiResponse {
                message: format!("Failed to start session: {}", e),
                status: "error".to_string(),
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&response),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub async fn reply_session_handler(
    req: SessionReplyRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Replying to session with prompt: {}", req.prompt);

    cleanup_expired_sessions().await;

    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    let session_entry = match SESSIONS.get(&req.session_id) {
        Some(s) => s,
        None => {
            let response = ApiResponse {
                message: "Session not found".to_string(),
                status: "error".to_string(),
            };
            return Ok(warp::reply::with_status(
                warp::reply::json(&response),
                warp::http::StatusCode::NOT_FOUND,
            ));
        }
    };
    session_entry.touch();
    let agent_ref = session_entry.agent.clone();
    drop(session_entry);

    let mut messages = match session::read_messages(&session_path) {
        Ok(m) => m,
        Err(_) => {
            let response = ApiResponse {
                message: "Session not found".to_string(),
                status: "error".to_string(),
            };
            return Ok(warp::reply::with_status(
                warp::reply::json(&response),
                warp::http::StatusCode::NOT_FOUND,
            ));
        }
    };

    messages.push(Message::user().with_text(&req.prompt));

    let provider = agent_ref.lock().await.provider().await.ok();

    let agent_locked = agent_ref.lock().await;
    let result = agent_locked
        .reply(
            &messages,
            Some(SessionConfig {
                id: Identifier::Name(session_name.clone()),
                working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                schedule_id: None,
            }),
        )
        .await;

    match result {
        Ok(mut stream) => {
            let mut full_response_text = String::new();
            let mut final_status = "success".to_string();

            while let Some(agent_event) = stream.try_next().await.map_err(|e| custom(AnyhowRejection(e)))? {
                let response = match agent_event {
                    AgentEvent::Message(msg) => msg,
                    _ => {
                        continue;
                    }
                };
                if matches!(response.content.first(), Some(MessageContent::ContextLengthExceeded(_))) {
                    // This block needs to be handled carefully.
                    // The `agent` here refers to the global AGENT, not the session-specific agent_ref.
                    // This might be a bug in the original code.
                    // For now, I'll keep the existing logic but note this potential issue.
                    let session_agent = agent_ref.lock().await; // Use session-specific agent
                    match session_agent.summarize_context(&messages).await {
                        Ok((summarized, _)) => {
                            messages = summarized;
                            final_status = "warning".to_string();
                            full_response_text = "Conversation summarized to fit context window".to_string();
                            // Persist summarized messages immediately
                            if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                                warn!("Failed to persist session {}: {}", session_name, e);
                            }
                            break; // Exit loop after summarization
                        }
                        Err(e) => {
                            warn!("Failed to summarize context: {}", e);
                            final_status = "error".to_string();
                            full_response_text = format!("Failed to summarize context: {}", e);
                            break; // Exit loop on summarization error
                        }
                    }
                } else {
                    let response_text = response.as_concat_text();
                    full_response_text.push_str(&response_text);
                    messages.push(response);
                }
            }

            if full_response_text.is_empty() && final_status == "success" {
                final_status = "warning".to_string();
                full_response_text = "Reply processed but no response generated".to_string();
            }

            // Persist all messages after the stream is fully consumed
            if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                warn!("Failed to persist session {}: {}", session_name, e);
            }

            let api_response = ApiResponse {
                message: format!("Reply: {}", full_response_text),
                status: final_status,
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&api_response),
                warp::http::StatusCode::OK,
            ))
        }
        Err(e) => {
            error!("Failed to reply to session: {}", e);
            let response = ApiResponse {
                message: format!("Failed to reply to session: {}", e),
                status: "error".to_string(),
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&response),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub async fn end_session_handler(
    req: EndSessionRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    cleanup_expired_sessions().await;

    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    // remove in-memory agent if present
    if let Some((_, api_session)) = SESSIONS.remove(&req.session_id) {
        shutdown_agent_extensions(api_session.agent).await;
    }

    if std::fs::remove_file(&session_path).is_ok() {
        let response = ApiResponse {
            message: "Session ended".to_string(),
            status: "success".to_string(),
        };
        Ok(warp::reply::with_status(
            warp::reply::json(&response),
            warp::http::StatusCode::OK,
        ))
    } else {
        let response = ApiResponse {
            message: "Session not found".to_string(),
            status: "error".to_string(),
        };
        Ok(warp::reply::with_status(
            warp::reply::json(&response),
            warp::http::StatusCode::NOT_FOUND,
        ))
    }
}

pub async fn summarize_session_handler(
    req: SummarizeSessionRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Summarizing session: {}", req.session_id);

    let agent = AGENT.lock().await;

    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    let messages = match session::read_messages(&session_path) {
        Ok(m) => m,
        Err(_) => {
            let response = ApiResponse {
                message: "Session not found".to_string(),
                status: "error".to_string(),
            };
            return Ok(warp::reply::with_status(
                warp::reply::json(&response),
                warp::http::StatusCode::NOT_FOUND,
            ));
        }
    };

    let provider = agent.provider().await.ok();

    match agent.summarize_context(&messages).await {
        Ok((summarized_messages, _)) => {
            let summary_text = summarized_messages
                .first()
                .map(|m| m.as_concat_text())
                .unwrap_or_default();

            if let Err(e) = session::persist_messages(&session_path, &summarized_messages, provider.clone()).await {
                warn!("Failed to persist session {}: {}", session_name, e);
            }

            let resp = ApiResponse {
                message: summary_text,
                status: "success".to_string(),
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&resp),
                warp::http::StatusCode::OK,
            ))
        }
        Err(e) => {
            error!("Failed to summarize session: {}", e);
            let resp = ApiResponse {
                message: format!("Failed to summarize session: {}", e),
                status: "error".to_string(),
            };
            Ok(warp::reply::with_status(
                warp::reply::json(&resp),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }
    }
}

pub async fn list_extensions_handler() -> Result<impl warp::Reply, Rejection> {
    info!("Listing extensions");

    match EXTENSION_MANAGER.list_extensions().await {
        Ok(exts) => {
            let response = ExtensionsResponse { extensions: exts };
            Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
        }
        Err(e) => {
            error!("Failed to list extensions: {}", e);
            let response = ExtensionsResponse {
                extensions: vec!["Failed to list extensions".to_string()],
            };
            Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
        }
    }
}

pub async fn get_provider_config_handler() -> Result<impl warp::Reply, Rejection> {
    info!("Getting provider configuration");

    let config = Config::global();
    let provider = config
        .get_param::<String>("GOOSE_PROVIDER")
        .unwrap_or_else(|_| "Not configured".to_string());
    let model = config
        .get_param::<String>("GOOSE_MODEL")
        .unwrap_or_else(|_| "Not configured".to_string());

    let response = ProviderConfig { provider, model };
    Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
}


pub async fn shutdown_agent_extensions(agent_ref: Arc<Mutex<Agent>>) {
    let agent_guard = agent_ref.lock().await;
    let extensions = agent_guard.list_extensions().await;
    drop(agent_guard);

    for ext_name in extensions {
        let agent_guard = agent_ref.lock().await;
        if let Err(e) = agent_guard.remove_extension(&ext_name).await {
            error!("Failed to remove extension {} during shutdown: {}", ext_name, e);
        }
    }
}

pub async fn metrics_handler() -> Result<impl warp::Reply, Rejection> {
    info!("Getting metrics");


    // Gather pending request sizes for each extension
    let agent_guard = AGENT.lock().await;
    let pending_requests: HashMap<String, usize> = agent_guard
        .get_tool_stats()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v as usize))
        .collect();

    let resp = MetricsResponse {
        active_sessions: SESSIONS.len(),
        pending_requests,
    };

    Ok(warp::reply::json(&resp))
}

pub async fn handle_rejection(err: Rejection) -> Result<impl warp::Reply, Rejection> {
    if let Some(e) = err.find::<AnyhowRejection>() {
        let message = e.0.to_string();
        let status_code = if message.contains("Unauthorized") {
            warp::http::StatusCode::UNAUTHORIZED
        } else if message.contains("Failed to add extension") || message.contains("Failed to remove extension") {
            warp::http::StatusCode::BAD_REQUEST
        }
        else {
            warp::http::StatusCode::INTERNAL_SERVER_ERROR
        };

        let response = ApiResponse {
            message,
            status: "error".to_string(),
        };
        let json = warp::reply::json(&response);
        Ok(warp::reply::with_status(json, status_code))
    } else {
        // If it's not a custom rejection, re-reject it
        Err(err)
    }
}

pub fn with_api_key(api_key: String) -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
    warp::header::value("x-api-key")
        .and_then(move |header_api_key: HeaderValue| {
            let api_key = api_key.clone();
            async move {
                if header_api_key == api_key {
                    Ok(api_key)
                } else {
                    warn!("Unauthorized access attempt with API key: {}", header_api_key.to_str().unwrap_or("invalid_header_value"));
                    Err(warp::reject::custom(AnyhowRejection(anyhow::anyhow!("Unauthorized"))))
                }
            }
        })
}
