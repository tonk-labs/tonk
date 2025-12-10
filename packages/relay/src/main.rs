mod error;
mod network;
mod server;
mod storage;

use error::Result;
use samod::RepoBuilder;
use samod::storage::TokioFilesystemStorage;
use server::RelayServer;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    let port = args
        .get(1)
        .and_then(|s: &String| s.parse::<u16>().ok())
        .unwrap_or(8081);

    let bundle_path: PathBuf = args
        .get(2)
        .map(PathBuf::from)
        .ok_or_else(|| error::RelayError::Other("Bundle path is required".to_string()))?;

    let storage_dir: PathBuf = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("automerge-repo-data"));

    if !bundle_path.exists() {
        return Err(error::RelayError::NotFound(format!(
            "Bundle file not found: {}",
            bundle_path.display()
        )));
    }

    tracing::info!("Starting Tonk Relay Server");
    tracing::info!("Port: {}", port);
    tracing::info!("Bundle: {}", bundle_path.display());
    tracing::info!("Storage: {}", storage_dir.display());

    let s3_config = (
        std::env::var("S3_BUCKET_NAME").unwrap_or_else(|_| "host-web-bundle-storage".to_string()),
        (std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".to_string())),
    );

    let filesystem_storage = TokioFilesystemStorage::new(storage_dir.clone());

    let runtime = tokio::runtime::Handle::current();
    let repo = RepoBuilder::new(runtime)
        .with_storage(filesystem_storage)
        .load()
        .await;

    let repo = Arc::new(repo);

    let connection_count = Arc::new(AtomicUsize::new(0));

    let server_addr: SocketAddr = format!(
        "{}:{}",
        std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port
    )
    .parse()
    .expect("Invalid server address");

    let relay_server: RelayServer = RelayServer::create(
        Arc::clone(&repo),
        bundle_path.clone(),
        bundle_path.clone(),
        s3_config,
        Arc::clone(&connection_count),
    )
    .await?;

    let server_handle = tokio::spawn(async move {
        if let Err(e) = relay_server.run(server_addr).await {
            tracing::error!("Server error: {}", e);
        }
    });

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Shutting down gracefully...");

    server_handle.abort();

    Ok(())
}
