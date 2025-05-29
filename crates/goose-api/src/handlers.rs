use warp::{http::HeaderValue, Filter, Rejection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use futures_util::TryStreamExt;
use tracing::{info, warn, error};
use mcp_core::tool::Tool;
use goose::agents::{extension::Envs, extension_manager::ExtensionManager, ExtensionConfig, Agent, SessionConfig};
use goose::message::Message;
use goose::session::{self, Identifier};
use goose::config::Config;
use std::sync::LazyLock;

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

    let agent = AGENT.lock().await;
    let mut messages = vec![Message::user().with_text(&req.prompt)];
    let session_id = Uuid::new_v4();
    let session_name = session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    let provider = agent.provider().await.ok();

    let result = agent
        .reply(
            &messages,
            Some(SessionConfig {
                id: Identifier::Name(session_name.clone()),
                working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            }),
        )
        .await;

    match result {
        Ok(mut stream) => {
            if let Ok(Some(response)) = stream.try_next().await {
                let response_text = response.as_concat_text();
                messages.push(response);
                if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                    warn!("Failed to persist session {}: {}", session_name, e);
                }

                let api_response = StartSessionResponse {
                    message: response_text,
                    status: "success".to_string(),
                    session_id,
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            } else {
                if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                    warn!("Failed to persist session {}: {}", session_name, e);
                }

                let api_response = StartSessionResponse {
                    message: "Session started but no response generated".to_string(),
                    status: "warning".to_string(),
                    session_id,
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            }
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

    let agent = AGENT.lock().await;

    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

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

    let provider = agent.provider().await.ok();

    let result = agent
        .reply(
            &messages,
            Some(SessionConfig {
                id: Identifier::Name(session_name.clone()),
                working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            }),
        )
        .await;

    match result {
        Ok(mut stream) => {
            if let Ok(Some(response)) = stream.try_next().await {
                let response_text = response.as_concat_text();
                messages.push(response);
                if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                    warn!("Failed to persist session {}: {}", session_name, e);
                }
                let api_response = ApiResponse {
                    message: format!("Reply: {}", response_text),
                    status: "success".to_string(),
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            } else {
                if let Err(e) = session::persist_messages(&session_path, &messages, provider.clone()).await {
                    warn!("Failed to persist session {}: {}", session_name, e);
                }
                let api_response = ApiResponse {
                    message: "Reply processed but no response generated".to_string(),
                    status: "warning".to_string(),
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            }
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
    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

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

pub async fn add_extension_handler(
    req: ExtensionConfigRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Adding extension: {:?}", req);

    #[cfg(target_os = "windows")]
    if let ExtensionConfigRequest::Stdio { cmd, .. } = &req {
        if cmd.ends_with("npx.cmd") || cmd.ends_with("npx") {
            let node_exists = std::path::Path::new(r"C:\Program Files\nodejs\node.exe").exists()
                || std::path::Path::new(r"C:\Program Files (x86)\nodejs\node.exe").exists();

            if !node_exists {
                let cmd_path = std::path::Path::new(cmd);
                let script_dir = cmd_path.parent().ok_or_else(|| warp::reject())?;

                let install_script = script_dir.join("install-node.cmd");

                if install_script.exists() {
                    eprintln!("Installing Node.js...");
                    let output = std::process::Command::new(&install_script)
                        .arg("https://nodejs.org/dist/v23.10.0/node-v23.10.0-x64.msi")
                        .output()
                        .map_err(|_| warp::reject())?;

                    if !output.status.success() {
                        eprintln!(
                            "Failed to install Node.js: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        let resp = ExtensionResponse {
                            error: true,
                            message: Some(format!(
                                "Failed to install Node.js: {}",
                                String::from_utf8_lossy(&output.stderr)
                            )),
                        };
                        return Ok(warp::reply::json(&resp));
                    }
                    eprintln!("Node.js installation completed");
                } else {
                    eprintln!("Node.js installer script not found at: {}", install_script.display());
                    let resp = ExtensionResponse {
                        error: true,
                        message: Some("Node.js installer script not found".to_string()),
                    };
                    return Ok(warp::reply::json(&resp));
                }
            }
        }
    }

    let extension = match req {
        ExtensionConfigRequest::Sse { name, uri, envs, env_keys, timeout } => {
            ExtensionConfig::Sse {
                name,
                uri,
                envs,
                env_keys,
                description: None,
                timeout,
                bundled: None,
            }
        }
        ExtensionConfigRequest::Stdio { name, cmd, args, envs, env_keys, timeout } => {
            ExtensionConfig::Stdio {
                name,
                cmd,
                args,
                envs,
                env_keys,
                timeout,
                description: None,
                bundled: None,
            }
        }
        ExtensionConfigRequest::Builtin { name, display_name, timeout } => {
            ExtensionConfig::Builtin {
                name,
                display_name,
                timeout,
                bundled: None,
            }
        }
        ExtensionConfigRequest::Frontend { name, tools, instructions } => {
            ExtensionConfig::Frontend {
                name,
                tools,
                instructions,
                bundled: None,
            }
        }
    };

    let agent = AGENT.lock().await;
    let result = agent.add_extension(extension).await;

    let resp = match result {
        Ok(_) => ExtensionResponse { error: false, message: None },
        Err(e) => ExtensionResponse {
            error: true,
            message: Some(format!("Failed to add extension configuration, error: {:?}", e)),
        },
    };
    Ok(warp::reply::json(&resp))
}

pub async fn remove_extension_handler(
    name: String,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Removing extension: {}", name);
    let agent = AGENT.lock().await;
    agent.remove_extension(&name).await;

    let resp = ExtensionResponse { error: false, message: None };
    Ok(warp::reply::json(&resp))
}

pub fn with_api_key(api_key: String) -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
    warp::header::value("x-api-key")
        .and_then(move |header_api_key: HeaderValue| {
            let api_key = api_key.clone();
            async move {
                if header_api_key == api_key {
                    Ok(api_key)
                } else {
                    Err(warp::reject::not_found())
                }
            }
        })
}
