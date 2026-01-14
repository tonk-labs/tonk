use ::axum::Json;
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use url::Url;
use web_time::Duration;

use crate::TonkWorkerError;

/// Authorization request with account credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    /// Secret key for authentication.
    pub secret_key: String,
    /// Account identifier.
    pub account_id: String,
}

/// Authorization response containing a presigned URL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    /// Presigned URL for accessing authorized resources.
    pub presigned_url: Url,
}

/// Handles authorization requests for accessing cloud storage resources.
#[wasm_compat]
pub async fn authorize(
    Json(_body): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    use crate::sleep;

    log!("NOTE: Simulating S3/R2 API request latency of ~1 second...");

    if let Err(error) = sleep(Duration::from_secs(1)).await {
        log!("{:?}", error);
    }

    Ok(Json(AuthorizeResponse {
        presigned_url: Url::parse("https://www.example.com").unwrap(),
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_artifacts;
    use crate::{AuthorizeRequest, AuthorizeResponse, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_authorizes_and_returns_presigned_url() {
        let artifacts = test_artifacts().await;
        let app = api_router(artifacts);

        let auth_request = AuthorizeRequest {
            secret_key: "test-secret".to_string(),
            account_id: "test-account".to_string(),
        };

        let request = Request::builder()
            .uri("/api/authorize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&auth_request).expect("Failed to serialize request"),
            ))
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let auth_response: AuthorizeResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert_eq!(
            auth_response.presigned_url.as_str(),
            "https://www.example.com/"
        );
    }
}
