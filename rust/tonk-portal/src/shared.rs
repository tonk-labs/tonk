//! Shared portal setup logic.
//!
//! Both `<tonk-portal>` and `<tonk-fab-portal>` create a sandboxed iframe
//! and wire it to the bridge; the only difference is how they style that
//! iframe. This module provides the common setup path, parameterised by a
//! caller-supplied style function.
//!
//! # How the guest document reaches its iframe
//!
//! A guest is normally loaded through `srcdoc`, so its document URL is
//! `about:srcdoc`. WebKit refuses to load a frame when TWO of its ancestors
//! already carry the requested URL (`HTMLFrameOwnerElement::
//! isProhibitedSelfReference` — "we allow one level of self-reference"), and
//! every sealed guest is `about:srcdoc`, so the THIRD nested guest never
//! loads on Safari: the frame silently stays at its blank initial document.
//! The chain is exactly that deep in practice — the shell's `<tonk-site>`
//! guest, the space chrome's nested `<tonk-site>`, then a text/html view's
//! `<tonk-portal>` — which is why portal views render empty on Safari and
//! iOS while `<tonk-view>` content renders fine. WebKit fixed this upstream
//! (bugs.webkit.org/305276, 2026-01), but shipping Safari still refuses it.
//!
//! So a guest created by a document that is itself nested below the top
//! document ([`GuestSource::DataUrl`]) is loaded from a unique
//! `data:text/html` URL instead. Same sandbox, same opaque origin, same
//! bridge; only the document URL differs, and no two frames in one chain can
//! share it. The top document and its direct guests keep `srcdoc`
//! ([`GuestSource::Srcdoc`]), which is the path every other browser has
//! always run. The one thing `srcdoc` gives a guest that a `data:` document
//! lacks is the creator's base URL, so [`guest_document`] writes that out as
//! an explicit `<base>` when the portal has no space base of its own.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Function, Reflect};
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};

/// How a guest document is handed to its iframe. See the module docs for
/// why two sources exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestSource {
    /// The `srcdoc` attribute: the guest's document URL is `about:srcdoc`
    /// and it inherits the creator's base URL. Used by the top document and
    /// by guests that are its direct children.
    Srcdoc,
    /// A unique `data:text/html;charset=utf-8,…` URL in `src`: sidesteps
    /// WebKit's self-reference refusal for the third (and any deeper) nested
    /// `about:srcdoc` frame. Used by any creator that is itself nested below
    /// the top document.
    DataUrl,
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

    // Append before loading the document so `contentWindow` exists;
    // the `hello` listener matches the live `contentWindow`, so the
    // bootstrap script resolves this portal when it posts `hello`.
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

/// The document that will own a guest mounted under `host`, and its window
/// — the guest's CREATOR. In production that is the document this runtime
/// runs in (the host lives there), so this equals the global `window`;
/// resolving it through the host keeps the decision attached to the element
/// rather than the global, which is also what lets a test hand in a host
/// adopted into a nested frame.
fn creator_document(host: &Element) -> Option<web_sys::Document> {
    host.owner_document()
        .or_else(|| window().and_then(|w| w.document()))
}

fn creator_window(host: &Element) -> Option<web_sys::Window> {
    creator_document(host)
        .and_then(|d| d.default_view())
        .or_else(window)
}

/// Which [`GuestSource`] a guest mounted under `host` gets.
///
/// `window.parent !== window.top` in the creator means it is at least two
/// frames deep, so a `srcdoc` child would be the third `about:srcdoc` in
/// the chain — the one WebKit refuses. `parent` and `top` are readable
/// across the sandbox boundary (identity only), so a sealed guest can
/// answer this for itself.
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
        GuestSource::DataUrl
    }
}

/// The complete guest document for `host`: the bridge bootstrap (plus the
/// runtime bootstrap in `runtime` mode), the author `content`, and the
/// `<base>` the guest resolves URLs against.
///
/// The base is the portal's per-space synthetic origin when it has one
/// (see [`space_base`]). Without one, a `srcdoc` guest inherits the
/// creator's base URL from the browser, but a `data:` document's base is the
/// data URL itself — so for [`GuestSource::DataUrl`] the inherited value
/// (the creator's `document.baseURI`) is written out explicitly, keeping
/// link and request resolution identical between the two sources.
fn guest_document(host: &Element, state: &PortalState, source: GuestSource) -> String {
    let content = host.get_attribute("content").unwrap_or_default();
    let mut base = space_base(state);
    if base.is_empty() && source == GuestSource::DataUrl {
        base = creator_document(host)
            .and_then(|d| d.base_uri().ok().flatten())
            .unwrap_or_default();
    }
    // In `runtime` mode the guest renders OUR elements (a real
    // `<tonk-display>`): the bootstrap additionally pulls in the
    // injected element runtime + CSS before `content` upgrades.
    if host.has_attribute("runtime") {
        bridge::bootstrap_srcdoc_with_runtime(&content, &base)
    } else {
        bridge::bootstrap_srcdoc(&content, &base)
    }
}

