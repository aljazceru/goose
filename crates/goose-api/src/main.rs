use goose_api::run_server;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    run_server().await
}
