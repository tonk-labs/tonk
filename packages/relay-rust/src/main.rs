mod error;
mod network;
mod server;
mod storage;

use error::Result;
use network::WebSocketServer;
use samod::storage::TokioFilesystemStorage;
use samod::RepoBuilder;
use server::RelayServer;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

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
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(8080);

    let bundle_path = args
        .get(2)
        .map(PathBuf::from)
        .ok_or_else(|| error::RelayError::Other("Bundle path is required".to_string()))?;

    let storage_dir = args
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

    let s3_config = std::env::var("S3_BUCKET_NAME").ok().map(|bucket| {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-north-1".to_string());
        (bucket, region)
    });

    if let Some((ref bucket, ref region)) = s3_config {
        tracing::info!("S3 storage enabled: bucket={}, region={}", bucket, region);
    } else {
        tracing::info!("S3 storage disabled (no S3_BUCKET_NAME configured)");
    }

    let filesystem_storage = TokioFilesystemStorage::new(storage_dir.clone());

    let runtime = tokio::runtime::Handle::current();
    let repo = RepoBuilder::new(runtime)
        .with_storage(filesystem_storage)
        .load()
        .await;

    let repo = Arc::new(repo);

    let connection_count = Arc::new(AtomicUsize::new(0));

    let ws_addr: SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .expect("Invalid WebSocket address");

    let http_addr: SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .expect("Invalid HTTP address");

    let wasm_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("core-js")
        .join("dist")
        .join("tonk_core_bg.wasm");

    let relay_server = RelayServer::create(
        Arc::clone(&repo),
        bundle_path.clone(),
        wasm_path,
        bundle_path.clone(),
        s3_config,
        Arc::clone(&connection_count),
    )
    .await?;

    let ws_server =
        WebSocketServer::new(Arc::clone(&repo), ws_addr, Arc::clone(&connection_count)).await?;

    let ws_handle = tokio::spawn(async move {
        if let Err(e) = ws_server.run().await {
            tracing::error!("WebSocket server error: {}", e);
        }
    });

    let http_handle = tokio::spawn(async move {
        if let Err(e) = relay_server.run(http_addr).await {
            tracing::error!("HTTP server error: {}", e);
        }
    });

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Shutting down gracefully...");

    ws_handle.abort();
    http_handle.abort();

    Ok(())
}
