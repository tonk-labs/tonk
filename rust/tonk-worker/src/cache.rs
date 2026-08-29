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

use std::sync::OnceLock;

use js_sys::{Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Cache, Request, Response, ServiceWorkerGlobalScope};

/// Prefixes for this worker's caches. The full name is the prefix
/// plus the build id, so every build owns its own caches and two
/// builds can never read or write the same one.
const SHELL_PREFIX: &str = "TONK_SHELL_";
const WORKER_PREFIX: &str = "TONK_WORKER_";

/// The build id, handed in by the JS shim at activate time (see
/// `set_build_id`). The shim gets it from the stamp
/// `scripts/hash-guest.sh` writes, so both sides derive their cache
/// names from one injected value instead of hand-syncing a literal
/// across two languages.
static BUILD_ID: OnceLock<String> = OnceLock::new();

/// Record the build id for cache naming. Called once, from the
/// worker binary's `activate` export, before any cache use.
pub fn set_build_id(id: String) {
    let _ = BUILD_ID.set(id);
}

fn build_id() -> &'static str {
    BUILD_ID.get().map(String::as_str).unwrap_or("dev")
}

/// The build this worker was stamped with, or `None` when it has not
/// been set (a dev build, or a native test). Used by the version
/// handshake, which must not classify anything when it has no identity
/// of its own to compare against.
pub fn current_build_id() -> Option<String> {
    BUILD_ID.get().cloned()
}

/// This build's shell cache name.
fn shell_cache() -> String {
    format!("{SHELL_PREFIX}{}", build_id())
}

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
    // Same-origin only — see `isShellCacheable` in `service_worker.js`,
    // which this mirrors. Excluding opaque responses isn't enough: a
    // CORS-enabled cross-origin GET succeeds normally and would be
    // stored in the app's own shell cache.
    if !is_same_origin(&request.url()) {
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

/// Drop every cache belonging to a build other than this one.
/// Called from `onactivate`, once this worker is the controller —
/// only then is the previous build's cache genuinely nobody's.
///
/// This is what makes an install atomic: the incoming worker
/// populates its OWN shell cache, so the still-serving old worker
/// never observes a half-written one, and the crossing that
/// `serve_navigation`'s prune logic works to avoid can't arise.
pub async fn purge_old_caches() -> Result<(), JsValue> {
    let caches = caches()?;
    let shell = shell_cache();
    let worker = format!("{WORKER_PREFIX}{}", build_id());
    let keys: js_sys::Array = JsFuture::from(caches.keys()).await?.dyn_into()?;
    for key in keys.iter() {
        let Some(name) = key.as_string() else {
            continue;
        };
        // Both families, so a superseded build leaves nothing behind:
        // its shell AND the copy of its wasm the JS shim precached.
        let stale = (name.starts_with(SHELL_PREFIX) && name != shell)
            || (name.starts_with(WORKER_PREFIX) && name != worker);
        if stale {
            let _ = JsFuture::from(caches.delete(&name)).await;
        }
    }
    Ok(())
}

/// Whether `url` is on this worker's own origin.
///
/// A request URL is always absolute and already normalized by the
/// browser, so an origin prefix test is exact here: the character
/// after the origin can only be `/`, `?` or `#`, none of which can
/// appear inside an origin — so `https://evil.test/` cannot match a
/// base of `https://tonk.network`.
///
/// Anything we can't confirm is treated as foreign: refusing to cache
/// it costs a network fetch, whereas caching it wrongly puts another
/// origin's bytes behind our own paths.
fn is_same_origin(url: &str) -> bool {
    let Ok(global) = js_sys::global().dyn_into::<ServiceWorkerGlobalScope>() else {
        return false;
    };
    let base = global.location().origin();
    if base.is_empty() {
        return false;
    }
    match url.strip_prefix(&base) {
        Some("") => true,
        Some(rest) => rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#'),
        None => false,
    }
}

async fn open_cache() -> Result<Cache, JsValue> {
    let cache_value = JsFuture::from(caches()?.open(&shell_cache())).await?;
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::RequestInit;

    wasm_bindgen_test_configure!(run_in_service_worker);

    fn get(url: &str) -> Request {
        let init = RequestInit::new();
        init.set_method("GET");
        Request::new_with_str_and_init(url, &init).expect("request")
    }

    fn origin() -> String {
        js_sys::global()
            .dyn_into::<ServiceWorkerGlobalScope>()
            .expect("service worker scope")
            .location()
            .origin()
    }

    /// A cross-origin GET must never enter the app's shell cache.
    /// Excluding opaque responses was not enough: a CORS-enabled
    /// cross-origin fetch succeeds like any same-origin one, so it
    /// would have been stored under, and later served from, this
    /// app's own cache.
    #[dialog_common::test]
    async fn it_refuses_to_cache_another_origin() {
        assert!(
            !is_cacheable(&get("https://evil.test/app.js"), "/app.js"),
            "a foreign origin must not be shell-cacheable"
        );
        // A prefix test alone would be fooled by a longer host that
        // starts with ours; the origin must end at a path boundary.
        let lookalike = format!("{}.evil.test/app.js", origin());
        assert!(
            !is_cacheable(&get(&lookalike), "/app.js"),
            "a host merely PREFIXED by our origin is still foreign"
        );
    }

    /// The same request on our own origin still caches — the origin
    /// check must not have closed the door on the normal path.
    #[dialog_common::test]
    async fn it_still_caches_our_own_assets() {
        let url = format!("{}/ui-abc123.js", origin());
        assert!(
            is_cacheable(&get(&url), "/ui-abc123.js"),
            "a same-origin static asset is the whole point of the cache"
        );
    }

    /// The data plane is never served from cache, same origin or not.
    #[dialog_common::test]
    async fn it_never_caches_the_data_plane() {
        let url = format!("{}/api/health", origin());
        assert!(!is_cacheable(&get(&url), "/api/health"));
    }

    /// Cache names carry the build id, so two builds cannot share a
    /// cache. That is what makes an install atomic: the incoming
    /// worker populates its OWN shell cache, and the still-serving old
    /// worker can never observe a half-written one.
    #[dialog_common::test]
    async fn it_scopes_cache_names_to_the_build() {
        let name = shell_cache();
        assert!(
            name.starts_with(SHELL_PREFIX),
            "shell cache keeps its prefix so purge can find it: {name}"
        );
        assert!(
            name.ends_with(build_id()),
            "shell cache is scoped to this build: {name}"
        );
        assert_ne!(
            name,
            format!("{SHELL_PREFIX}some-other-build"),
            "a different build must name a different cache"
        );
    }
}
