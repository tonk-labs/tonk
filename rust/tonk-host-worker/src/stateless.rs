//! Reads that may scan, answered in the isolate the request reached
//! instead of in the space writer.
//!
//! A space writer's Durable Object keeps its peer warm, and wasm memory
//! never shrinks, not even across a `ctx.abort()` that keeps the isolate,
//! so a read that scans a large share of the space would hold the object
//! at that high water for as long as it lives. A read the writer cannot
//! bound from its document ([`tonk_worker::is_narrow_read`]), and a read
//! in pages, runs here instead, on a peer opened for the one request.
//!
//! The peer acts as the session's writer key, as the space writer's does.
//! A read commits nothing and its pull only adopts the remote head, so
//! the two never sign a revision side by side.

use axum::body::Body;
use dialog_varsig::Did;
use http::{Method, Request, Response, header};
use tonk_worker::SpaceWriter;
use tonk_worker::mcp::Verified;

/// Whether `request`, one of the writer's routes, is a read.
pub(crate) fn is_read(request: &Request<Body>) -> bool {
    request.method() == Method::POST && request.uri().path().trim_matches('/') == "api/query"
}

/// Whether the read with `uri` and the document `body` runs here rather
/// than in the writer: it pages, or it may scan.
pub(crate) fn runs_here(uri: &http::Uri, body: &[u8]) -> bool {
    let paged = uri
        .query()
        .unwrap_or("")
        .split('&')
        .any(|pair| pair.starts_with("limit=") || pair.starts_with("cursor="));
    paged || !tonk_worker::is_narrow_read(&String::from_utf8_lossy(body))
}

/// Answer the paged read `request` with a peer opened for it.
pub(crate) async fn read(
    verified: &Verified,
    mcp_session: &str,
    space: &Did,
    remote: &str,
    request: Request<Body>,
) -> Result<Response<Body>, String> {
    let remote = url::Url::parse(remote).map_err(|error| error.to_string())?;
    let session = verified.writer_session(space, mcp_session);
    let root = format!(
        "/stateless-read/{:016x}",
        (js_sys::Math::random() * 1e16) as u64
    );
    let writer = SpaceWriter::open(&root, &session, space, &remote)
        .await
        .map_err(|error| error.to_string())?;
    let (parts, body) = request.into_parts();
    let query = parts.uri.query().unwrap_or("");
    let page: String = query
        .split('&')
        .filter(|pair| pair.starts_with("limit=") || pair.starts_with("cursor="))
        .map(|pair| format!("&{pair}"))
        .collect();
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/evaluate?transact=false{page}"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(body)
        .map_err(|error| error.to_string())?;
    let response = writer.read(request).await;
    writer.close().await;
    Ok(response)
}