/// First load of `html` into a freshly appended `iframe`. Either source
/// navigates the frame away from its initial blank document, which the
/// browser treats as a replacement — no joint-session-history entry.
fn mount_guest(iframe: &HtmlIFrameElement, html: &str, source: GuestSource) {
    match source {
        GuestSource::Srcdoc => {
            let _ = iframe.set_attribute("srcdoc", html);
        }
        GuestSource::DataUrl => {
            let _ = iframe.set_attribute("src", &guest_data_url(html));
        }
    }
}

/// Reload a live `iframe` with new `html` (a `content` change).
///
/// A `srcdoc` guest is reloaded by reassigning the attribute, as before. A
/// `data:` guest goes through the frame's own `location.replace()` rather
/// than a fresh `src`: setting `src` on a live frame NAVIGATES it and a
/// frame navigation appends a joint-session-history entry (see the
/// teardown notes in `element.rs`), whereas `replace` swaps the entry in
/// place. `location.replace()` is permitted across the sandbox boundary. The
/// `src` attribute is left as the URL of the first load; the live document
/// is the truth.
fn reload_guest(iframe: &HtmlIFrameElement, html: &str, source: GuestSource) {
    match source {
        GuestSource::Srcdoc => {
            let _ = iframe.set_attribute("srcdoc", html);
        }
        GuestSource::DataUrl => {
            let url = guest_data_url(html);
            match iframe.content_window() {
                Some(frame_window) => {
                    let _ = frame_window.location().replace(&url);
                }
                // No live window (never mounted): fall back to a first load.
                None => {
                    let _ = iframe.set_attribute("src", &url);
                }
            }
        }
    }
}

/// Wrap a guest document in a `data:text/html;charset=utf-8,…` URL that
/// is unique to this load.
///
/// Unique, because WebKit's self-reference rule compares frame URLs: two
/// guests with byte-identical documents in one ancestor chain would share
/// a URL and trip it again. The marker is a trailing HTML comment — inert
/// for the parser and invisible to the content. Percent-encoding (not
/// base64) so the document decodes to exactly the string we built; it
/// roughly doubles the size, well inside what browsers accept for a frame
/// `src` (Chromium caps URLs at 2 MB; guest documents are tens of KB).
pub(crate) fn guest_data_url(html: &str) -> String {
    let nonce = format!(
        "{:08x}{:08x}",
        (js_sys::Math::random() * 4_294_967_296.0) as u32,
        (js_sys::Math::random() * 4_294_967_296.0) as u32,
    );
    let marked = format!("{html}<!--tonk-guest:{nonce}-->");
    format!(
        "data:text/html;charset=utf-8,{}",
        String::from(js_sys::encode_uri_component(&marked))
    )
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

    const DATA_PREFIX: &str = "data:text/html;charset=utf-8,";

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// A fresh sealed iframe appended to the body, exactly as
    /// `connect_portal` sets one up (sandbox first, then attached).
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

    /// Resolve to `true` when the bridge bootstrap inside `iframe` posts its
    /// `hello` to this (parent) window, or `false` after a timeout. Install
    /// BEFORE loading the guest — the bootstrap posts synchronously on load.
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
        // The test page is the top document: its guests are the first nested
        // `about:srcdoc` level, which every engine allows.
        let host: Element = document().body().expect("body").into();
        assert_eq!(guest_source(&host), GuestSource::Srcdoc);
    }

    #[dialog_common::test]
    fn a_guest_data_url_round_trips_the_document_and_is_unique_per_load() {
        let html = "<base href=\"https://x.tonk.network/\"><script>1<2</script>\
                    <p>hi &amp; <b>\"q\"</b> 100% #frag</p>";
        let first = guest_data_url(html);
        let second = guest_data_url(html);
        assert!(first.starts_with(DATA_PREFIX), "got: {first}");
        assert_ne!(
            first, second,
            "two loads of one document must never share a URL — WebKit's \
             self-reference rule compares ancestor URLs"
        );
        let decoded = String::from(
            js_sys::decode_uri_component(&first[DATA_PREFIX.len()..]).expect("decodes"),
        );
        assert!(
            decoded.starts_with(html),
            "the document must decode byte-for-byte; got: {decoded}"
        );
        assert!(
            decoded[html.len()..].starts_with("<!--tonk-guest:"),
            "the uniqueness marker is a trailing comment; got: {}",
            &decoded[html.len()..]
        );
    }

    #[dialog_common::test]
    fn a_data_url_guest_writes_its_inherited_base_explicitly() {
        let host = document().create_element("tonk-portal").expect("host");
        host.set_attribute("content", "<p>hi</p>").expect("content");
        // No `with`, so no space base — the case that used to rely on the
        // browser's srcdoc base-URL inheritance.
        let state = PortalState::new();

        let as_srcdoc = guest_document(&host, &state, GuestSource::Srcdoc);
        assert!(
            !as_srcdoc.contains("<base "),
            "a srcdoc guest inherits its base from the browser; got: {as_srcdoc}"
        );

        let as_data = guest_document(&host, &state, GuestSource::DataUrl);
        let inherited = document().base_uri().expect("baseURI").expect("set");
        assert!(
            as_data.starts_with(&format!("<base href=\"{inherited}\">")),
            "a data: guest must carry the creator's base URL explicitly; got: {}",
            &as_data[..as_data.len().min(160)]
        );
        assert!(as_data.contains("<p>hi</p>"), "content survives");
    }

    /// The load path WebKit needs: a sealed guest handed over as a `data:`
    /// URL still runs the bridge bootstrap (it posts `hello` to its parent),
    /// and a reload through `location.replace()` runs it again.
    #[dialog_common::test]
    async fn a_data_url_guest_boots_the_bridge_and_reloads_in_place() {
        let iframe = sealed_iframe();
        let first = hello_from(&iframe);
        mount_guest(
            &iframe,
            &bridge::bootstrap_srcdoc("<p>one</p>", ""),
            GuestSource::DataUrl,
        );
        assert_eq!(
            first.await.ok().and_then(|v| v.as_bool()),
            Some(true),
            "the data: guest must boot and greet its parent"
        );
        assert!(
            iframe
                .get_attribute("src")
                .is_some_and(|src| src.starts_with(DATA_PREFIX)),
            "the first load is the frame's `src`"
        );
        assert_eq!(iframe.get_attribute("srcdoc"), None);

        let again = hello_from(&iframe);
        reload_guest(
            &iframe,
            &bridge::bootstrap_srcdoc("<p>two</p>", ""),
            GuestSource::DataUrl,
        );
        assert_eq!(
            again.await.ok().and_then(|v| v.as_bool()),
            Some(true),
            "a reload must bring the guest back up"
        );
        iframe.remove();
    }
}

