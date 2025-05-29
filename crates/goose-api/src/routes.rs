use warp::Filter;
use tracing::{info, warn, error};

use crate::handlers::{
    add_extension_handler, end_session_handler, get_provider_config_handler,
    list_extensions_handler, remove_extension_handler, reply_session_handler,
    start_session_handler, summarize_session_handler, with_api_key,

};
use crate::config::{
    initialize_extensions, initialize_provider_config, load_configuration,
    run_init_tests,
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

    let add_extension = warp::path("extensions")
        .and(warp::path("add"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(add_extension_handler);

    let remove_extension = warp::path("extensions")
        .and(warp::path("remove"))
        .and(warp::post())
        .and(warp::body::json())
        .and(with_api_key(api_key.clone()))
        .and_then(remove_extension_handler);

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
        .or(add_extension)
        .or(remove_extension)
        .or(get_provider_config)
        .or(metrics)
}

pub async fn run_server() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting goose-api server");

    let api_config = load_configuration()?;

    let api_key: String = std::env::var("GOOSE_API_KEY")
        .or_else(|_| api_config.get_string("api_key"))
        .unwrap_or_else(|_| {
            warn!("No API key configured, using default");
            "default_api_key".to_string()
        });

    if let Err(e) = initialize_provider_config().await {
        error!("Failed to initialize provider: {}", e);
        return Err(e);
    }

    if let Err(e) = initialize_extensions(&api_config).await {
        error!("Failed to initialize extensions: {}", e);
    }

    if let Err(e) = run_init_tests().await {
        error!("Initialization tests failed: {}", e);
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

    info!("Starting server on {}:{}", host, port);

    let host_parts: Vec<u8> = host
        .split('.')
        .map(|part| part.parse::<u8>().unwrap_or(127))
        .collect();
    let addr = if host_parts.len() == 4 {
        [host_parts[0], host_parts[1], host_parts[2], host_parts[3]]
    } else {
        [127, 0, 0, 1]
    };

    warp::serve(routes).run((addr, port)).await;
    Ok(())
}
