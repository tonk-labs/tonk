//! The `<tonk-portal>` custom element.
//!
//! A portal owns one child `<iframe>` and is two things at once:
//!
//! - a **painter** — it mirrors the `content` attribute into the
//!   iframe's `srcdoc` and applies an optional pixel `height`; and
//! - a **transport** — it injects a small `tonk` object into the
//!   iframe (see [`crate::bridge`]) through which author code reads and
//!   writes live data, relaying the iframe's calls onto the existing
//!   `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer events.
//!
//! The iframe is sandboxed `allow-scripts allow-same-origin`. The
//! same-origin grant is what lets the bridge inject `tonk` by a direct
//! `parent.__tonkConnect` call with no MessageChannel. Author code must
//! reach data only through `tonk`, never through `parent.document`.
//!
//! State lives in [`crate::bridge::PortalState`] behind `Rc<RefCell<…>>`
//! so the lifecycle callbacks, the prototype `reset` / `error` delegates,
//! and the bridge closures all share it.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};

/// The custom element. Holds the shared [`PortalState`]; `None` until
/// `connected_callback` builds it.
#[derive(Default)]
pub struct TonkPortal {
    inner: RefCell<Option<Rc<RefCell<PortalState>>>>,
}

impl CustomElement for TonkPortal {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["content", "height", "entity", "model"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
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

        // Same-origin sandbox: scripts run and `window.parent` is
        // reachable, so the bridge can inject `tonk` synchronously.
        let _ = iframe.set_attribute("sandbox", "allow-scripts allow-same-origin");

        // By default the iframe fills its container; `height` pins a
        // fixed pixel height instead.
        let style = iframe.style();
        let _ = style.set_property("width", "100%");
        let _ = style.set_property("height", "100%");
        let _ = style.set_property("border", "0");
        if let Some(height) = host.get_attribute("height") {
            set_height(&iframe, &height);
        }

        let state = Rc::new(RefCell::new(PortalState::new()));
        let bridge = bridge::build_bridge(&host, &state);
        bridge::register_portal(&iframe, &bridge);
        install_method_delegates(&host, &state);

        // Append before assigning `srcdoc` so `contentWindow` exists;
        // `__tonkConnect` matches on the live `contentWindow`, so the
        // bootstrap script resolves this portal's bridge when it runs.
        let content = host.get_attribute("content").unwrap_or_default();
        let _ = host.append_child(&iframe);
        let _ = iframe.set_attribute("srcdoc", &bridge::bootstrap_srcdoc(&content));

        state.borrow_mut().iframe = Some(iframe);
        *self.inner.borrow_mut() = Some(state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take() {
            let mut s = state.borrow_mut();
            s.disposed = true;
            s.clear_subs();
            if let Some(iframe) = s.iframe.take() {
                bridge::unregister_portal(&iframe);
                if let Some(parent) = iframe.parent_node() {
                    let _ = parent.remove_child(&iframe);
                }
            }
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        _old: Option<String>,
        new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        // Pre-connect callbacks (during upgrade) have no state yet; the
        // initial values are read live in `connected_callback`.
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        match name.as_str() {
            // Height patches in place; no reload, no subscription churn.
            "height" => {
                if let Some(iframe) = state.borrow().iframe.as_ref() {
                    match new {
                        Some(height) => set_height(iframe, &height),
                        // Back to filling the container.
                        None => {
                            let _ = iframe.style().set_property("height", "100%");
                        }
                    }
                }
            }
            // New content reloads the iframe wholesale.
            "content" => reload(&host, &state),
            // A re-scope updates the author-facing context, then reloads
            // so the bootstrap re-runs author code against it.
            "entity" | "model" => {
                bridge::rescope(&host, &state);
                reload(&host, &state);
            }
            _ => {}
        }
    }
}

/// Cancel the portal's live subscriptions and reload the iframe from
/// the current `content`. Reassigning `srcdoc` discards the old
/// window — and with it any author `for await` loops — so we cancel
/// the host subscriptions they fed first.
fn reload(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let mut s = state.borrow_mut();
    s.clear_subs();
    if let Some(iframe) = s.iframe.as_ref() {
        let content = host.get_attribute("content").unwrap_or_default();
        let _ = iframe.set_attribute("srcdoc", &bridge::bootstrap_srcdoc(&content));
    }
}

/// Write the per-instance `__tonkReset` / `__tonkError` closures the
/// prototype shims forward subscription frames to. Mirrors
/// `<tonk-display>`'s method-delegate pattern.
fn install_method_delegates(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let reset_state = state.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_reset(&reset_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let error_state = state.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_error(&error_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}

/// Set the iframe's pixel height. The `height` attribute is an
/// `unsigned-integer` grid measure, so it carries a bare number; we
/// append the `px` unit.
fn set_height(iframe: &HtmlIFrameElement, height: &str) {
    let _ = iframe
        .style()
        .set_property("height", &format!("{height}px"));
}

/// Register `<tonk-portal>` with the page. Idempotent. Installs the
/// page-level `__tonkConnect` function, defines the element, and
/// installs the `reset` / `error` prototype shims that route
/// subscription frames into the per-instance delegates.
pub fn register() {
    bridge::install_connect();
    if already_registered() {
        return;
    }
    TonkPortal::define("tonk-portal");
    install_method_shims();
}

/// Install `reset` / `update` / `error` on the `<tonk-portal>`
/// prototype, each forwarding to the per-instance `__tonk*` closure.
/// On the prototype (not each instance) so `this`-binding is correct
/// when the host invokes `consumer.reset(payload, opts)`.
fn install_method_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-portal");
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

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-portal").is_undefined()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::Document;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Mount a `<tonk-portal>` with the given attributes and attach it
    /// to the body. `register()` runs first so the element upgrades on
    /// connect.
    fn mount(content: Option<&str>, height: Option<&str>) -> Element {
        register();
        let document = document();
        let host = document
            .create_element("tonk-portal")
            .expect("create tonk-portal");
        if let Some(content) = content {
            host.set_attribute("content", content).expect("set content");
        }
        if let Some(height) = height {
            host.set_attribute("height", height).expect("set height");
        }
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    /// The single child iframe a connected portal owns.
    fn iframe_of(host: &Element) -> HtmlIFrameElement {
        host.query_selector("iframe")
            .expect("query_selector")
            .expect("iframe mounted")
            .dyn_into::<HtmlIFrameElement>()
            .expect("HtmlIFrameElement")
    }

    #[dialog_common::test]
    fn it_mounts_one_same_origin_sandboxed_iframe_on_connect() {
        let host = mount(Some("<p>hi</p>"), None);
        assert_eq!(
            host.query_selector_all("iframe").unwrap().length(),
            1,
            "exactly one iframe should mount; got: {}",
            host.inner_html(),
        );
        let sandbox = iframe_of(&host)
            .get_attribute("sandbox")
            .expect("sandbox attribute present");
        // The bridge slice runs same-origin to inject `tonk` cheaply;
        // origin isolation returns with the postMessage transport.
        assert_eq!(sandbox, "allow-scripts allow-same-origin");
    }

    #[dialog_common::test]
    fn it_prepends_the_bridge_bootstrap_and_keeps_the_content() {
        let host = mount(Some("<canvas id=\"c\"></canvas>"), None);
        let srcdoc = iframe_of(&host)
            .get_attribute("srcdoc")
            .expect("srcdoc present");
        assert!(
            srcdoc.contains("__tonkConnect"),
            "srcdoc should carry the bridge bootstrap; got: {srcdoc}",
        );
        assert!(
            srcdoc.contains("<canvas id=\"c\"></canvas>"),
            "srcdoc should still carry the author content; got: {srcdoc}",
        );
    }

    #[dialog_common::test]
    fn it_sets_the_iframe_height_in_pixels_from_the_attribute() {
        let host = mount(Some("<p>hi</p>"), Some("400"));
        let style = iframe_of(&host)
            .style()
            .get_property_value("height")
            .expect("height property");
        assert_eq!(style, "400px");
    }

    #[dialog_common::test]
    fn it_reloads_srcdoc_when_content_changes_keeping_the_same_iframe() {
        let host = mount(Some("<p>one</p>"), None);
        let iframe_before = iframe_of(&host);

        host.set_attribute("content", "<p>two</p>")
            .expect("update content");

        let iframe_after = iframe_of(&host);
        let srcdoc = iframe_after.get_attribute("srcdoc").expect("srcdoc");
        assert!(srcdoc.contains("<p>two</p>"), "new content; got: {srcdoc}");
        assert!(
            srcdoc.contains("__tonkConnect"),
            "bootstrap survives reload"
        );
        // A content change reassigns srcdoc on the *same* iframe — the
        // element is not torn down and rebuilt.
        assert!(
            iframe_before.is_same_node(Some(iframe_after.unchecked_ref())),
            "content change should reuse the iframe, not replace it",
        );
    }

    #[dialog_common::test]
    fn it_updates_height_in_place_without_reloading_the_iframe() {
        let host = mount(Some("<p>hi</p>"), Some("400"));
        let iframe_before = iframe_of(&host);

        host.set_attribute("height", "600").expect("update height");

        let iframe_after = iframe_of(&host);
        assert_eq!(
            iframe_after
                .style()
                .get_property_value("height")
                .as_deref()
                .ok(),
            Some("600px"),
        );
        assert!(
            iframe_before.is_same_node(Some(iframe_after.unchecked_ref())),
            "height change must patch in place, never reload the iframe",
        );
    }

    #[dialog_common::test]
    fn it_removes_the_iframe_on_disconnect() {
        let host = mount(Some("<p>hi</p>"), None);
        assert_eq!(host.query_selector_all("iframe").unwrap().length(), 1);

        host.remove();

        assert!(
            host.query_selector("iframe").unwrap().is_none(),
            "disconnect should detach the iframe; got: {}",
            host.inner_html(),
        );
    }
}
