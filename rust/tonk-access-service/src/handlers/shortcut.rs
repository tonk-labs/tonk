//! Shortcut endpoints.
//!
//! - `PUT /@[?ttl=<seconds>]` — validate and store a path + query
//!   target under its blake3 hash with an expiry stamp; respond with
//!   the base58 hash.
//! - `GET /@/{hash}` — permanent redirect (301) whose `Location` is the stored
//!   relative target. Allowlisted public campaign/source query fields on the
//!   short URL are merged into it; authority and space attribution cannot be
//!   overridden. The browser resolves it against this origin and carries the
//!   original URL's `#fragment` over per RFC 7231 fragment inheritance. 404
//!   for missing or logically expired shortcuts.
//!
//! Both are permissionless: the stored half of a link is non-secret by
//! construction (an invite's seed rides the fragment, which never
//! reaches the server), and a relative redirect cannot leave the
//! origin. See [`crate::shortcut`] for the validation rules.

use crate::shortcut::{
    EXPIRES_METADATA_KEY, Shortcut, object_key_for, referral_redirect_target, requested_ttl,
    unavailable_invite_html,
};
use std::collections::HashMap;
use worker::*;

/// Add CORS headers to a response for WASM compatibility.
fn with_cors_headers(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "GET, PUT, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Access-Control-Expose-Headers", "Content-Type");
    response.with_headers(headers)
}

/// OPTIONS → Handle CORS preflight
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let response = with_cors_headers(Response::empty()?.with_status(204));
    // Only the preflight can be cached, so the lifetime is set here
    // rather than on every response.
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Max-Age", crate::PREFLIGHT_MAX_AGE);
    Ok(response.with_headers(headers))
}

/// PUT /@ → store a shortcut target, respond with its hash
pub async fn handle_put(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match put_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err((status, message)) => Response::error(message, status)?,
    };
    Ok(with_cors_headers(response))
}

async fn put_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, (u16, String)> {
    let query = req
        .url()
        .map_err(|e| (400, format!("Failed to parse request URL: {e}")))?
        .query()
        .map(str::to_owned);
    let ttl = requested_ttl(query.as_deref()).map_err(|reason| (400, reason))?;

    let body = req
        .bytes()
        .await
        .map_err(|e| (400, format!("Failed to read request body: {e}")))?;
    let shortcut = Shortcut::new(&body).map_err(|reason| (400, reason))?;

    // A repeated PUT overwrites, but the key is derived from the
    // content, so it can only re-store identical bytes — the write is
    // an idempotent expiry refresh, never a repoint.
    let key = shortcut.object_key();
    let hash = shortcut.hash_str();
    let expires = unix_now() + ttl;
    let bucket = ctx
        .bucket("BUCKET")
        .map_err(|e| (500, format!("Missing BUCKET: {e}")))?;
    bucket
        .put(key, shortcut.target.into_bytes())
        .custom_metadata(HashMap::from([(
            EXPIRES_METADATA_KEY.to_string(),
            expires.to_string(),
        )]))
        .execute()
        .await
        .map_err(|e| (500, format!("Failed to store shortcut: {e}")))?;

    Response::from_bytes(hash.into_bytes())
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "text/plain");
            r.with_headers(headers)
        })
        .map_err(|e| (500, format!("Response error: {e}")))
}

/// GET /@/{hash} → permanent redirect to the stored target
pub async fn handle_get(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let query = req.url()?.query().map(str::to_owned);
    let response = match get_inner(&ctx, query.as_deref()).await {
        Ok(response) => response,
        Err((404, _)) => unavailable_response()?,
        Err((status, message)) => Response::error(message, status)?,
    };
    Ok(with_cors_headers(response))
}

fn unavailable_response() -> Result<Response> {
    let response = Response::from_bytes(unavailable_invite_html().into_bytes())?.with_status(404);
    let headers = response.headers().clone();
    headers.set("Content-Type", "text/html; charset=utf-8")?;
    headers.set("Cache-Control", "no-store")?;
    Ok(response.with_headers(headers))
}

async fn get_inner(
    ctx: &RouteContext<()>,
    request_query: Option<&str>,
) -> std::result::Result<Response, (u16, String)> {
    let hash = ctx
        .param("hash")
        .ok_or_else(|| (400, "Missing hash".to_string()))?;
    let key = object_key_for(hash).map_err(|reason| (400, reason))?;

    let bucket = ctx
        .bucket("BUCKET")
        .map_err(|e| (500, format!("Missing BUCKET: {e}")))?;
    let object = bucket
        .get(key)
        .execute()
        .await
        .map_err(|e| (500, format!("Failed to read shortcut: {e}")))?
        .ok_or_else(|| (404, "Not Found".to_string()))?;
    let expires: u64 = object
        .custom_metadata()
        .ok()
        .and_then(|metadata| metadata.get(EXPIRES_METADATA_KEY).cloned())
        .and_then(|expires| expires.parse().ok())
        .ok_or((404, "Not Found".to_string()))?;
    let remaining = expires.saturating_sub(unix_now());
    if remaining == 0 {
        return Err((404, "Not Found".to_string()));
    }

    let bytes = match object.body() {
        Some(body) => body
            .bytes()
            .await
            .map_err(|e| (500, format!("Failed to read body: {e}")))?,
        None => return Err((404, "Not Found".to_string())),
    };
    let target = String::from_utf8(bytes)
        .map_err(|_| (500, "Stored shortcut is not valid UTF-8".to_string()))?;
    let target = referral_redirect_target(&target, request_query);

    // `Location` stays relative on purpose (`Response::redirect`
    // demands an absolute URL, so the header is set by hand): the
    // browser resolves it against this origin, and because it carries
    // no fragment, the short link's `#fragment` is inherited onto the
    // target. Cache no longer than the logical expiry.
    Response::empty()
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Location", &target);
            let _ = headers.set(
                "Cache-Control",
                &format!("public, max-age={}", remaining.min(86_400)),
            );
            r.with_headers(headers).with_status(301)
        })
        .map_err(|e| (500, format!("Response error: {e}")))
}

/// Current time as unix seconds.
fn unix_now() -> u64 {
    Date::now().as_millis() / 1000
}
