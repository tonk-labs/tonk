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
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};
use crate::shared::{connect_portal, install_method_shims, reload_portal};

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
        // `false`: a generic content portal renders synced/untrusted markup,
        // so it must NOT be able to escape its handshake repo context. A
        // guest-forwarded route is ignored for this portal.
        connect_portal(this, &self.inner, false, |iframe| {
            // The iframe always fills its container. `flex: 1` + `align-self:
            // stretch` make it fill a flex-column host (the display-route layout)
            // without needing a definite-height ancestor for `height: 100%`.
            let style = iframe.style();
            let _ = style.set_property("width", "100%");
            let _ = style.set_property("height", "100%");
            let _ = style.set_property("flex", "1 1 auto");
            let _ = style.set_property("align-self", "stretch");
            let _ = style.set_property("border", "0");
        });
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
            "content" => reload_portal(&host, &state),
            // A re-scope reloads the iframe so the bootstrap re-runs
            // author code; the fresh `context` rides the new handshake.
            "entity" | "model" => reload_portal(&host, &state),
            _ => {}
        }
    }
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
    install_method_shims("tonk-portal");
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
        // `allow-forms` lets a guest `<form>` fire its `submit` event (a
        // capture-phase guard cancels the native navigation).
        assert_eq!(sandbox, "allow-scripts allow-forms");
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
