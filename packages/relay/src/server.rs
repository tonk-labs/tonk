use crate::error::{RelayError, Result};
use crate::network::handle_websocket_connection;
use crate::storage::{BundleStorageAdapter, S3Storage};
use axum::extract::ws::{rejection::WebSocketUpgradeRejection, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use samod::Repo;
use serde_json::json;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};
use zip::ZipArchive;

pub struct AppState {
    pub repo: Arc<Repo>,
    pub bundle_storage: Arc<BundleStorageAdapter>,
    pub s3_storage: Option<Arc<S3Storage>>,
    pub connection_count: Arc<AtomicUsize>,
    pub start_time: SystemTime,
    pub wasm_path: PathBuf,
    pub blank_tonk_path: PathBuf,
}

pub struct RelayServer {
    pub state: Arc<AppState>,
}

impl RelayServer {
    pub async fn create(
        repo: Arc<Repo>,
        bundle_path: PathBuf,
        wasm_path: PathBuf,
        blank_tonk_path: PathBuf,
        s3_config: (String, String),
        connection_count: Arc<AtomicUsize>,
    ) -> Result<Self> {
        let bundle_bytes = std::fs::read(&bundle_path)?;
        let bundle_storage = Arc::new(BundleStorageAdapter::from_bundle(bundle_bytes).await?);
        let s3_storage = Some(Arc::new(S3Storage::new(s3_config.0, s3_config.1).await?));

        let state = Arc::new(AppState {
            repo: Arc::clone(&repo),
            bundle_storage,
            s3_storage,
            connection_count,
            start_time: SystemTime::now(),
            wasm_path,
            blank_tonk_path,
        });

        Ok(Self { state })
    }

    pub fn router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/", get(root_handler))
            .route("/tonk_core_bg.wasm", get(serve_wasm))
            .route("/.manifest.tonk", get(serve_manifest))
            .route("/api/bundles", post(upload_bundle))
            .route("/api/bundles/{id}", get(download_bundle))
            .route("/api/bundles/{id}/manifest", get(download_bundle_manifest))
            .route("/api/blank-tonk", get(serve_blank_tonk))
            .route("/metrics", get(metrics))
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .with_state(state)
    }

    pub async fn run(self, http_addr: SocketAddr) -> Result<()> {
        let app = Self::router(Arc::clone(&self.state));

        let listener = tokio::net::TcpListener::bind(http_addr).await?;

        tracing::info!(
            "Unified server (HTTP + WebSocket) listening on {}",
            http_addr
        );

        axum::serve(listener, app)
            .await
            .map_err(|e| RelayError::Other(format!("HTTP server error: {}", e)))?;

        Ok(())
    }
}

async fn health_check() -> impl IntoResponse {
    "👍 Tonk relay server is running"
}

async fn root_handler(
    headers: HeaderMap,
    ws: std::result::Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    State(state): State<Arc<AppState>>,
) -> Response {
    if headers
        .get(header::UPGRADE)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|v: &str| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        match ws {
            Ok(ws) => ws
                .on_upgrade(move |socket| handle_websocket(socket, state))
                .into_response(),
            Err(_) => {
                (StatusCode::BAD_REQUEST, "Invalid WebSocket upgrade request").into_response()
            }
        }
    } else {
        health_check().await.into_response()
    }
}

async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let start = std::time::Instant::now();
    tracing::info!("WebSocket handler started");

    let result = handle_websocket_connection(
        socket,
        Arc::clone(&state.repo),
        Arc::clone(&state.connection_count),
    )
    .await;

    let duration = start.elapsed();
    tracing::info!(
        "WebSocket handler finished after {:?}, reason {:?}",
        duration,
        result
    );
}

async fn serve_wasm(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse> {
    let wasm_bytes = tokio::fs::read(&state.wasm_path).await?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/wasm"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS"),
        ],
        wasm_bytes,
    ))
}

async fn serve_manifest(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse> {
    tracing::info!("Received request for /.manifest.tonk");

    let slim_bundle: Vec<u8> = state.bundle_storage.create_slim_bundle().await?;

    tracing::info!(
        "Slim bundle created successfully, size: {}",
        slim_bundle.len()
    );

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"manifest.tonk\"",
            ),
        ],
        slim_bundle,
    ))
}

