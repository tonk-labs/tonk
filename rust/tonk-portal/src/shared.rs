//! Shared portal setup logic.
//!
//! Both `<tonk-portal>` and `<tonk-fab-portal>` create a sandboxed iframe
//! and wire it to the bridge; the only difference is how they style that
//! iframe. This module provides the common setup path, parameterised by a
//! caller-supplied style function.
//!
//! # How the guest document gets into its iframe
//!
//! In the usual case the `srcdoc` attribute loads a guest. The document URL
//! is then `about:srcdoc`. WebKit does not load a frame when two of its
//! ancestors already have the requested URL. (See WebKit's
//! `HTMLFrameOwnerElement::isProhibitedSelfReference`. It permits one level
//! of self-reference, not two.) Each sealed guest is `about:srcdoc`. The
//! third nested guest then does not load on Safari. The frame stays at its
//! blank initial document and shows no error.
//!
//! The chain in this app has exactly that depth. Level 1 is the shell's
//! `<tonk-site>` guest. Level 2 is the space chrome's nested `<tonk-site>`.
//! Level 3 is the `<tonk-portal>` of a text/html view. That is why portal
//! views are empty on Safari and iOS while `<tonk-view>` content is correct.
//! WebKit fixed the rule in January 2026 (bugs.webkit.org/305276). Released
//! Safari still has the old rule.
//!
//! The fix: a document that is nested below the top document loads its
//! guests from a `blob:` URL ([`GuestSource::BlobUrl`]). The browser mints
//! a unique URL for each load, so no two frames in one chain can have the
//! same URL. The sandbox, the opaque origin, and the bridge do not change.
//! Only the document URL changes. The top document and its direct guests
//! keep `srcdoc` ([`GuestSource::Srcdoc`]). That is the path all other
//! browsers have always used. A `srcdoc` guest inherits the creator's base
//! URL. A `blob:` document does not. For that reason [`guest_document`]
//! writes an explicit `<base>` when the portal has no space base of its own.
//!
//! Why `blob:` and not `data:`: a `data:` URL carries the whole document in
//! the URL. Chromium rejects a URL above 2 MB, Firefox above 32 MB, and every
//! engine must percent-encode and parse it. A portal document can be very
//! large. A `blob:` URL is 46 characters, has no size limit, and needs no
//! encoding. The code revokes each `blob:` URL as soon as it has started the
//! navigation; see [`guest_blob_url`].
//!
//! # Known limits
//!
//! - The gate counts frames from the browser's top document, not from the
//!   tonk shell. If another page embeds the tonk shell in an iframe, the
//!   shell's level-2 guests (the space chrome's nested `<tonk-site>`) also
//!   change to `blob:`. Those guests run the element runtime. That path has
//!   no test. If you embed tonk as an iframe, expect problems.
//! - WebKit's rule also applies to author content. Levels 1 and 2 are
//!   `about:srcdoc`. An author `<iframe srcdoc>` inside a level-3 guest is
//!   the third `about:srcdoc` in the chain, and Safari does not load it.
//!   Author content below level 2 must use `src=` (a `blob:`, `data:`, or
//!   http URL), not `srcdoc`.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Function, Reflect};
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};

/// How the guest document gets into its iframe. The module docs explain why
/// there are two sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestSource {
    /// The `srcdoc` attribute. The guest's document URL is `about:srcdoc`,
    /// and the guest inherits the creator's base URL. The top document and
    /// its direct guests use this source.
    Srcdoc,
    /// A `blob:` URL in `src`. The browser mints a unique URL for each load,
    /// so WebKit's self-reference rule never matches. WebKit refuses the
    /// third nested `about:srcdoc` frame and all deeper ones. A creator
    /// below the top document uses this source.
    BlobUrl,
}

