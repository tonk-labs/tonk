//! Web join page handler.
//!
//! Serves the HTML page and WASM assets for browser-based invite claiming.
//! Users click an invite URL (`/join?access=...&remote=...#<seed>`) and land
//! on this page, which loads `carry-web` (WASM) and performs the redelgation
//! in the browser.

use worker::*;

const JOIN_PAGE: &str = include_str!("../join_page.html");

// WASM assets produced by `wasm-pack build rust/carry-web --target web`
// and copied into the access service source tree at build time.
const CARRY_WEB_JS: &str = include_str!("../assets/carry_web.js");
const CARRY_WEB_WASM: &[u8] = include_bytes!("../assets/carry_web_bg.wasm");

/// Serve the HTML join page.
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let mut response = Response::from_html(JOIN_PAGE)?;
    response
        .headers_mut()
        .set("cache-control", "public, max-age=300")?;
    Ok(response)
}

/// Serve the wasm-bindgen JS glue.
pub async fn handle_js(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let mut response = Response::ok(CARRY_WEB_JS)?;
    response
        .headers_mut()
        .set("content-type", "application/javascript")?;
    // No caching: WASM iterates frequently in dev. Production deployments
    // can layer their own CDN cache via Cloudflare's edge config.
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}

/// Serve the WASM binary.
pub async fn handle_wasm(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let mut response = Response::from_bytes(CARRY_WEB_WASM.to_vec())?;
    response
        .headers_mut()
        .set("content-type", "application/wasm")?;
    response.headers_mut().set("cache-control", "no-store")?;
    Ok(response)
}
