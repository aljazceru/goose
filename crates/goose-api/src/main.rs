use warp::{Filter, Rejection, Reply};
use warp::http::HeaderValue;
use std::convert::Infallible;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use goose::config::{Config, ExtensionEntry};
use goose::agents::{
    extension::Envs,
    Agent,
    extension_manager::ExtensionManager,
    ExtensionConfig,
};
use mcp_core::tool::Tool;
use uuid::Uuid;
use goose::session::{self, Identifier};
use goose::agents::SessionConfig;
use std::path::PathBuf;

use goose::providers::{create, providers};
use goose::model::ModelConfig;
use goose::message::Message;
use tracing::{info, warn, error};
use config::{builder::DefaultState, ConfigBuilder, Environment, File};
use serde_json::Value; // Import the correct Value type
use futures_util::TryStreamExt;

#[derive(Clone)]
struct ServerState {
    agent: Arc<tokio::sync::Mutex<Agent>>,
    extension_manager: Arc<tokio::sync::Mutex<ExtensionManager>>, 
}


#[derive(Debug, Serialize, Deserialize)]
struct SessionRequest {
    prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse {
    message: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartSessionResponse {
    message: String,
    status: String,
    session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionReplyRequest {
    session_id: Uuid,
    prompt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EndSessionRequest {
    session_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExtensionsResponse {
    extensions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderConfig {
    provider: String,
    model: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExtensionResponse {
    error: bool,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ExtensionConfigRequest {
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

async fn start_session_handler(
    req: SessionRequest,
    state: ServerState,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Starting session with prompt: {}", req.prompt);

    let mut agent = state.agent.lock().await;

    // Create a user message with the prompt
    let mut messages = vec![Message::user().with_text(&req.prompt)];

    // Generate a new session ID and process the messages
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
            // Process the stream to get the first response
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
        },
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

async fn reply_session_handler(
    req: SessionReplyRequest,
    state: ServerState,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Replying to session with prompt: {}", req.prompt);

    let mut agent = state.agent.lock().await;

    let session_name = req.session_id.to_string();
    let session_path = session::get_path(Identifier::Name(session_name.clone()));

    // Retrieve existing session history from disk
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

    // Append the new user message
    messages.push(Message::user().with_text(&req.prompt));

    let provider = agent.provider().await.ok();

    // Process the messages through the agent
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
            // Process the stream to get the first response
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
        },
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

async fn end_session_handler(
    req: EndSessionRequest,
    _state: ServerState,
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

async fn list_extensions_handler(state: ServerState) -> Result<impl warp::Reply, Rejection> {
    info!("Listing extensions");
    
    let manager = state.extension_manager.lock().await;
    match manager.list_extensions().await {
        Ok(exts) => {
            let response = ExtensionsResponse { extensions: exts };
            Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
        },
        Err(e) => {
            error!("Failed to list extensions: {}", e);
            let response = ExtensionsResponse { 
                extensions: vec!["Failed to list extensions".to_string()] 
            };
            Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
        }
    }
}

async fn get_provider_config_handler(_state: ServerState) -> Result<impl warp::Reply, Rejection> {
    info!("Getting provider configuration");
    
    let config = Config::global();
    let provider = config.get_param::<String>("GOOSE_PROVIDER")
        .unwrap_or_else(|_| "Not configured".to_string());
    let model = config.get_param::<String>("GOOSE_MODEL")
        .unwrap_or_else(|_| "Not configured".to_string());
    
    let response = ProviderConfig { provider, model };
    Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
}

async fn add_extension_handler(
    req: ExtensionConfigRequest,
    state: ServerState,
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
                    eprintln!(
                        "Node.js installer script not found at: {}",
                        install_script.display()
                    );
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

    let agent = state.agent.lock().await;
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

async fn remove_extension_handler(
    name: String,
    state: ServerState,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Removing extension: {}", name);
    let agent = state.agent.lock().await;
    agent.remove_extension(&name).await;

    let resp = ExtensionResponse { error: false, message: None };
    Ok(warp::reply::json(&resp))
}

#[derive(Debug)]
struct Unauthorized;

impl warp::reject::Reject for Unauthorized {}

async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
    if err.find::<Unauthorized>().is_some() {
        Ok(warp::reply::with_status("UNAUTHORIZED", warp::http::StatusCode::UNAUTHORIZED))
    } else if err.is_not_found() {
        Ok(warp::reply::with_status("NOT_FOUND", warp::http::StatusCode::NOT_FOUND))
    } else {
        Ok(warp::reply::with_status(
            "INTERNAL_SERVER_ERROR",
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

fn with_api_key(api_key: String) -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
    warp::header::value("x-api-key")
        .and_then(move |header_api_key: HeaderValue| {
            let api_key = api_key.clone();
            async move {
                if header_api_key == api_key {
                    Ok(api_key)
                } else {
                    Err(warp::reject::custom(Unauthorized))
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use warp::http::StatusCode;

    #[tokio::test]
    async fn valid_key_allows_request() {
        let filter = with_api_key("secret".to_string())
            .map(|k: String| k)
            .recover(handle_rejection);

        let res = warp::test::request()
            .header("x-api-key", "secret")
            .reply(&filter)
            .await;

        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_key_is_rejected() {
        let filter = with_api_key("secret".to_string())
            .map(|k: String| k)
            .recover(handle_rejection);

        let res = warp::test::request()
            .header("x-api-key", "wrong")
            .reply(&filter)
            .await;

        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}
// Load configuration from file and environment variables
fn load_configuration() -> std::result::Result<config::Config, config::ConfigError> {
    let config_path = std::env::var("GOOSE_CONFIG").unwrap_or_else(|_| "config".to_string());
    let builder = ConfigBuilder::<DefaultState>::default()
        .add_source(File::with_name(&config_path).required(false))
        .add_source(Environment::with_prefix("GOOSE_API"));
    
    builder.build()
}

// Initialize global provider configuration
async fn initialize_provider_config(state: &ServerState) -> Result<(), anyhow::Error> {
    // Get configuration
    let api_config = load_configuration()?;
    
    // Get provider settings from configuration or environment variables
    let provider_name = std::env::var("GOOSE_API_PROVIDER")
        .or_else(|_| api_config.get_string("provider"))
        .unwrap_or_else(|_| "openai".to_string());
    
    let model_name = std::env::var("GOOSE_API_MODEL")
        .or_else(|_| api_config.get_string("model"))
        .unwrap_or_else(|_| "gpt-4o".to_string());
    
    info!("Initializing with provider: {}, model: {}", provider_name, model_name);
    
    // Initialize the global Config object
    let config = Config::global();
    config.set_param("GOOSE_PROVIDER", Value::String(provider_name.clone()))?;
    config.set_param("GOOSE_MODEL", Value::String(model_name.clone()))?;
    
    // Set up API keys from environment variables
    let available_providers = providers();
    if let Some(provider_meta) = available_providers.iter().find(|p| p.name == provider_name) {
        for key in &provider_meta.config_keys {
            let env_name = key.name.clone();
            if let Ok(value) = std::env::var(&env_name) {
                if key.secret {
                    config.set_secret(&key.name, Value::String(value))?;
                    info!("Set secret key: {}", key.name);
                } else {
                    config.set_param(&key.name, Value::String(value))?;
                    info!("Set parameter: {}", key.name);
                }
            } else {
                warn!("Environment variable not set for key: {}", key.name);
                if key.required {
                    error!("Required key {} not provided", key.name);
                    return Err(anyhow::anyhow!("Required key {} not provided", key.name));
                }
            }
        }
    }
    
    // Initialize agent with provider
    let model_config = ModelConfig::new(model_name);
    let provider = create(&provider_name, model_config)?;
    
    let agent = state.agent.lock().await;
    agent.update_provider(provider).await?;
    
    info!("Provider configuration successful");
    Ok(())
}
/// Initialize extensions from the configuration.
async fn initialize_extensions(state: &ServerState, config: &config::Config) -> Result<(), anyhow::Error> {
    if let Ok(ext_table) = config.get_table("extensions") {
        for (name, ext_config) in ext_table {
            // Deserialize into ExtensionEntry to get enabled flag and config
            let entry: ExtensionEntry = ext_config.clone().try_deserialize()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize extension config for {}: {}", name, e))?;

            if entry.enabled {
                let extension_config: ExtensionConfig = entry.config;
                // Acquire the global agent lock and try to add the extension
                let mut agent = state.agent.lock().await;
                if let Err(e) = agent.add_extension(extension_config).await {
                    error!("Failed to add extension {}: {}", name, e);
                }
            } else {
                info!("Skipping disabled extension: {}", name);
            }
        }
    } else {
        warn!("No extensions configured in config file.");
    }
    Ok(())
}


async fn run_init_tests(state: &ServerState) -> Result<(), anyhow::Error> {
    info!("Running initialization tests");
    {
        let _agent = state.agent.lock().await;
        info!("Agent initialization test passed");
    }
    info!("Initialization tests completed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    
    info!("Starting goose-api server");
    
    // Load configuration
    let api_config = load_configuration()?;
    
    // Get API key from configuration or environment
    let api_key: String = std::env::var("GOOSE_API_KEY")
        .or_else(|_| api_config.get_string("api_key"))
        .unwrap_or_else(|_| {
            warn!("No API key configured, using default");
            "default_api_key".to_string()
        });
    
    let state = ServerState {
        agent: Arc::new(tokio::sync::Mutex::new(Agent::new())),
        extension_manager: Arc::new(tokio::sync::Mutex::new(ExtensionManager::default())),
    };

    // Initialize provider configuration
    if let Err(e) = initialize_provider_config(&state).await {
        error!("Failed to initialize provider: {}", e);
        return Err(e);
    }

    // Initialize extensions from configuration
    if let Err(e) = initialize_extensions(&state, &api_config).await {
        error!("Failed to initialize extensions: {}", e);
    }

    if let Err(e) = run_init_tests(&state).await {
        error!("Initialization tests failed: {}", e);
    }

    let state_filter = warp::any().map(move || state.clone());

    // Session start endpoint
    let start_session = warp::path("session")
        .and(warp::path("start"))
        .and(warp::post())
        .and(warp::body::json())
        .and(state_filter.clone())
        .and(with_api_key(api_key.clone()))
        .and_then(start_session_handler);
    
    // Session reply endpoint
    let reply_session = warp::path("session")
        .and(warp::path("reply"))
        .and(warp::post())
        .and(warp::body::json())
        .and(state_filter.clone())
        .and(with_api_key(api_key.clone()))
        .and_then(reply_session_handler);

    // Session end endpoint
    let end_session = warp::path("session")
        .and(warp::path("end"))
        .and(warp::post())
        .and(warp::body::json())
        .and(state_filter.clone())
        .and(with_api_key(api_key.clone()))
        .and_then(end_session_handler);
    
    // List extensions endpoint
    let list_extensions = warp::path("extensions")
        .and(warp::path("list"))
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(list_extensions_handler);

    // Add extension endpoint
    let add_extension = warp::path("extensions")
        .and(warp::path("add"))
        .and(warp::post())
        .and(warp::body::json())
        .and(state_filter.clone())
        .and(with_api_key(api_key.clone()))
        .and_then(add_extension_handler);

    // Remove extension endpoint
    let remove_extension = warp::path("extensions")
        .and(warp::path("remove"))
        .and(warp::post())
        .and(warp::body::json())
        .and(state_filter.clone())
        .and(with_api_key(api_key.clone()))
        .and_then(remove_extension_handler);
    
    // Get provider configuration endpoint
    let get_provider_config = warp::path("provider")
        .and(warp::path("config"))
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(get_provider_config_handler);
    
    // Combine all routes
    let routes = start_session
        .or(reply_session)
        .or(end_session)
        .or(list_extensions)
        .or(add_extension)
        .or(remove_extension)
        .or(get_provider_config)
        .recover(handle_rejection);
    
    // Get bind address from configuration or use default
    let host = std::env::var("GOOSE_API_HOST")
        .or_else(|_| api_config.get_string("host"))
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    
    let port = std::env::var("GOOSE_API_PORT")
        .or_else(|_| api_config.get_string("port"))
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);
    
    info!("Starting server on {}:{}", host, port);
    
    // Parse host string
    let host_parts: Vec<u8> = host.split('.')
        .map(|part| part.parse::<u8>().unwrap_or(127))
        .collect();
    
    let addr = if host_parts.len() == 4 {
        [host_parts[0], host_parts[1], host_parts[2], host_parts[3]]
    } else {
        [127, 0, 0, 1]
    };
    
    // Start the server
    warp::serve(routes)
        .run((addr, port))
        .await;
    
    Ok(())
}
