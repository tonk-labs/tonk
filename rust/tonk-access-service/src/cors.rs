//! CORS handling utilities.
//!
//! Provides helpers for handling Cross-Origin Resource Sharing (CORS) requests.
//!
//! NOTE: Currently allows all origins (`*`). We should consider restricting
//! to specific allowed origins for production.

use worker::*;

/// Standard CORS headers to allow cross-origin requests.
///
/// Allows:
/// - All origins (`*`)
/// - Methods: GET, PUT, POST, DELETE, OPTIONS
/// - Headers: Authorization, X-UCAN-Proofs, X-Checksum-SHA256, Content-Type
/// - Preflight cache: 24 hours
pub fn cors_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Access-Control-Allow-Origin", "*"),
        (
            "Access-Control-Allow-Methods",
            "GET, PUT, POST, DELETE, OPTIONS",
        ),
        (
            "Access-Control-Allow-Headers",
            "Authorization, X-UCAN-Proofs, X-Checksum-SHA256, Content-Type",
        ),
        ("Access-Control-Max-Age", "86400"),
    ]
}

/// Handle CORS preflight (OPTIONS) request.
///
/// Returns a 204 No Content response with CORS headers.
pub fn preflight_response() -> Result<Response> {
    let mut response = Response::empty()?.with_status(204);
    let headers = response.headers_mut();
    for (key, value) in cors_headers() {
        headers.set(key, value)?;
    }
    Ok(response)
}

/// Add CORS headers to an existing response.
pub fn with_cors(response: Response) -> Result<Response> {
    let mut response = response;
    let headers = response.headers_mut();
    for (key, value) in cors_headers() {
        headers.set(key, value)?;
    }
    Ok(response)
}