async fn upload_bundle(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let s3_storage = state
        .s3_storage
        .as_ref()
        .ok_or_else(|| RelayError::S3("S3 storage not configured".to_string()))?;

    if body.is_empty() {
        return Err(RelayError::Bundle("Invalid bundle data".to_string()));
    }

    let cursor = std::io::Cursor::new(body.as_ref());
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| RelayError::Bundle(format!("Invalid bundle: {}", e)))?;

    let mut manifest_file = archive
        .by_name("manifest.json")
        .map_err(|_| RelayError::Bundle("Invalid bundle: manifest.json not found".to_string()))?;

    let mut manifest_content = String::new();
    manifest_file.read_to_string(&mut manifest_content)?;

    let manifest: serde_json::Value = serde_json::from_str(&manifest_content)?;

    let bundle_id = manifest
        .get("rootId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            RelayError::Bundle("Invalid bundle: rootId not found in manifest".to_string())
        })?;

    s3_storage.upload_bundle(bundle_id, body.to_vec()).await?;

    Ok(Json(json!({
        "id": bundle_id,
        "message": "Bundle uploaded successfully"
    })))
}

async fn download_bundle(
    State(state): State<Arc<AppState>>,
    Path(bundle_id): Path<String>,
) -> Result<impl IntoResponse> {
    let s3_storage = state
        .s3_storage
        .as_ref()
        .ok_or_else(|| RelayError::S3("S3 storage not configured".to_string()))?;

    let bundle_data = s3_storage.download_bundle(&bundle_id).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}.tonk\"", bundle_id)).unwrap(),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );

    Ok((StatusCode::OK, headers, bundle_data))
}

async fn download_bundle_manifest(
    State(state): State<Arc<AppState>>,
    Path(bundle_id): Path<String>,
) -> Result<impl IntoResponse> {
    let s3_storage = state
        .s3_storage
        .as_ref()
        .ok_or_else(|| RelayError::S3("S3 storage not configured".to_string()))?;

    let bundle_data = s3_storage.download_bundle(&bundle_id).await?;

    let cursor = std::io::Cursor::new(&bundle_data);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| RelayError::Bundle(format!("Failed to open bundle: {}", e)))?;

    let root_id_prefix = bundle_id.chars().take(2).collect::<String>();
    let storage_folder_prefix = format!("storage/{}", root_id_prefix);

    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let mut zip_data = Vec::new();
    let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

    if let Ok(mut manifest_file) = archive.by_name("manifest.json") {
        let mut manifest_content = Vec::new();
        manifest_file.read_to_end(&mut manifest_content)?;
        zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
        zip_writer.write_all(&manifest_content)?;
    }

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_name = file.name().to_string();

        if !file.is_dir()
            && file_name != "manifest.json"
            && file_name.starts_with(&storage_folder_prefix)
        {
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;
            zip_writer.start_file(&file_name, SimpleFileOptions::default())?;
            zip_writer.write_all(&content)?;
        }
    }

    zip_writer.finish()?;

    let mut headers = HeaderMap::new();

    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}.tonk\"", bundle_id)).unwrap(),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );

    Ok((StatusCode::OK, headers, zip_data))
}

async fn serve_blank_tonk(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse> {
    let blank_tonk_bytes = tokio::fs::read(&state.blank_tonk_path).await?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"latergram.tonk\"",
            ),
            (header::CACHE_CONTROL, "public, max-age=31536000"),
        ],
        blank_tonk_bytes,
    ))
}

async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use sysinfo::System;

    let mut sys = System::new_all();
    sys.refresh_memory();

    let uptime = state.start_time.elapsed().unwrap_or_default().as_secs();

    Json(json!({
        "timestamp": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        "memory": {
            "rss": sys.used_memory(),
            "total": sys.total_memory(),
        },
        "connections": state.connection_count.load(Ordering::Relaxed),
        "uptime": uptime,
        "process": {
            "pid": std::process::id(),
        }
    }))
}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            RelayError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            RelayError::S3(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
            RelayError::Bundle(msg) => (StatusCode::BAD_REQUEST, msg),
            RelayError::InvalidManifest(msg) => (StatusCode::BAD_REQUEST, msg),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "error": error_message
        }));

        (status, body).into_response()
    }
}
