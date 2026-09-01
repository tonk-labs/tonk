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
use tonk_host::location::{Allow, Location};
use web_sys::{Element, HtmlElement, window};

use crate::bridge::{self, PortalState};
use crate::shared::{connect_portal, install_method_shims, reload_portal};

/// Parse an optional `with` attribute off a portal element. A malformed
/// value is logged and treated as absent rather than failing the mount —
/// the portal then runs sealed on the ambient context.
pub(crate) fn portal_with(this: &HtmlElement) -> Option<Location> {
    let value = this
        .get_attribute("with")
        .filter(|v| !v.is_empty() && !v.contains('{'))?;
    match value.parse() {
        Ok(location) => Some(location),
        Err(error) => {
            tonk_common::log!("tonk-portal: malformed with={value:?}: {error}");
            None
        }
    }
}

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
        // An optional `with` pins the portal's context; a malformed value is
        // logged and treated as absent (the portal stays on the ambient
        // context). The allow list is exactly the pinned context (or
        // nothing): a generic content portal renders synced/untrusted
        // markup, so it must NOT be able to escape its pinned context — a
        // guest-forwarded off-context route is denied.
        let with = portal_with(this);
        let allow = with.clone().map(Allow::only).unwrap_or_else(Allow::none);
        connect_portal(this, &self.inner, with, allow, |iframe| {
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
                // Two-phase: unload the guest realm first, remove the
                // element a tick later — synchronous destruction of a live
                // guest is the pattern the browser process crashes under
                // (see `site::teardown`).
                //
                // Unload via the frame's own `location.replace()`, never by
                // setting `src`: setting `src` NAVIGATES the frame, and a
                // frame navigation appends an entry to the joint session
                // history, so each teardown left an extra Back step behind.
                let _ = iframe.remove_attribute("srcdoc");
                if let Some(frame_window) = iframe.content_window() {
                    let _ = frame_window.location().replace("about:blank");
                }
                wasm_bindgen_futures::spawn_local(async move {
                    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                        if let Some(win) = web_sys::window() {
                            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                                &resolve, 100,
                            );
                        }
                    });
                    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                    if let Some(parent) = iframe.parent_node() {
                        let _ = parent.remove_child(&iframe);
                    }
                });
            }
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // `attributeChangedCallback` fires on every setAttribute, same
        // value included — and the host re-sets these on re-renders
        // that changed nothing. A reload here is a full guest reboot:
        // live subscriptions cancelled, DOM rebuilt, and a boot that
        // races a busy worker sits on its loader. Only an actual change
        // may cost that.
        if old == new {
            return;
        }
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
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Document, HtmlIFrameElement};

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
        // capture-phase guard cancels the native navigation); `allow-downloads`
        // lets a guest trigger a save (e.g. the blob file card's Download).
        assert_eq!(sandbox, "allow-scripts allow-forms allow-downloads");
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

    /// Teardown is TWO-PHASE: on disconnect the guest realm is unloaded
    /// immediately (comms already severed), and the element itself is removed
    /// a tick later — synchronous destruction of a live guest is the pattern
    /// the browser process crashed under.
    ///
    /// Phase one asserts the guest CONTENT is gone, not the mechanism that
    /// removed it. The unload goes through the frame's own
    /// `location.replace()` rather than `iframe.src = "about:blank"`, because
    /// setting `src` navigates the frame and a frame navigation appends an
    /// entry to the joint session history (Back then needed an extra press per
    /// teardown). `location.replace()` leaves the `src` attribute untouched,
    /// so asserting on it would only pin the old mechanism in place.
    #[dialog_common::test]
    async fn it_removes_the_iframe_on_disconnect() {
        let host = mount(Some("<p>hi</p>"));
        assert_eq!(host.query_selector_all("iframe").unwrap().length(), 1);

        host.remove();

        // Phase one, immediate: the frame is unloading, not yet detached — and
        // it must NOT have been navigated via its `src` attribute, which is
        // what used to grow the history.
        if let Some(iframe) = host.query_selector("iframe").unwrap() {
            assert_eq!(
                iframe.get_attribute("srcdoc"),
                None,
                "disconnect must unload the guest realm first",
            );
            assert_eq!(
                iframe.get_attribute("src"),
                None,
                "the unload must not navigate the frame via `src` — that appends \
                 a joint-session-history entry",
            );
        }

        // Phase two: detached shortly after.
        for _ in 0..80 {
            if host.query_selector("iframe").unwrap().is_none() {
                break;
            }
            let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                if let Some(win) = web_sys::window() {
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 25);
                }
            });
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        }
        assert!(
            host.query_selector("iframe").unwrap().is_none(),
            "the deferred phase detaches the iframe; got: {}",
            host.inner_html(),
        );
    }
}
