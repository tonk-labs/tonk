//! App-shell cache for immutable service-worker generations.
//!
//! Strategy: immutable generation-cache reads for the complete installed
//! resource graph; a later eviction miss fails closed.
//!
//! The JS shim normally serves top-level assets directly so static requests do
//! not wait for Wasm boot. This mirror handles any cacheable request that does
//! reach the Rust worker and uses `CacheStorage.match`, which cannot recreate
//! an evicted generation cache.

use std::collections::HashSet;
use std::sync::OnceLock;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CacheQueryOptions, Request, Response, ServiceWorkerGlobalScope};

/// Prefixes for this worker's caches. The full name is the prefix
/// plus the build id, so every build owns its own caches and two
/// builds can never read or write the same one.
const SHELL_PREFIX: &str = "TONK_SHELL_";
const WORKER_PREFIX: &str = "TONK_WORKER_";

/// The build id, handed in by the JS shim at activate time (see
/// `set_build_id`). The shim gets it from the identity that
/// `scripts/stamp-service-worker.sh` writes, so both sides derive their cache
/// names from one injected value instead of hand-syncing a literal
/// across two languages.
static BUILD_ID: OnceLock<String> = OnceLock::new();
static ASSET_PATHS: OnceLock<HashSet<String>> = OnceLock::new();

/// Record the build id for cache naming. Called once, from the
/// worker binary's `activate` export, before any cache use.
pub fn set_build_id(id: String) {
    let _ = BUILD_ID.set(id);
}

