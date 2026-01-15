//! API router configuration and handlers.

use ::axum::{Router, extract::State, routing::get, routing::post};
use dialog_artifacts::Artifacts;

use crate::ServiceWorkerStorageBackend;

mod authorize;
pub use authorize::*;

/// Root handler that returns a welcome message.
async fn root(State(_artifacts): State<Artifacts<ServiceWorkerStorageBackend>>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the artifacts storage as shared state.
pub fn api_router(artifacts: Artifacts<ServiceWorkerStorageBackend>) -> Router {
    Router::new()
        .route("/api", get(root))
        .route("/api/authorize", post(authorize))
        .with_state(artifacts)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use crate::{ServiceWorkerStorageBackend, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dialog_artifacts::Artifacts;
    use tower::ServiceExt;

    pub async fn test_artifacts() -> Artifacts<ServiceWorkerStorageBackend> {
        let backend = ServiceWorkerStorageBackend::new().await;
        Artifacts::open("tonk-test".into(), backend)
            .await
            .expect("Failed to create test artifacts")
    }

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let artifacts = test_artifacts().await;
        let app = api_router(artifacts);

        let request = Request::builder()
            .uri("/api")
            .method("GET")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");
        assert_eq!(body.as_ref(), b"Hello, Tonk!");
    }
}
