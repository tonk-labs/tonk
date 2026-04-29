//! Carry Telemetry Service
//!
//! A minimal Cloudflare Worker that receives anonymous usage pings from
//! the `carry` CLI and writes them to Workers Analytics Engine.
//!
//! No IP addresses are read, logged, or stored.
//!
//! ## Endpoints
//!
//! - `POST /ping` -- record a usage event
//! - `GET  /health` -- health check
//!
//! ## Data point schema (Analytics Engine)
//!
//! - index: blinded user ID (first 16 hex chars of blake3 hash of DID)
//! - blob1: command name (init, query, assert, ...)
//! - blob2: carry version

use serde::Deserialize;
use worker::*;

/// A telemetry ping from the carry CLI.
#[derive(Deserialize, Debug, Clone)]
pub struct Ping {
    /// Blinded user ID -- blake3(salt || DID), truncated to hex
    pub id: String,
    /// Subcommand that was run
    pub command: String,
    /// Carry CLI version
    pub version: String,
}

/// Validate a ping payload. Returns an error message on failure.
pub fn validate_ping(ping: &Ping) -> Result<()> {
    if ping.id.is_empty() || ping.id.len() > 32 || !ping.id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::RustError("invalid id".to_string()));
    }
    if ping.command.is_empty() || ping.command.len() > 64 {
        return Err(Error::RustError("invalid command".to_string()));
    }
    if ping.version.is_empty() || ping.version.len() > 32 {
        return Err(Error::RustError("invalid version".to_string()));
    }
    Ok(())
}

/// Test helpers for integration testing (native HTTP server).
#[cfg(feature = "helpers")]
pub mod helpers;

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        .get_async("/health", handle_health)
        .options_async("/ping", handle_preflight)
        .post_async("/ping", handle_ping)
        .run(req, env)
        .await
}

async fn handle_health(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::ok("OK")
}

async fn handle_preflight(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("Access-Control-Allow-Methods", "POST, OPTIONS")?;
    headers.set("Access-Control-Allow-Headers", "Content-Type")?;
    headers.set("Access-Control-Max-Age", "86400")?;
    Ok(Response::empty()?.with_status(204).with_headers(headers))
}

async fn handle_ping(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let ping: Ping = match req.json().await {
        Ok(p) => p,
        Err(_) => return Response::error("Bad request", 400),
    };

    if validate_ping(&ping).is_err() {
        return Response::error("Bad request", 400);
    }

    // Write to Analytics Engine -- no IP is ever read from the request
    let dataset = ctx.env.analytics_engine("TELEMETRY")?;
    AnalyticsEngineDataPointBuilder::new()
        .indexes([ping.id.as_str()])
        .add_blob(ping.command)
        .add_blob(ping.version)
        .write_to(&dataset)?;

    let headers = Headers::new();
    headers.set("Access-Control-Allow-Origin", "*")?;
    Response::ok("OK").map(|r| r.with_headers(headers))
}