/// Create and wire up the portal iframe, then store the resulting state.
///
/// `apply_style` receives the newly created (but not yet appended) iframe
/// and should write all CSS properties it needs. Called after the
/// `sandbox` and `allow` attributes are set, before the iframe is
/// appended to the host.
///
/// On success `inner` is set to `Some(state)`. If the DOM is unavailable
/// (no window/document) the function returns early and `inner` stays
/// `None`.
pub(crate) fn connect_portal(
    this: &HtmlElement,
    inner: &RefCell<Option<Rc<RefCell<PortalState>>>>,
    with: Option<Location>,
    allow: Allow,
    apply_style: impl Fn(&HtmlIFrameElement),
) {
    let host: Element = this.clone().into();

    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(iframe) = document.create_element("iframe") else {
        return;
    };
    let Ok(iframe) = iframe.dyn_into::<HtmlIFrameElement>() else {
        return;
    };

    // Opaque-origin sandbox: scripts run but `parent.document` is
    // unreachable. The bridge bootstrap reaches the parent only over
    // a `MessagePort` it opens and transfers in its `hello`.
    //
    // `allow-forms` lets the guest's `<form>`s fire their `submit` event
    // (so declarative `onsubmit=` bindings run); it does NOT let a form
    // navigate the guest away, because the runtime installs a global
    // capture-phase `submit` guard that `preventDefault`s every
    // submission before its native action. `allow-downloads` lets a guest
    // trigger a save (e.g. the blob file card's Download button, which
    // clicks an `<a download>` over an object URL); without it Chrome
    // silently blocks the download. We deliberately withhold
    // `allow-top-navigation` and `allow-same-origin` — the guest still
    // can't reach the parent or a real origin.
    let _ = iframe.set_attribute("sandbox", "allow-scripts allow-forms allow-downloads");

    // Delegate the `clipboard-write` Permissions Policy into the guest so
    // its copy buttons (e.g. the share dialog's invite-link copy) can call
    // `navigator.clipboard.writeText`. This is Permissions Policy, NOT a
    // sandbox grant — orthogonal to the sandbox lockdown above; without it
    // the API is blocked outright regardless of sandbox flags.
    let _ = iframe.set_attribute("allow", "clipboard-write");

    apply_style(&iframe);

    let state = Rc::new(RefCell::new(PortalState::new()));
    // The portal's routing context (`with`) and reach (`allow`) are set by
    // the trusted caller element, never by the guest: `<tonk-site>` parses
    // its attributes, `<tonk-fab-portal>` grants `*`, the generic
    // `<tonk-portal>` grants `self` — so a synced/untrusted content guest
    // can forward a route but the bridge denies anything un-listed.
    state.borrow_mut().set_route(with, allow);
    bridge::register_portal(&iframe, &host, &state);
    install_method_delegates(&host, &state);

    // Append the iframe before you load the document. Then `contentWindow`
    // exists, and the `hello` listener can match the live `contentWindow`
    // to this portal when the bootstrap posts `hello`.
    let _ = host.append_child(&iframe);
    let source = guest_source(&host);
    let html = guest_document(&host, &state.borrow(), source);
    mount_guest(&iframe, &html, source);

    state.borrow_mut().iframe = Some(iframe);
    *inner.borrow_mut() = Some(state);
}

/// Reload the portal's iframe from the current `content` attribute,
/// cancelling live subscriptions first.
///
/// Shared by both portal elements; `content`/`entity`/`model` attribute
/// changes call this.
pub(crate) fn reload_portal(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let mut s = state.borrow_mut();
    s.clear_subs();
    if let Some(iframe) = s.iframe.as_ref() {
        let source = guest_source(host);
        let html = guest_document(host, &s, source);
        reload_guest(iframe, &html, source);
    }
}

/// The document that will own a guest mounted under `host`, and the window
/// of that document. That document is the guest's CREATOR. In production it
/// is the document this runtime runs in, because the host lives there. It is
/// then equal to the global `window`. We resolve it through the host for one
/// reason: a test can then give us a host that lives in a nested frame.
fn creator_document(host: &Element) -> Option<web_sys::Document> {
    host.owner_document()
        .or_else(|| window().and_then(|w| w.document()))
}

fn creator_window(host: &Element) -> Option<web_sys::Window> {
    creator_document(host)
        .and_then(|d| d.default_view())
        .or_else(window)
}

