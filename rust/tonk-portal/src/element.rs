//! The `<tonk-portal>` custom element.
//!
//! A portal owns one child `<iframe>` and is two things at once:
//!
//! - a **painter** — it mirrors the `content` attribute into the
//!   iframe's `srcdoc`; and
//! - a **transport** — it injects a small `tonk` object into the
//!   iframe (see [`crate::bridge`]) through which author code reads and
//!   writes live data, relaying the iframe's calls onto the existing
//!   `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer events.
//!
//! The iframe is sandboxed `allow-scripts` — an opaque origin. It
//! cannot reach `parent.document`; it talks to the parent only over a
//! `MessagePort` opened by the bridge bootstrap (see [`crate::bridge`]).
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
        &["content", "entity", "model", "runtime"]
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

        // Opaque-origin sandbox: scripts run but `parent.document` is
        // unreachable. The bridge bootstrap reaches the parent only over
        // a `MessagePort` it opens and transfers in its `hello`.
        let _ = iframe.set_attribute("sandbox", "allow-scripts");

        // The iframe always fills its container. `flex: 1` + `align-self:
        // stretch` make it fill a flex-column host (the display-route layout)
        // without needing a definite-height ancestor for `height: 100%`.
        let style = iframe.style();
        let _ = style.set_property("width", "100%");
        let _ = style.set_property("height", "100%");
        let _ = style.set_property("flex", "1 1 auto");
        let _ = style.set_property("align-self", "stretch");
        let _ = style.set_property("border", "0");

        let state = Rc::new(RefCell::new(PortalState::new()));
        bridge::register_portal(&iframe, &host, &state);
        install_method_delegates(&host, &state);

        // Append before assigning `srcdoc` so `contentWindow` exists;
        // the `hello` listener matches the live `contentWindow`, so the
        // bootstrap script resolves this portal when it posts `hello`.
        let content = host.get_attribute("content").unwrap_or_default();
        // In `runtime` mode the guest renders OUR elements (a real
        // `<tonk-display>`): the bootstrap additionally pulls in the
        // injected element runtime + CSS before `content` upgrades.
        let runtime = host.has_attribute("runtime");
        let _ = host.append_child(&iframe);
        let srcdoc = if runtime {
            bridge::bootstrap_srcdoc_with_runtime(&content)
        } else {
            bridge::bootstrap_srcdoc(&content)
        };
        let _ = iframe.set_attribute("srcdoc", &srcdoc);

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
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        // Pre-connect callbacks (during upgrade) have no state yet; the
        // initial values are read live in `connected_callback`.
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        match name.as_str() {
            // New content reloads the iframe wholesale.
            "content" => reload(&host, &state),
            // A re-scope reloads the iframe so the bootstrap re-runs
            // author code; the fresh `context` rides the new handshake.
            "entity" | "model" => reload(&host, &state),
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
        let srcdoc = if host.has_attribute("runtime") {
            bridge::bootstrap_srcdoc_with_runtime(&content)
        } else {
            bridge::bootstrap_srcdoc(&content)
        };
        let _ = iframe.set_attribute("srcdoc", &srcdoc);
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

/// Register `<tonk-portal>` with the page. Idempotent. Installs the
/// page-level `hello` message listener, defines the element, and
/// installs the `reset` / `error` prototype shims that route
/// subscription frames into the per-instance delegates.
pub fn register() {
    bridge::install_message_listener();
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

    /// Mount a `<tonk-portal>` with the given content and attach it to
    /// the body. `register()` runs first so the element upgrades on
    /// connect.
    fn mount(content: Option<&str>) -> Element {
        register();
        let document = document();
        let host = document
            .create_element("tonk-portal")
            .expect("create tonk-portal");
        if let Some(content) = content {
            host.set_attribute("content", content).expect("set content");
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
    fn it_mounts_one_opaque_origin_sandboxed_iframe_on_connect() {
        let host = mount(Some("<p>hi</p>"));
        assert_eq!(
            host.query_selector_all("iframe").unwrap().length(),
            1,
            "exactly one iframe should mount; got: {}",
            host.inner_html(),
        );
        let sandbox = iframe_of(&host)
            .get_attribute("sandbox")
            .expect("sandbox attribute present");
        // No `allow-same-origin`: the iframe is an opaque origin and
        // reaches the parent only over the bridge's `MessagePort`.
        assert_eq!(sandbox, "allow-scripts");
    }

    #[dialog_common::test]
    fn it_prepends_the_bridge_bootstrap_and_keeps_the_content() {
        let host = mount(Some("<canvas id=\"c\"></canvas>"));
        let srcdoc = iframe_of(&host)
            .get_attribute("srcdoc")
            .expect("srcdoc present");
        assert!(
            srcdoc.contains("MessageChannel") && srcdoc.contains("window.tonk"),
            "srcdoc should carry the bridge bootstrap; got: {srcdoc}",
        );
        assert!(
            srcdoc.contains("<canvas id=\"c\"></canvas>"),
            "srcdoc should still carry the author content; got: {srcdoc}",
        );
    }

    #[dialog_common::test]
    fn it_fills_its_container_height() {
        let host = mount(Some("<p>hi</p>"));
        let style = iframe_of(&host)
            .style()
            .get_property_value("height")
            .expect("height property");
        assert_eq!(style, "100%");
    }

    #[dialog_common::test]
    fn it_reloads_srcdoc_when_content_changes_keeping_the_same_iframe() {
        let host = mount(Some("<p>one</p>"));
        let iframe_before = iframe_of(&host);

        host.set_attribute("content", "<p>two</p>")
            .expect("update content");

        let iframe_after = iframe_of(&host);
        let srcdoc = iframe_after.get_attribute("srcdoc").expect("srcdoc");
        assert!(srcdoc.contains("<p>two</p>"), "new content; got: {srcdoc}");
        assert!(
            srcdoc.contains("MessageChannel"),
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
    fn it_removes_the_iframe_on_disconnect() {
        let host = mount(Some("<p>hi</p>"));
        assert_eq!(host.query_selector_all("iframe").unwrap().length(), 1);

        host.remove();

        assert!(
            host.query_selector("iframe").unwrap().is_none(),
            "disconnect should detach the iframe; got: {}",
            host.inner_html(),
        );
    }
}
