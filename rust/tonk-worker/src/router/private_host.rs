//! A worker router served to one trusted host process over loopback.

use axum::{
    Router,
    body::Body,
    extract::Request,
    middleware::{self, Next},
    response::Response,
};

/// Accept only requests that carry `token` as a bearer credential, and take
/// the page's client id from `x-tonk-client-id`, as the service worker would
/// supply it. The host authenticates people and scopes their requests; this
/// layer only proves a request came from that host.
pub fn private_host(router: Router, token: &str) -> Router {
    let expected = format!("Bearer {token}");
    router.layer(middleware::from_fn(
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
                        .body(Body::empty())
                        .expect("an empty response");
                }
                if let Some(client) = request
                    .headers()
                    .get("x-tonk-client-id")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned)
                {
                    request.extensions_mut().insert(crate::ClientId(client));
                }
                next.run(request).await
            }
        },
    ))
}
