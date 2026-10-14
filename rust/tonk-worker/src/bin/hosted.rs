//! Private native worker host. The outer service owns user authentication and
//! scopes requests to a selected space; this loopback API is never public.
#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use axum::{
        extract::Request,
        middleware::{self, Next},
        response::Response,
    };
    use std::path::PathBuf;
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("explicit data directory required")?,
    );
    let secret = std::env::var("TONK_WORKER_HOST_TOKEN")?;
    if secret.len() < 32 {
        return Err("private host token must contain at least 32 characters".into());
    }
    let state = tonk_worker::TonkState::open_native(&directory).await?;
    let (router, state, hub) = tonk_worker::api_router_with_state(state);
    let expected = format!("Bearer {secret}");
    let router = router.layer(middleware::from_fn(
        move |mut request: Request, next: Next| {
            let expected = expected.clone();
            async move {
                if request
                    .headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    != Some(expected.as_str())
                {
                    return Response::builder()
                        .status(401)
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                if let Some(client) = request
                    .headers()
                    .get("x-tonk-client-id")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned)
                {
                    request
                        .extensions_mut()
                        .insert(tonk_worker::ClientId(client));
                }
                next.run(request).await
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    println!(
        "{}",
        serde_json::json!({"address":listener.local_addr()?.to_string()})
    );
    let sync_state = state.clone();
    let sync_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            tonk_worker::drain_sync(&sync_state).await;
        }
    });
    let stop_sync = sync_task.abort_handle();
    let shutdown = async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        stop_sync.abort();
        state.read().await.shutdown();
        hub.shutdown().await;
    };
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
