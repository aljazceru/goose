use warp::Filter;
use tracing::{info, warn};

use crate::handlers::{
    end_session_handler, get_provider_config_handler, handle_rejection,
    list_extensions_handler, metrics_handler, reply_session_handler,
    start_session_handler, summarize_session_handler, with_api_key,
};
use crate::config::{
    load_provider_config, load_configuration,
};

pub fn build_routes(api_key: String) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let start_session = warp::path("session")
        .and(warp::path("start"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(start_session_handler);

    let reply_session = warp::path("session")
        .and(warp::path("reply"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(reply_session_handler);

    let summarize_session = warp::path("session")
        .and(warp::path("summarize"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(summarize_session_handler);

    let end_session = warp::path("session")
        .and(warp::path("end"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(end_session_handler);

    let list_extensions = warp::path("extensions")
        .and(warp::path("list"))
        .and(warp::get())
        .and_then(list_extensions_handler);


    let get_provider_config = warp::path("provider")
        .and(warp::path("config"))
        .and(warp::get())
        .and_then(get_provider_config_handler);

    let metrics = warp::path("metrics")
        .and(warp::get())
        .and_then(metrics_handler);

    start_session
        .or(reply_session)
        .or(summarize_session)
        .or(end_session)
        .or(list_extensions)
        .or(get_provider_config)
        .or(metrics)
        .recover(handle_rejection)
}

pub async fn run_server() -> Result<(), anyhow::Error> {
    
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting goose-api server");

    let api_config = load_configuration()?;

    let api_key_source = if std::env::var("GOOSE_API_KEY").is_ok() {
        "environment variable"
    } else if api_config.get_string("api_key").is_ok() {
        "config file"
    } else {
        "default"
    };
    info!("API key loaded from: {}", api_key_source);

    let api_key: String = std::env::var("GOOSE_API_KEY")
        .or_else(|_| api_config.get_string("api_key"))
        .unwrap_or_else(|_| {
            warn!("No API key configured, using default");
            "default_api_key".to_string()
        });
    info!("Using API key: {}", api_key);

    // Load provider config but don't initialize a global agent
    let provider_config = load_provider_config().await?;
    info!("Provider config loaded: {} with model {}", provider_config.provider_name, provider_config.model_name);
    
    // Store provider config globally
    {
        use crate::config::PROVIDER_CONFIG;
        let mut config_guard = PROVIDER_CONFIG.write().await;
        *config_guard = Some(provider_config.clone());
    }
    

    let routes = build_routes(api_key.clone());

    let host = std::env::var("GOOSE_API_HOST")
        .or_else(|_| api_config.get_string("host"))
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("GOOSE_API_PORT")
        .or_else(|_| api_config.get_string("port"))
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);

    info!("Server binding to {}:{}", host, port);

    let host_parts: Vec<u8> = host
        .split('.')
        .map(|part| part.parse::<u8>().unwrap_or(127))
        .collect();
    let addr = if host_parts.len() == 4 {
        [host_parts[0], host_parts[1], host_parts[2], host_parts[3]]
    } else {
        [127, 0, 0, 1]
    };

    let (_addr, server) = warp::serve(routes).bind_with_graceful_shutdown((addr, port), async {
        tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        info!("Received Ctrl+C, initiating graceful shutdown...");

        // Perform cleanup here - shutdown all active sessions
        use crate::api_sessions::SESSIONS;
        use crate::handlers::shutdown_agent_extensions;
        
        info!("Shutting down {} active sessions", SESSIONS.len());
        
        // Collect all sessions to avoid holding locks
        let sessions_to_shutdown: Vec<_> = SESSIONS.iter()
            .map(|entry| (entry.key().clone(), entry.value().agent.clone()))
            .collect();
        
        // Shutdown each session's extensions
        for (session_id, agent) in sessions_to_shutdown {
            info!("Shutting down session {}", session_id);
            shutdown_agent_extensions(agent).await;
            SESSIONS.remove(&session_id);
        }
        
        info!("All sessions shut down during graceful shutdown.");
    });

    server.await; // Await the server
    Ok(())
}
