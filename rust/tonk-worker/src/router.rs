//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post};
use tokio::sync::RwLock;
use tonk_space::Space;

use crate::ServiceWorkerStorageBackend;

mod authorize;
pub use authorize::{AuthorizeRequest, AuthorizeResponse, StatusResponse, authorize, status};

/// Shared application state containing the Space.
pub type AppState = Arc<RwLock<Space<ServiceWorkerStorageBackend>>>;

/// Root handler that returns a welcome message.
async fn root(State(_space): State<AppState>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the space as shared state.
pub fn api_router(space: Space<ServiceWorkerStorageBackend>) -> Router {
    let state: AppState = Arc::new(RwLock::new(space));
    Router::new()
        .route("/api", get(root))
        .route("/api/authorize", post(authorize))
        .route("/api/status", get(status))
        .with_state(state)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use crate::{ServiceWorkerStorageBackend, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tonk_space::{Operator, Space};
    use tower::ServiceExt;

    pub async fn test_space() -> Space<ServiceWorkerStorageBackend> {
        // Generate a unique operator for each test to avoid IndexedDB conflicts
        let operator = Operator::generate();
        let space_did = operator.did().to_string();
        let backend = ServiceWorkerStorageBackend::new(&space_did).await;
        Space::open(space_did, &operator, backend)
            .await
            .expect("Failed to create test space")
    }

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let space = test_space().await;
        let app = api_router(space);

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
