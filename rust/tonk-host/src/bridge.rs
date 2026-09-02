//! Guest-side bridge context. Inside a sealed iframe the portal
//! bootstrap installs `window.tonk`, whose `context` (delivered in the
//! host's `ready` envelope) carries what the guest cannot read itself —
//! the host's origin, path, hash, site entity, and the portal's pinned
//! routing context. This module reads that context; it carries NO data
//! transport (all IO is plain `fetch`, which the bootstrap's override
//! relays for a guest).

use crate::error::{ErrorDetail, ErrorKind};
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::window;

/// Read a string field off the bridge `window.tonk.context`, if present and
/// non-empty. The host populates the context (`{this, model, origin, path,
/// hash, repo, branch, with, site}`) in its `ready` envelope; guest controls
/// read routing they can't resolve from the DOM (the pinned `with` context
/// lives outside the iframe) from here.
pub fn context_field(name: &str) -> Option<String> {
    let win = window()?;
    let tonk = Reflect::get(&win, &JsValue::from_str("tonk")).ok()?;
    if tonk.is_undefined() || tonk.is_null() {
        return None;
    }
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    Reflect::get(&context, &JsValue::from_str(name))
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

/// The host page's real origin, as the bridge reports it in
/// `window.tonk.context.origin`. In a sealed guest `window.location.origin`
/// is `"null"` (opaque origin), so anything that needs a same-origin URL —
/// the invite link, the sync `/api` route — must read the origin from the
/// bridge context the host supplies. Falls back to `window.location.origin`
/// when there is no bridge (the element running in the real top document).
pub fn context_origin() -> Option<String> {
    // Reject a forwarded `"null"` as well as an empty one: a nested host whose
    // own location is `about:srcdoc` could forward the opaque `"null"`, and a
    // literal `null` origin must never reach a same-origin URL.
    if let Some(origin) = context_field("origin").filter(|o| o != "null") {
        return Some(origin);
    }
    window()?
        .location()
        .origin()
        .ok()
        .filter(|o| !o.is_empty() && o != "null")
}

thread_local! {
    /// This document's `site` entity, as assigned by the service worker. The SW
    /// derives it from the requesting client id (`site:<client-id>`) when the
    /// page registers via `POST /api/site` — so it is browser-managed and GC-able
    /// rather than a locally-minted uuid. `None` until [`ensure_site`] has run.
    static SITE_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// This document's `site` entity, as assigned by the SW — empty until
/// [`ensure_site`] has registered the page. The shell reads the tab's
/// location/route facts the SW stamped on this entity.
pub fn site_id() -> String {
    SITE_ID.with(|cell| cell.borrow().clone().unwrap_or_default())
}

/// Register this page's site with the service worker and cache the assigned id.
///
/// On first load the navigation predates the SW (the page is served before the
/// SW exists), so the SW never sees a navigation for this document — the page
/// must announce itself. `POST /api/site` (carrying the current path on
/// `X-Tonk-Path`) makes the SW assert this tab's `tonk:site` and return the
/// `site:<client-id>` entity to render against. Idempotent: re-call on
/// navigation to update the site in place.
#[cfg(target_arch = "wasm32")]
pub async fn ensure_site(path: &str) -> Result<String, ErrorDetail> {
    let site = crate::http::post_site(path).await?;
    SITE_ID.with(|cell| *cell.borrow_mut() = Some(site.clone()));
    Ok(site)
}

/// Register this document's site against a per-branch `/site` endpoint (`url`),
/// matching `path` on that branch. The branch is named in `url` (e.g.
/// `/api/profile/branch/main/site`), so the SW does no document-path routing.
/// Returns and caches the `site:<client-id>` entity, like [`ensure_site`].
#[cfg(target_arch = "wasm32")]
pub async fn ensure_site_on(url: &str, path: &str) -> Result<String, ErrorDetail> {
    let site = crate::http::post_site_to(url, path).await?;
    SITE_ID.with(|cell| *cell.borrow_mut() = Some(site.clone()));
    Ok(site)
}

/// Native stub — the `/site` fetch is wasm-only.
#[cfg(not(target_arch = "wasm32"))]
pub async fn ensure_site_on(_url: &str, _path: &str) -> Result<String, ErrorDetail> {
    Err(ErrorDetail::new(
        ErrorKind::Network,
        "ensure_site_on is only available on wasm32",
    ))
}

/// Native stub — the `/api/site` fetch is wasm-only.
#[cfg(not(target_arch = "wasm32"))]
pub async fn ensure_site(_path: &str) -> Result<String, ErrorDetail> {
    Err(ErrorDetail::new(
        ErrorKind::Network,
        "ensure_site is only available on wasm32",
    ))
}

/// The request-context headers every host-relative `/api` request carries, so
/// the SW can tie the request to its originating document and route/contain it:
/// `X-Tonk-Path`, `X-Tonk-Hash`, `X-Tonk-Site`. The host does not interpret
/// these; the SW decides how to use them.
///
/// The document path is stamped explicitly rather than relying on `Referer`: a
/// service worker intercepting the request reads `request.headers`, which never
/// includes `Referer` (the browser exposes it as the separate `request.referrer`
/// property, not as a header), so the SW cannot see it. The host knows its own
/// document path, so it carries it directly. The hash is stamped too (the
/// network strips fragments), only when there is one.
///
/// Path/hash come from the bridge context in a sealed guest (its
/// `window.location` is `about:srcdoc`, useless) and from `window.location` in
/// the top document — the same source split as [`context_origin`].
pub fn context_headers() -> Vec<(&'static str, String)> {
    let path = context_field("path")
        .or_else(|| window().and_then(|w| w.location().pathname().ok()))
        .filter(|path| !path.is_empty());
    let hash = context_field("hash")
        .or_else(|| window().and_then(|w| w.location().hash().ok()))
        .filter(|hash| !hash.is_empty());

    let mut headers = vec![("x-tonk-site", site_id())];
    if let Some(path) = path {
        headers.push(("x-tonk-path", path));
    }
    if let Some(hash) = hash {
        headers.push(("x-tonk-hash", hash));
    }
    headers
}

/// GET a host-relative path and return its body text. One transport
/// everywhere: in a sealed guest `window.fetch` is the portal bootstrap's
/// override, which has the host perform the request and streams the
/// response back, so the same call works in the top document and in any
/// guest.
pub async fn host_fetch_text(path: &str) -> Result<String, ErrorDetail> {
    let win = window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no window"))?;
    let resp_value = JsFuture::from(win.fetch_with_str(path))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch {path}: {e:?}")))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch: not a Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("fetch {path}: {}", resp.status()),
        ));
    }
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text(): {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("await text: {e:?}")))?;
    text.as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "fetch body not a string"))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Object;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Install `window.tonk.context.origin = value` so the reader sees a
    /// host-forwarded origin, mirroring what a parent portal's `ready`
    /// envelope sets on a sealed guest.
    fn set_context_origin(value: &str) {
        let win = window().unwrap();
        let tonk = Object::new();
        let context = Object::new();
        let _ = Reflect::set(&context, &"origin".into(), &JsValue::from_str(value));
        let _ = Reflect::set(&tonk, &"context".into(), &context);
        let _ = Reflect::set(&win, &"tonk".into(), &tonk);
    }

    /// Clear `window.tonk` so a later test sees no bridge context (the
    /// top-document state). `context_field` treats an undefined `tonk` as
    /// absent, so setting it back to `undefined` is enough.
    fn clear_context() {
        let win = window().unwrap();
        let _ = Reflect::set(&win, &"tonk".into(), &JsValue::UNDEFINED);
    }

    /// A real forwarded origin is returned verbatim — this is the value a
    /// nested portal forwards down so a sealed guest can build a same-origin
    /// invite link without reading its `about:srcdoc` location.
    #[dialog_common::test]
    async fn it_prefers_the_forwarded_context_origin() {
        set_context_origin("https://example.test");
        assert_eq!(context_origin().as_deref(), Some("https://example.test"));
        clear_context();
    }

    /// A forwarded `"null"` (a nested host whose own location was
    /// `about:srcdoc`) is treated as absent, so it never surfaces as a
    /// literal `null` origin in an invite URL. It falls through to the real
    /// harness location, which is never `"null"`.
    #[dialog_common::test]
    async fn it_treats_a_null_forwarded_origin_as_absent() {
        set_context_origin("null");
        assert_ne!(
            context_origin().as_deref(),
            Some("null"),
            "a forwarded \"null\" must not surface as the origin",
        );
        clear_context();
    }
}
