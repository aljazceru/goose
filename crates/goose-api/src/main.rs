use warp::{Filter, Rejection};
use warp::http::HeaderValue;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use goose::agents::{Agent, ExtensionConfig, extension_manager::ExtensionManager};
use goose::config::{Config, ExtensionEntry};
use goose::providers::{create, providers};
use goose::model::ModelConfig;
use goose::message::Message;
use tracing::{info, warn, error};
use config::{builder::DefaultState, ConfigBuilder, Environment, File};
use serde_json::Value; // Import the correct Value type
use futures_util::TryStreamExt;

// Global extension manager for extension listing
static EXTENSION_MANAGER: LazyLock<ExtensionManager> = LazyLock::new(|| ExtensionManager::default());

// Global agent for handling sessions
static AGENT: LazyLock<tokio::sync::Mutex<Agent>> = LazyLock::new(|| {
    tokio::sync::Mutex::new(Agent::new())
});

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
struct ExtensionsResponse {
    extensions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderConfig {
    provider: String,
    model: String,
}

async fn start_session_handler(
    req: SessionRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Starting session with prompt: {}", req.prompt);
    
    let agent = AGENT.lock().await;
    
    // Create a user message with the prompt
    let messages = vec![Message::user().with_text(&req.prompt)];
    
    // Process the messages through the agent
    let result = agent.reply(&messages, None).await;
    
    match result {
        Ok(mut stream) => {
            // Process the stream to get the first response
            if let Ok(Some(response)) = stream.try_next().await {
                let response_text = response.as_concat_text();
                let api_response = ApiResponse {
                    message: format!("Session started with prompt: {}. Response: {}", req.prompt, response_text),
                    status: "success".to_string(),
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            } else {
                let api_response = ApiResponse {
                    message: format!("Session started but no response generated"),
                    status: "warning".to_string(),
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
    req: SessionRequest,
    _api_key: String,
) -> Result<impl warp::Reply, Rejection> {
    info!("Replying to session with prompt: {}", req.prompt);
    
    let agent = AGENT.lock().await;
    
    // Create a user message with the prompt
    let messages = vec![Message::user().with_text(&req.prompt)];
    
    // Process the messages through the agent
    let result = agent.reply(&messages, None).await;
    
    match result {
        Ok(mut stream) => {
            // Process the stream to get the first response
            if let Ok(Some(response)) = stream.try_next().await {
                let response_text = response.as_concat_text();
                let api_response = ApiResponse {
                    message: format!("Reply: {}", response_text),
                    status: "success".to_string(),
                };
                Ok(warp::reply::with_status(
                    warp::reply::json(&api_response),
                    warp::http::StatusCode::OK,
                ))
            } else {
                let api_response = ApiResponse {
                    message: format!("Reply processed but no response generated"),
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

async fn list_extensions_handler() -> Result<impl warp::Reply, Rejection> {
    info!("Listing extensions");
    
    match EXTENSION_MANAGER.list_extensions().await {
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

async fn get_provider_config_handler() -> Result<impl warp::Reply, Rejection> {
    info!("Getting provider configuration");
    
    let config = Config::global();
    let provider = config.get_param::<String>("GOOSE_PROVIDER")
        .unwrap_or_else(|_| "Not configured".to_string());
    let model = config.get_param::<String>("GOOSE_MODEL")
        .unwrap_or_else(|_| "Not configured".to_string());
    
    let response = ProviderConfig { provider, model };
    Ok::<warp::reply::Json, warp::Rejection>(warp::reply::json(&response))
}

fn with_api_key(api_key: String) -> impl Filter<Extract = (String,), Error = Rejection> + Clone {
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

// Load configuration from file and environment variables
fn load_configuration() -> std::result::Result<config::Config, config::ConfigError> {
    let config_path = std::env::var("GOOSE_CONFIG").unwrap_or_else(|_| "config".to_string());
    let builder = ConfigBuilder::<DefaultState>::default()
        .add_source(File::with_name(&config_path).required(false))
        .add_source(Environment::with_prefix("GOOSE_API"));
    
    builder.build()
}

// Initialize global provider configuration
async fn initialize_provider_config() -> Result<(), anyhow::Error> {
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
    
    let agent = AGENT.lock().await;
    agent.update_provider(provider).await?;
    
    info!("Provider configuration successful");
    Ok(())
}
/// Initialize extensions from the configuration.
async fn initialize_extensions(config: &config::Config) -> Result<(), anyhow::Error> {
    if let Ok(ext_table) = config.get_table("extensions") {
        for (name, ext_config) in ext_table {
            // Deserialize into ExtensionEntry to get enabled flag and config
            let entry: ExtensionEntry = ext_config.clone().try_deserialize()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize extension config for {}: {}", name, e))?;

            if entry.enabled {
                let extension_config: ExtensionConfig = entry.config;
                // Acquire the global agent lock and try to add the extension
                let mut agent = AGENT.lock().await;
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


async fn run_init_tests() -> Result<(), anyhow::Error> {
    info!("Running initialization tests");
    {
        let _agent = AGENT.lock().await;
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
    
    // Initialize provider configuration
    if let Err(e) = initialize_provider_config().await {
        error!("Failed to initialize provider: {}", e);
        return Err(e);
    }
    
    // Initialize extensions from configuration
    if let Err(e) = initialize_extensions(&api_config).await {
        error!("Failed to initialize extensions: {}", e);
    }
    
    if let Err(e) = run_init_tests().await {
        error!("Initialization tests failed: {}", e);
    }

    // Session start endpoint
    let start_session = warp::path("session")
        .and(warp::path("start"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(start_session_handler);
    
    // Session reply endpoint 
    let reply_session = warp::path("session")
        .and(warp::path("reply"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(reply_session_handler);
    
    // List extensions endpoint
    let list_extensions = warp::path("extensions")
        .and(warp::path("list"))
        .and(warp::get())
        .and_then(list_extensions_handler);
    
    // Get provider configuration endpoint
    let get_provider_config = warp::path("provider")
        .and(warp::path("config"))
        .and(warp::get())
        .and_then(get_provider_config_handler);
    
    // Combine all routes
    let routes = start_session
        .or(reply_session)
        .or(list_extensions)
        .or(get_provider_config);
    
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