/// The WebKit case, end to end, through the production mount path: a guest
/// created from a document that is already TWO frames below the top must
/// still boot. Safari refuses a third `about:srcdoc` frame outright (see the
/// module docs), so without the `data:` source this test times out there and
/// passes with it; Chrome loads either. Self-contained on purpose — it uses
/// only `connect_portal`, so the very same test runs against the pre-fix
/// code to show the red.
///
/// CI runs this in headless Chrome like every other test here. To see the
/// Safari red/green locally (Safari ▸ Develop ▸ Allow Remote Automation):
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

    /// Append a `srcdoc` frame under `parent`'s body and wait for it to
    /// load. Same-origin (`allow-same-origin`) so the test can reach inside
    /// it; its URL is still `about:srcdoc`, which is all WebKit's
    /// self-reference rule looks at.
    async fn srcdoc_frame(parent: &Document) -> HtmlIFrameElement {
        // `unchecked_into`, not `dyn_into`: an element created in a nested
        // document belongs to that realm, and `dyn_into`'s `instanceof`
        // checks against THIS realm's constructor — false across realms.
        let iframe = parent
            .create_element("iframe")
            .expect("create iframe")
            .unchecked_into::<HtmlIFrameElement>();
        iframe
            .set_attribute("sandbox", "allow-scripts allow-same-origin")
            .expect("sandbox");
        // `srcdoc` BEFORE insertion: an iframe inserted without one fires
        // `load` for its initial about:blank, which would resolve this too
        // early — before the srcdoc document (the one we hand back) exists.
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

    /// Resolve to `true` when a `hello` from the window `from` reaches
    /// `on` (the guest's parent), or `false` after a timeout.
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
        // top → A → B, both `about:srcdoc`: the real shape (shell site
        // guest → space chrome's nested site) minus the element runtime.
        let a = srcdoc_frame(&document()).await;
        let a_doc = a.content_document().expect("A is same-origin");
        let b = srcdoc_frame(&a_doc).await;
        let b_doc = b.content_document().expect("B is same-origin");
        let b_win = b.content_window().expect("B window");

        // The portal host lives in B, so B is the guest's creator.
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

        // The production mount path, exactly as `<tonk-portal>` calls it.
        let cell: RefCell<Option<Rc<RefCell<PortalState>>>> = RefCell::new(None);
        connect_portal(&host, &cell, None, Allow::none(), |_| {});
        let guest = host
            .query_selector("iframe")
            .expect("query")
            .expect("guest iframe mounted")
            .unchecked_into::<HtmlIFrameElement>();
        let guest_window: JsValue = guest.content_window().expect("guest window").into();

        // The bootstrap posts `hello` to its parent — B's window — as its
        // first act. No `hello` means the frame never loaded.
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