/// Record the exact immutable resource graph stamped into the JS shim.
/// Called once beside [`set_build_id`] before any cache policy is evaluated.
pub fn set_asset_paths(paths: Vec<String>) {
    let _ = ASSET_PATHS.set(paths.into_iter().collect());
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
/// In a stamped production build, only exact members of the publisher's
/// immutable resource graph qualify. The unstamped development worker keeps
/// the broader same-origin GET policy needed by Trunk. `/api/*` requests are
/// excluded because they are the live data plane. Document navigations are
/// also excluded because the JS shim handles top-level documents directly and
/// sends nested-client navigations here for branch-aware routing.
pub fn is_cacheable(request: &Request, path: &str) -> bool {
    let Some(origin) = worker_origin() else {
        return false;
    };
    is_cacheable_on_origin(request, path, &origin)
}

/// Apply the shell-cache policy relative to a trusted app origin.
///
/// Reading the origin from the ambient service-worker global stays in
/// [`is_cacheable`]. Keeping the policy itself independent of that global lets
/// the browser test harness exercise the production decisions even when the
/// harness runs in a `Window`.
fn is_cacheable_on_origin(request: &Request, path: &str, origin: &str) -> bool {
    is_cacheable_for_build(request, path, origin, build_id())
}

fn is_cacheable_for_build(request: &Request, path: &str, origin: &str, build: &str) -> bool {
    is_cacheable_for_graph(request, path, origin, build, ASSET_PATHS.get())
}

fn is_cacheable_for_graph(
    request: &Request,
    path: &str,
    origin: &str,
    build: &str,
    asset_paths: Option<&HashSet<String>>,
) -> bool {
    if request.method() != "GET" {
        return false;
    }
    // Same-origin only — see `isShellCacheable` in `service_worker.js`,
    // which this mirrors. Excluding opaque responses isn't enough: a
    // CORS-enabled cross-origin GET succeeds normally and would be
    // stored in the app's own shell cache.
    if !is_same_origin(origin, &request.url()) {
        return false;
    }
    if request.mode() == web_sys::RequestMode::Navigate {
        return false;
    }
    if path.starts_with("/api/") {
        return false;
    }
    if build != "dev" && !asset_paths.is_some_and(|paths| paths.contains(path)) {
        return false;
    }
    // Only an unstamped development worker honors explicit cache bypasses;
    // Trunk hot reload needs live edited bytes. A production generation is
    // sealed: caller-controlled `no-store`/`reload`/`no-cache` must not turn an
    // old controller into a stable-name network passthrough.
    if build == "dev" {
        match request.cache() {
            web_sys::RequestCache::NoStore
            | web_sys::RequestCache::Reload
            | web_sys::RequestCache::NoCache => return false,
            _ => {}
        }
    }
    true
}

/// Read this worker's immutable generation cache, failing coherently on a miss.
///
/// Install verifies and writes the complete build-produced resource graph. An
/// old controller can outlive a deployment, so accepting a live stable-name
/// response here would mix current-deployment bytes into a retained generation
/// even without storing them. A miss therefore reports an actionable 503 both
/// online and offline.
pub async fn immutable_cache_first(request: &Request) -> Result<JsValue, JsValue> {
    if let Some(response) = cache_match(request).await? {
        return Ok(JsValue::from(response));
    }
    let init = web_sys::ResponseInit::new();
    init.set_status(503);
    let response = Response::new_with_opt_str_and_init(
        Some(
            "A resource required by this retained Tonk version is unavailable. \
             Reload to check for the current version.",
        ),
        &init,
    )?;
    response
        .headers()
        .set("content-type", "text/plain; charset=utf-8")?;
    response.headers().set("cache-control", "no-store")?;
    Ok(JsValue::from(response))
}

/// The service worker's own origin, or `None` outside a worker scope.
///
/// Cache eligibility fails closed when there is no service-worker global or
/// the worker reports no origin: caching foreign bytes is worse than a network
/// fetch.
fn worker_origin() -> Option<String> {
    js_sys::global()
        .dyn_into::<ServiceWorkerGlobalScope>()
        .ok()
        .map(|global| global.location().origin())
        .filter(|origin| !origin.is_empty())
}

/// Whether `url` is on `origin`.
///
/// A request URL is always absolute and already normalized by the
/// browser, so an origin prefix test is exact here: the character
/// after the origin can only be `/`, `?` or `#`, none of which can
/// appear inside an origin — so `https://evil.test/` cannot match a
/// base of `https://tonk.network`.
///
/// Anything we can't confirm is treated as foreign: another origin's bytes do
/// not belong behind this generation's own paths.
fn is_same_origin(origin: &str, url: &str) -> bool {
    if origin.is_empty() {
        return false;
    }
    match url.strip_prefix(origin) {
        Some("") => true,
        Some(rest) => rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#'),
        None => false,
    }
}

fn caches() -> Result<web_sys::CacheStorage, JsValue> {
    let global: ServiceWorkerGlobalScope = js_sys::global().dyn_into()?;
    global.caches()
}

async fn cache_match(request: &Request) -> Result<Option<Response>, JsValue> {
    let options = CacheQueryOptions::new();
    options.set_cache_name(&shell_cache());
    // Eligibility is decided from the exact same-origin pathname above. The
    // sealed graph stores one canonical response per path, so a cache-busting
    // query must resolve to that path instead of becoming an unrecoverable
    // retained-generation miss.
    options.set_ignore_search(true);
    let value = JsFuture::from(caches()?.match_with_request_and_options(request, &options)).await?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    Ok(Some(value.dyn_into::<Response>()?))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use web_sys::RequestInit;

    const ORIGIN: &str = "https://tonk.example";

    fn get(url: &str) -> Request {
        let init = RequestInit::new();
        init.set_method("GET");
        Request::new_with_str_and_init(url, &init).expect("request")
    }

    fn no_store(url: &str) -> Request {
        let init = RequestInit::new();
        init.set_method("GET");
        init.set_cache(web_sys::RequestCache::NoStore);
        Request::new_with_str_and_init(url, &init).expect("request")
    }

    /// A cross-origin GET must never enter the app's shell cache.
    /// Excluding opaque responses was not enough: a CORS-enabled
    /// cross-origin fetch succeeds like any same-origin one, so it
    /// would have been stored under, and later served from, this
    /// app's own cache.
    #[dialog_common::test]
    async fn it_refuses_to_cache_another_origin() {
        assert!(
            !is_cacheable_on_origin(&get("https://evil.test/app.js"), "/app.js", ORIGIN),
            "a foreign origin must not be shell-cacheable"
        );
        // A prefix test alone would be fooled by a longer host that
        // starts with ours; the origin must end at a path boundary.
        let lookalike = format!("{ORIGIN}.evil.test/app.js");
        assert!(
            !is_cacheable_on_origin(&get(&lookalike), "/app.js", ORIGIN),
            "a host merely PREFIXED by our origin is still foreign"
        );
    }

    /// The same request on our own origin still caches — the origin
    /// check must not have closed the door on the normal path.
    #[dialog_common::test]
    async fn it_still_caches_our_own_assets() {
        let url = format!("{ORIGIN}/ui-abc123.js");
        assert!(
            is_cacheable_on_origin(&get(&url), "/ui-abc123.js", ORIGIN),
            "a same-origin static asset is the whole point of the cache"
        );
    }

    #[dialog_common::test]
    async fn it_honors_cache_bypass_only_for_an_unstamped_development_worker() {
        let url = format!("{ORIGIN}/tonk-prose/tonk-prose-editor.js");
        let request = no_store(&url);
        let assets = HashSet::from(["/tonk-prose/tonk-prose-editor.js".to_string()]);
        assert!(
            is_cacheable_for_graph(
                &request,
                "/tonk-prose/tonk-prose-editor.js",
                ORIGIN,
                "deadbeef",
                Some(&assets),
            ),
            "a production caller cannot escape its sealed generation with a cache flag",
        );
        assert!(
            !is_cacheable_for_graph(
                &request,
                "/tonk-prose/tonk-prose-editor.js",
                ORIGIN,
                "dev",
                None,
            ),
            "Trunk development still needs explicit live asset reads",
        );
    }

    /// The data plane is never served from cache, same origin or not.
    #[dialog_common::test]
    async fn it_never_caches_the_data_plane() {
        let url = format!("{ORIGIN}/api/health");
        assert!(!is_cacheable_on_origin(&get(&url), "/api/health", ORIGIN));
    }

    #[dialog_common::test]
    async fn it_caches_only_exact_members_of_a_stamped_graph() {
        let assets = HashSet::from(["/ui-abc.js".to_string()]);
        let asset = get(&format!("{ORIGIN}/ui-abc.js"));
        let live = get(&format!("{ORIGIN}/.well-known/tonk"));
        assert!(is_cacheable_for_graph(
            &asset,
            "/ui-abc.js",
            ORIGIN,
            "deadbeef",
            Some(&assets),
        ));
        assert!(!is_cacheable_for_graph(
            &live,
            "/.well-known/tonk",
            ORIGIN,
            "deadbeef",
            Some(&assets),
        ));
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
            "shell cache keeps its stable diagnostic prefix: {name}"
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
