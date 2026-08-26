//! App-shell cache (milestone 1 of `plan/offline-support.md`).
//!
//! Strategy: stale-while-revalidate for hashed shell assets,
//! network-first with SPA fallback for document navigations.
//!
//! Owned by the Rust worker so the SW JS shim stays a thin
//! dispatcher. The shim forwards every fetch to `onfetch`, and
//! this module decides whether to serve from the shell cache or
//! pass through. Caching policy lives here in one place rather
//! than being split between JS and Rust.

use js_sys::{Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Cache, Request, Response, ServiceWorkerGlobalScope};

/// Cache name. Carries a version segment so an SW update that
/// changes the cache surface invalidates the entire cache
/// atomically — the new worker installs against `_v2`, and
/// `purge_old_caches` (called from `onactivate`) drops every
/// older `TONK_SHELL_*` cache. Bump when the cached-asset
/// surface changes shape.
const SHELL_CACHE: &str = "TONK_SHELL_v1";

/// Should this request be served via the shell cache?
///
/// Same-origin GETs that don't touch the data plane qualify.
/// `/api/*` requests are excluded — those are the live data
/// plane and must always go through the axum router. Document
/// navigations (`mode: navigate`) are also excluded — the JS
/// shim handles those directly so navigation TTFB doesn't wait
/// on the Rust worker boot.
pub fn is_cacheable(request: &Request, path: &str) -> bool {
    if request.method() != "GET" {
        return false;
    }
    if request.mode() == web_sys::RequestMode::Navigate {
        return false;
    }
    if path.starts_with("/api/") {
        return false;
    }
    // Honor a caller's explicit cache-bypass. A `fetch(url, { cache:
    // "no-store" })` (or `"reload"` / `"no-cache"`) signals the caller
    // needs fresh content, so don't serve stale-while-revalidate. The
    // dev hot-reload client uses this to read the just-edited standard
    // library rather than the previous cached copy (the "one version
    // behind" reseed).
    match request.cache() {
        web_sys::RequestCache::NoStore
        | web_sys::RequestCache::Reload
        | web_sys::RequestCache::NoCache => return false,
        _ => {}
    }
    true
}

/// Stale-while-revalidate: serve from cache when present
/// (instant), revalidate in the background so the next visit
/// sees fresh content. On cache miss, hit the network and put
/// the response in the cache. Network failures fall back to the
/// cached entry if one exists; otherwise the error propagates.
pub async fn stale_while_revalidate(request: &Request) -> Result<JsValue, JsValue> {
    let cache = open_cache().await?;
    let cached = cache_match(&cache, request).await?;
    match cached {
        Some(response) => {
            // Background revalidation: spawn a task that hits
            // the network and replaces the cache entry. We
            // intentionally don't await it; the cache miss path
            // serves the user immediately.
            let cache_clone = cache.clone();
            let request_clone = request.clone()?;
            wasm_bindgen_futures::spawn_local(async move {
                let _ = revalidate(&cache_clone, &request_clone).await;
            });
            Ok(JsValue::from(response))
        }
        None => {
            let response = fetch_and_cache(&cache, request).await?;
            Ok(JsValue::from(response))
        }
    }
}

/// Drop every cache key that doesn't match the current version.
/// Called from `onactivate` so the page's first fetch after an
/// SW update doesn't race against a stale entry from the
/// previous worker.
pub async fn purge_old_caches() -> Result<(), JsValue> {
    let caches = caches()?;
    let keys: js_sys::Array = JsFuture::from(caches.keys()).await?.dyn_into()?;
    for key in keys.iter() {
        let Some(name) = key.as_string() else {
            continue;
        };
        if name.starts_with("TONK_SHELL_") && name != SHELL_CACHE {
            let _ = JsFuture::from(caches.delete(&name)).await;
        }
    }
    Ok(())
}

async fn open_cache() -> Result<Cache, JsValue> {
    let cache_value = JsFuture::from(caches()?.open(SHELL_CACHE)).await?;
    cache_value.dyn_into::<Cache>()
}

fn caches() -> Result<web_sys::CacheStorage, JsValue> {
    let global: ServiceWorkerGlobalScope = js_sys::global().dyn_into()?;
    global.caches()
}

async fn cache_match(cache: &Cache, request: &Request) -> Result<Option<Response>, JsValue> {
    let value = JsFuture::from(cache.match_with_request(request)).await?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    Ok(Some(value.dyn_into::<Response>()?))
}

/// Fetch the request from the network and put a clone in the
/// cache. Returns the network response. Skips caching for
/// non-OK and opaque responses since they round-trip oddly.
async fn fetch_and_cache(cache: &Cache, request: &Request) -> Result<Response, JsValue> {
    let response: Response = JsFuture::from(sw_fetch(request)).await?.dyn_into()?;
    if response.ok() && !is_opaque(&response) && content_matches(request, &response) {
        let clone = response.clone()?;
        // Errors here are non-fatal: we still return the
        // response to the caller, the cache just stays cold for
        // this entry.
        let _ = JsFuture::from(cache.put_with_request(request, &clone)).await;
    }
    Ok(response)
}

/// Background revalidation: fetch fresh, replace the cache
/// entry on success. Errors are swallowed so a brief network
/// blip doesn't poison subsequent reads.
async fn revalidate(cache: &Cache, request: &Request) -> Result<(), JsValue> {
    let response: Response = JsFuture::from(sw_fetch(request)).await?.dyn_into()?;
    if response.ok() && !is_opaque(&response) && content_matches(request, &response) {
        let _ = JsFuture::from(cache.put_with_request(request, &response)).await;
    }
    Ok(())
}

/// Whether `response` is plausible content for the request's path,
/// rather than the SPA fallback wearing a 200.
///
/// A server asked for a hashed asset it does not hold (the window
/// while a rebuild or deploy rewrites the dist) answers with
/// `index.html`, status 200. Caching that under the asset URL poisons
/// the entry: every later load serves HTML where the page expects JS
/// or wasm, subresource integrity blocks it, and the boot shell spins
/// until a background revalidation happens to heal the cache. An HTML
/// answer for an asset path is a miss, not content — serve it if we
/// must, but never remember it.
fn content_matches(request: &Request, response: &Response) -> bool {
    const ASSET_EXTENSIONS: [&str; 6] = [".js", ".mjs", ".wasm", ".css", ".woff2", ".map"];

    let url = request.url();
    let path = url.split(['#', '?']).next().unwrap_or(url.as_str());
    if !ASSET_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        return true;
    }
    let content_type = response
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .unwrap_or_default();
    !content_type.starts_with("text/html")
}

/// `Response::type_` is exposed via web-sys as
/// `ResponseType` which we'd need to import. Cheap to read via
/// Reflect to avoid the extra binding noise.
fn is_opaque(response: &Response) -> bool {
    Reflect::get(response, &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|t| t == "opaque" || t == "opaqueredirect")
        .unwrap_or(false)
}

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = fetch)]
    fn sw_fetch(request: &Request) -> Promise;
}
