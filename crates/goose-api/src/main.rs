use goose_api::run_server;
use std::env;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = env::args().collect();
    
    // Check if this is being called as an MCP server
    if args.len() >= 3 && args[1] == "mcp" {
        let extension_name = &args[2];
        run_mcp_server(extension_name).await
    } else {
        // Run as the main API server
        run_server().await
    }
}

async fn run_mcp_server(extension_name: &str) -> Result<(), anyhow::Error> {
    use goose_mcp::*;
    use mcp_server::router::RouterService;
    use mcp_server::{ByteTransport, Server};
    use tokio::io::{stdin, stdout};
    use tracing_subscriber;

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Route to the appropriate MCP server based on extension name
    let result = match extension_name {
        "computercontroller" => {
            let router = RouterService(ComputerControllerRouter::new());
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        "developer" => {
            let router = RouterService(DeveloperRouter::new());
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        "memory" => {
            let router = RouterService(MemoryRouter::new());
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        "google_drive" => {
            let router = RouterService(GoogleDriveRouter::new().await);
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        "jetbrains" => {
            let router = RouterService(JetBrainsRouter::new());
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        "tutorial" => {
            let router = RouterService(TutorialRouter::new());
            let server = Server::new(router);
            let transport = ByteTransport::new(stdin(), stdout());
            server.run(transport).await
        },
        _ => {
            eprintln!("Unknown MCP extension: {}", extension_name);
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("MCP server error for {}: {}", extension_name, e);
        std::process::exit(1);
    }

    Ok(())
}