/// The [`GuestSource`] for a guest mounted under `host`.
///
/// If `window.parent !== window.top` in the creator, the creator is two or
/// more frames deep. A `srcdoc` child would then be the third `about:srcdoc`
/// frame in the chain, and WebKit refuses that one. A sealed guest can read
/// `parent` and `top` across the sandbox boundary (identity only). It can
/// make this decision itself.
fn guest_source(host: &Element) -> GuestSource {
    let Some(win) = creator_window(host) else {
        return GuestSource::Srcdoc;
    };
    let parent: JsValue = win
        .parent()
        .ok()
        .flatten()
        .map(JsValue::from)
        .unwrap_or(JsValue::NULL);
    let top: JsValue = win
        .top()
        .ok()
        .flatten()
        .map(JsValue::from)
        .unwrap_or(JsValue::NULL);
    if parent == top {
        GuestSource::Srcdoc
    } else {
        GuestSource::BlobUrl
    }
}

/// The complete guest document for `host`. It contains the bridge
/// bootstrap, the author `content`, and the `<base>` that the guest
/// resolves URLs against. In `runtime` mode it also contains the runtime
/// bootstrap.
///
/// The base is the portal's per-space synthetic origin when the portal has
/// one (see [`space_base`]). When it has none, a `srcdoc` guest inherits the
/// creator's base URL from the browser. A `blob:` document does not: its URL
/// cannot be a base. For [`GuestSource::BlobUrl`] this function writes the
/// inherited value (the creator's `document.baseURI`) explicitly. Link and
/// request resolution are then identical for the two sources.
fn guest_document(host: &Element, state: &PortalState, source: GuestSource) -> String {
    let content = host.get_attribute("content").unwrap_or_default();
    let mut base = space_base(state);
    if base.is_empty() && source == GuestSource::BlobUrl {
        base = creator_document(host)
            .and_then(|d| d.base_uri().ok().flatten())
            .unwrap_or_default();
    }
    // In `runtime` mode the guest renders OUR elements (a real
    // `<tonk-display>`). The bootstrap then also loads the injected element
    // runtime and CSS before `content` upgrades.
    if host.has_attribute("runtime") {
        bridge::bootstrap_srcdoc_with_runtime(&content, &base)
    } else {
        bridge::bootstrap_srcdoc(&content, &base)
    }
}

/// The first load of `html` into an `iframe` that was just appended. With
/// either source the frame navigates away from its initial blank document.
/// The browser treats that as a replacement. It adds no
/// joint-session-history entry.
fn mount_guest(iframe: &HtmlIFrameElement, html: &str, source: GuestSource) {
    match source {
        GuestSource::Srcdoc => {
            let _ = iframe.set_attribute("srcdoc", html);
        }
        GuestSource::BlobUrl => match guest_blob_url(html) {
            Some(url) => {
                let _ = iframe.set_attribute("src", &url);
                revoke_guest_url(&url);
            }
            // The browser could not mint a URL. Use `srcdoc` instead of
            // nothing. That works everywhere except in deep chains on WebKit.
            None => {
                tonk_common::log!("tonk-portal: createObjectURL failed; falling back to srcdoc");
                let _ = iframe.set_attribute("srcdoc", html);
            }
        },
    }
}

/// Reload a live `iframe` with new `html` (after a `content` change).
///
/// A `srcdoc` guest reloads when we assign the attribute again, as before.
/// A `blob:` guest reloads through the frame's own `location.replace()`,
/// not through a new `src`. A new `src` on a live frame NAVIGATES the frame,
/// and a frame navigation adds a joint-session-history entry (see the
/// teardown notes in `element.rs`). `replace` changes the entry in place.
/// `location.replace()` is permitted across the sandbox boundary. The `src`
/// attribute keeps the URL of the first load. The live document is the
/// truth.
fn reload_guest(iframe: &HtmlIFrameElement, html: &str, source: GuestSource) {
    match source {
        GuestSource::Srcdoc => {
            let _ = iframe.set_attribute("srcdoc", html);
        }
        GuestSource::BlobUrl => {
            let Some(url) = guest_blob_url(html) else {
                tonk_common::log!("tonk-portal: createObjectURL failed; falling back to srcdoc");
                let _ = iframe.set_attribute("srcdoc", html);
                return;
            };
            match iframe.content_window() {
                Some(frame_window) => {
                    if let Err(error) = frame_window.location().replace(&url) {
                        // The browser refused the in-place navigation. Load
                        // through `src`, so that the new content still shows.
                        // This adds one history entry. The log says so.
                        tonk_common::log!(
                            "tonk-portal: location.replace failed ({error:?}); reloading via src"
                        );
                        let _ = iframe.set_attribute("src", &url);
                    }
                }
                // There is no live window (the frame was never mounted). Do
                // a first load.
                None => {
                    let _ = iframe.set_attribute("src", &url);
                }
            }
            revoke_guest_url(&url);
        }
    }
}

/// Put a guest document into a `blob:` URL that the frame can load.
///
/// The browser mints a unique URL for each call. WebKit's self-reference
/// rule compares frame URLs, so it never matches an ancestor. The blob holds
/// the document bytes as they are: no encoding, no size limit, and a
/// 46-character URL. Returns `None` when the browser cannot make the URL.
///
/// The caller must call [`revoke_guest_url`] after it has started the
/// navigation. A `blob:` URL keeps its bytes in memory until the code
/// revokes it.
pub(crate) fn guest_blob_url(html: &str) -> Option<String> {
    let parts = js_sys::Array::of1(&JsValue::from_str(html));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/html;charset=utf-8");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Revoke a guest `blob:` URL directly after the navigation to it has
/// started.
///
/// This is safe. The HTML "navigate" algorithm resolves a `blob:` URL when
/// the navigation starts. The browser then keeps the bytes alive until the
/// load completes. Chromium takes a blob token at navigation start. WebKit
/// keeps a scheduled blob navigation alive since bug 243936 (2022). We
/// confirmed both with a 24 MB document three frames deep: a first load
/// through `src`, and a reload through `location.replace()`.
///
/// Because the code revokes at once, it holds no URL state and needs no
/// `load` listener. The iframe's `load` event is not a usable signal. An
/// iframe appended without `src` fires `load` for its initial blank
/// document. A "revoke on next load" would then revoke too early.
pub(crate) fn revoke_guest_url(url: &str) {
    let _ = web_sys::Url::revoke_object_url(url);
}

/// The per-space synthetic origin (`https://{label}.tonk.network/`) for this
/// portal's routing context, or empty when the portal targets no space (the
/// profile/Hub) — where in-guest links are genuinely top-level.
fn space_base(state: &bridge::PortalState) -> String {
    state
        .route_space()
        .and_then(|space| tonk_host::space_origin::space_origin_for(&space))
        .unwrap_or_default()
}

/// Install `reset` / `update` / `error` on the named element's prototype,
/// each forwarding to the per-instance `__tonk*` closure.
///
/// Called once during `register()` / `register_fab_portal()` after
/// `CustomElement::define` so the constructor is already registered.
/// The `element_name` must match the name passed to `define`.
pub(crate) fn install_method_shims(element_name: &str) {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get(element_name);
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let error_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkError === 'function') this.__tonkError(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
    let _ = Reflect::set(&proto, &"error".into(), &error_fn);
}

/// Write the per-instance `__tonkReset` / `__tonkError` closures the
/// prototype shims forward subscription frames to. Mirrors
/// `<tonk-display>`'s method-delegate pattern.
pub(crate) fn install_method_delegates(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let reset_state = state.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_reset(&reset_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let update_state = state.clone();
    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_update(&update_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkUpdate".into(), update.as_ref());
    update.forget();

    let error_state = state.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_error(&error_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Document, MessageEvent};

    wasm_bindgen_test_configure!(run_in_browser);

    const BLOB_PREFIX: &str = "blob:";

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Fetch `url` from this page and return the body as text. Returns `Err`
    /// when the fetch fails. For a `blob:` URL that means the URL is revoked.
    async fn fetch_text(url: &str) -> Result<String, JsValue> {
        let response = JsFuture::from(window().expect("window").fetch_with_str(url)).await?;
        let response: web_sys::Response = response.dyn_into()?;
        let text = JsFuture::from(response.text()?).await?;
        Ok(text.as_string().unwrap_or_default())
    }

    /// A new sealed iframe appended to the body, set up as `connect_portal`
    /// does it: sandbox first, then attached.
    fn sealed_iframe() -> HtmlIFrameElement {
        let iframe = document()
            .create_element("iframe")
            .expect("create iframe")
            .dyn_into::<HtmlIFrameElement>()
            .expect("HtmlIFrameElement");
        iframe
            .set_attribute("sandbox", "allow-scripts allow-forms allow-downloads")
            .expect("sandbox");
        document()
            .body()
            .expect("body")
            .append_child(&iframe)
            .expect("attach");
        iframe
    }

    /// Resolves to `true` when the bridge bootstrap in `iframe` posts its
    /// `hello` to this (parent) window. Resolves to `false` after a timeout.
    /// Install it BEFORE you load the guest: the bootstrap posts `hello` as
    /// soon as the document loads.
    fn hello_from(iframe: &HtmlIFrameElement) -> JsFuture {
        let target: JsValue = iframe.content_window().expect("contentWindow").into();
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let win = window().expect("window");
            let target = target.clone();
            let on_hello = resolve.clone();
            let listener = Closure::wrap(Box::new(move |event: MessageEvent| {
                let source = event.source().map(JsValue::from).unwrap_or(JsValue::NULL);
                let kind = Reflect::get(&event.data(), &"type".into())
                    .ok()
                    .and_then(|v| v.as_string());
                if source == target && kind.as_deref() == Some("hello") {
                    let _ = on_hello.call1(&JsValue::NULL, &JsValue::TRUE);
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            let _ =
                win.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
            listener.forget();

            let on_timeout = resolve.clone();
            let timeout = Closure::once_into_js(move || {
                let _ = on_timeout.call1(&JsValue::NULL, &JsValue::FALSE);
            });
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                timeout.unchecked_ref(),
                8_000,
            );
        });
        JsFuture::from(promise)
    }

    #[dialog_common::test]
    fn a_top_level_document_loads_its_guests_through_srcdoc() {
        // The test page is the top document. Its guests are the first nested
        // `about:srcdoc` level. All engines permit that level.
        let host: Element = document().body().expect("body").into();
        assert_eq!(guest_source(&host), GuestSource::Srcdoc);
    }

    #[dialog_common::test]
    async fn a_guest_blob_url_serves_the_document_and_is_unique_per_load() {
        let html = "<base href=\"https://x.tonk.network/\"><script>1<2</script>\
                    <p>hi &amp; <b>\"q\"</b> 100% #frag ünïcödé</p>";
        let first = guest_blob_url(html).expect("a blob URL");
        let second = guest_blob_url(html).expect("a blob URL");
        assert!(first.starts_with(BLOB_PREFIX), "got: {first}");
        assert_ne!(
            first, second,
            "two loads of one document must never share a URL — WebKit's \
             self-reference rule compares ancestor URLs"
        );
        let served = fetch_text(&first)
            .await
            .expect("the blob URL serves the document");
        assert_eq!(served, html, "the document must arrive byte-for-byte");

        revoke_guest_url(&first);
        revoke_guest_url(&second);
        assert!(
            fetch_text(&first).await.is_err(),
            "a revoked URL serves nothing; the bytes are free"
        );
    }

    #[dialog_common::test]
    fn a_blob_url_guest_writes_its_inherited_base_explicitly() {
        let host = document().create_element("tonk-portal").expect("host");
        host.set_attribute("content", "<p>hi</p>").expect("content");
        // No `with` means no space base. This is the case that depended on
        // the browser's srcdoc base-URL inheritance.
        let state = PortalState::new();

        let as_srcdoc = guest_document(&host, &state, GuestSource::Srcdoc);
        assert!(
            !as_srcdoc.contains("<base "),
            "a srcdoc guest inherits its base from the browser; got: {as_srcdoc}"
        );

        let as_blob = guest_document(&host, &state, GuestSource::BlobUrl);
        let inherited = document().base_uri().expect("baseURI").expect("set");
        assert!(
            as_blob.starts_with(&format!("<base href=\"{inherited}\">")),
            "a blob: guest must carry the creator's base URL explicitly; got: {}",
            &as_blob[..as_blob.len().min(160)]
        );
        assert!(as_blob.contains("<p>hi</p>"), "content survives");
    }

    /// The load path that WebKit needs. A sealed guest that we load from a
    /// `blob:` URL still runs the bridge bootstrap: it posts `hello` to its
    /// parent. A reload through `location.replace()` runs the bootstrap
    /// again. The code revokes each URL as soon as the navigation has
    /// started. Both loads still succeed. After the load, the URL serves
    /// nothing.
    #[dialog_common::test]
    async fn a_blob_url_guest_boots_the_bridge_reloads_in_place_and_revokes_its_url() {
        let iframe = sealed_iframe();
        let first = hello_from(&iframe);
        mount_guest(
            &iframe,
            &bridge::bootstrap_srcdoc("<p>one</p>", ""),
            GuestSource::BlobUrl,
        );
        assert_eq!(
            first.await.ok().and_then(|v| v.as_bool()),
            Some(true),
            "the blob: guest must boot and greet its parent"
        );
        let src = iframe
            .get_attribute("src")
            .expect("the first load is the frame's `src`");
        assert!(src.starts_with(BLOB_PREFIX), "got: {src}");
        assert_eq!(iframe.get_attribute("srcdoc"), None);
        assert!(
            fetch_text(&src).await.is_err(),
            "the URL is revoked once the navigation has started"
        );

        let again = hello_from(&iframe);
        reload_guest(
            &iframe,
            &bridge::bootstrap_srcdoc("<p>two</p>", ""),
            GuestSource::BlobUrl,
        );
        assert_eq!(
            again.await.ok().and_then(|v| v.as_bool()),
            Some(true),
            "a reload must bring the guest back up"
        );
        // `replace` keeps `src` at the first URL. The live document is the
        // truth.
        assert_eq!(iframe.get_attribute("src").as_deref(), Some(src.as_str()));
        iframe.remove();
    }
}

/// The WebKit case from end to end, through the production mount path. A
/// guest created from a document that is already TWO frames below the top
/// must still boot. Safari refuses a third `about:srcdoc` frame (see the
/// module docs). Without the `blob:` source this test times out there. With
/// it, the test passes. Chrome loads both. The test uses only
/// `connect_portal` on purpose. The same test then runs against the code
/// before the fix and shows the failure.
///
/// CI runs this test in headless Chrome, as it runs all tests here. Chrome
/// loads a third `about:srcdoc` frame, so the boot alone cannot catch a
/// broken gate there. For that reason the test also asserts the mechanism:
/// a guest created two frames deep has a `blob:` `src`. To see the Safari
/// failure and pass on your machine, enable Safari ▸ Develop ▸ Allow Remote
/// Automation, then run:
///
/// ```text
/// SAFARIDRIVER=/usr/bin/safaridriver \
///   cargo test -p tonk-portal --target wasm32-unknown-unknown -- nested_tests
/// ```
#[cfg(test)]
mod nested_tests {
    use super::*;
    use js_sys::Promise;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Document, MessageEvent, Window};

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Append a `srcdoc` frame under the body of `parent` and wait until it
    /// loads. The frame is same-origin (`allow-same-origin`), so the test
    /// can reach into it. Its URL is still `about:srcdoc`. WebKit's
    /// self-reference rule looks only at the URL.
    async fn srcdoc_frame(parent: &Document) -> HtmlIFrameElement {
        // Use `unchecked_into`, not `dyn_into`. An element created in a
        // nested document belongs to that realm. `dyn_into` does an
        // `instanceof` check against the constructor of THIS realm, and that
        // check is false across realms.
        let iframe = parent
            .create_element("iframe")
            .expect("create iframe")
            .unchecked_into::<HtmlIFrameElement>();
        iframe
            .set_attribute("sandbox", "allow-scripts allow-same-origin")
            .expect("sandbox");
        // Set `srcdoc` BEFORE insertion. An iframe inserted without it fires
        // `load` for its initial about:blank. That would resolve the promise
        // too early, before the srcdoc document exists. The srcdoc document
        // is the one we return.
        iframe
            .set_attribute("srcdoc", "<!doctype html><body></body>")
            .expect("srcdoc");
        let loaded = Promise::new(&mut |resolve, _reject| {
            iframe.set_onload(Some(&resolve));
        });
        parent
            .body()
            .expect("parent body")
            .append_child(&iframe)
            .expect("attach");
        let _ = JsFuture::from(loaded).await;
        iframe
    }

    /// Resolves to `true` when a `hello` from the window `from` reaches the
    /// window `on` (the guest's parent). Resolves to `false` after a timeout.
    fn hello_on(on: &Window, from: &JsValue) -> JsFuture {
        let from = from.clone();
        let on = on.clone();
        let promise = Promise::new(&mut |resolve, _reject| {
            let from = from.clone();
            let on_hello = resolve.clone();
            let listener = Closure::wrap(Box::new(move |event: MessageEvent| {
                let source = event.source().map(JsValue::from).unwrap_or(JsValue::NULL);
                let kind = Reflect::get(&event.data(), &"type".into())
                    .ok()
                    .and_then(|v| v.as_string());
                if source == from && kind.as_deref() == Some("hello") {
                    let _ = on_hello.call1(&JsValue::NULL, &JsValue::TRUE);
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            let _ =
                on.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
            listener.forget();

            let on_timeout = resolve.clone();
            let timeout = Closure::once_into_js(move || {
                let _ = on_timeout.call1(&JsValue::NULL, &JsValue::FALSE);
            });
            let _ = on.set_timeout_with_callback_and_timeout_and_arguments_0(
                timeout.unchecked_ref(),
                8_000,
            );
        });
        JsFuture::from(promise)
    }

    #[dialog_common::test]
    async fn a_guest_created_two_frames_deep_still_boots() {
        // top → A → B, both `about:srcdoc`. This is the real shape (shell
        // site guest → nested site of the space chrome) without the element
        // runtime.
        let a = srcdoc_frame(&document()).await;
        let a_doc = a.content_document().expect("A is same-origin");
        let b = srcdoc_frame(&a_doc).await;
        let b_doc = b.content_document().expect("B is same-origin");
        let b_win = b.content_window().expect("B window");

        // The portal host lives in B. B is then the guest's creator.
        let host = b_doc
            .create_element("div")
            .expect("host")
            .unchecked_into::<HtmlElement>();
        host.set_attribute("content", "<p>deep</p>")
            .expect("content");
        b_doc
            .body()
            .expect("B body")
            .append_child(&host)
            .expect("attach host");

        // The production mount path, called as `<tonk-portal>` calls it.
        let cell: RefCell<Option<Rc<RefCell<PortalState>>>> = RefCell::new(None);
        connect_portal(&host, &cell, None, Allow::none(), |_| {});
        let guest = host
            .query_selector("iframe")
            .expect("query")
            .expect("guest iframe mounted")
            .unchecked_into::<HtmlIFrameElement>();
        let guest_window: JsValue = guest.content_window().expect("guest window").into();

        // The mechanism. This assertion fails on every engine if the gate
        // breaks, so CI (Chrome) catches that too.
        assert!(
            guest
                .get_attribute("src")
                .is_some_and(|src| src.starts_with("blob:")),
            "a guest created two frames deep must load from a blob: URL, not srcdoc; \
             got src={:?} srcdoc={:?}",
            guest.get_attribute("src"),
            guest.get_attribute("srcdoc").map(|s| s.len())
        );

        // The bootstrap posts `hello` to its parent (the window of B) as its
        // first action. If no `hello` arrives, the frame did not load.
        let booted = hello_on(&b_win, &guest_window).await;
        assert_eq!(
            booted.ok().and_then(|v| v.as_bool()),
            Some(true),
            "a guest two frames below the top must boot — WebKit refuses a \
             third nested about:srcdoc frame, so it needs a different URL"
        );
        a.remove();
    }
}
