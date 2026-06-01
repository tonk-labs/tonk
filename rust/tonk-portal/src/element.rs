//! The `<tonk-portal>` custom element.
//!
//! Attribute-driven and entirely synchronous — no host bridge, no
//! subscription, no generation guard, because the element never
//! fetches. It owns exactly one child `<iframe>` and mirrors two
//! observed attributes onto it:
//!
//! - `content` → `iframe.srcdoc` (reloads the iframe wholesale)
//! - `height`  → `iframe.style.height` in pixels (patched in place)
//!
//! The iframe is sandboxed `allow-scripts` with **no**
//! `allow-same-origin`: author scripts run, but in an opaque origin
//! that cannot touch the parent page, its session, or the worker.
//! `portal/content` is dialog data any writer can assert, so granting
//! it same-origin would be a credential-grade XSS path.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

/// The custom element. Holds its single child iframe so the attribute
/// callbacks can patch it after the initial mount. Every lifecycle
/// method takes `&mut self`, so no interior mutability is needed.
#[derive(Default)]
pub struct TonkPortal {
    iframe: Option<HtmlIFrameElement>,
}

impl CustomElement for TonkPortal {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["content", "height"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();

        // Build the iframe from the attributes that are already on the
        // host. The canonical view sets `content`/`height` before the
        // element connects, and during upgrade `attribute_changed_callback`
        // fires before this with no iframe yet (a no-op) — so reading the
        // live attributes here is what actually applies the first values.
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(iframe) = document.create_element("iframe") else {
            return;
        };
        let Ok(iframe) = iframe.dyn_into::<HtmlIFrameElement>() else {
            return;
        };

        // Opaque-origin sandbox. Scripts run; same-origin is withheld.
        let _ = iframe.set_attribute("sandbox", "allow-scripts");

        // `content` is authoritative; absent ⇒ empty document (the
        // reserved height keeps layout stable).
        let content = host.get_attribute("content").unwrap_or_default();
        let _ = iframe.set_attribute("srcdoc", &content);

        if let Some(height) = host.get_attribute("height") {
            set_height(&iframe, &height);
        }

        let _ = host.append_child(&iframe);
        self.iframe = Some(iframe);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop our reference and detach the iframe. The browser would
        // collect it with the host anyway, but removing it explicitly
        // keeps re-connects (view swaps) from leaving a stale frame.
        if let Some(iframe) = self.iframe.take()
            && let Some(parent) = iframe.parent_node()
        {
            let _ = parent.remove_child(&iframe);
        }
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        name: String,
        _old: Option<String>,
        new: Option<String>,
    ) {
        // Before `connected_callback` there is no iframe yet; the
        // initial values are read from the live attributes on connect,
        // so dropping pre-connect callbacks is correct.
        let Some(iframe) = self.iframe.as_ref() else {
            return;
        };
        match name.as_str() {
            // Reassigning `srcdoc` reloads the iframe wholesale,
            // discarding its DOM/JS state — the intended default.
            "content" => {
                let _ = iframe.set_attribute("srcdoc", &new.unwrap_or_default());
            }
            // Height patches in place; no reload.
            "height" => match new {
                Some(height) => set_height(iframe, &height),
                None => {
                    let _ = iframe.style().remove_property("height");
                }
            },
            _ => {}
        }
    }
}

/// Set the iframe's pixel height. The `height` attribute is an
/// `unsigned-integer` grid measure, so it carries a bare number; we
/// append the `px` unit.
fn set_height(iframe: &HtmlIFrameElement, height: &str) {
    let _ = iframe
        .style()
        .set_property("height", &format!("{height}px"));
}

/// Register `<tonk-portal>` with the page. Idempotent — calling more
/// than once is harmless. Unlike `<tonk-view>` there is no prototype
/// method to install: the element is entirely attribute-driven.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkPortal::define("tonk-portal");
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
    use wasm_bindgen::JsValue;
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
    fn it_mounts_one_sandboxed_iframe_on_connect() {
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
        assert_eq!(sandbox, "allow-scripts");
        assert!(
            !sandbox.contains("allow-same-origin"),
            "same-origin must never be granted by default; got: {sandbox}",
        );
    }

    #[dialog_common::test]
    fn it_writes_content_into_the_iframe_srcdoc() {
        let host = mount(Some("<canvas id=\"c\"></canvas>"), None);
        assert_eq!(
            iframe_of(&host).get_attribute("srcdoc").as_deref(),
            Some("<canvas id=\"c\"></canvas>"),
        );
    }

    #[dialog_common::test]
    fn it_renders_empty_srcdoc_when_content_is_absent() {
        let host = mount(None, Some("400"));
        // An iframe still mounts (the reserved height keeps layout
        // stable); its document is just empty.
        assert_eq!(
            iframe_of(&host).get_attribute("srcdoc").as_deref(),
            Some(""),
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
        assert_eq!(
            iframe_after.get_attribute("srcdoc").as_deref(),
            Some("<p>two</p>"),
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

    // --- Integration: driven by the canonical view through the
    //     real `<tonk-view>` renderer, exactly as `<tonk-display>`
    //     would in production. Pins "one iframe, both attributes
    //     applied, patched in place across a content change".

    /// Build the `{ this, fields }` conclusion JsValue the way
    /// `<tonk-display>` passes it into a view's `render`, with a string
    /// `content` and an integer `height` (the real concept types).
    fn portal_conclusion(this: &str, content: &str, height: i128) -> JsValue {
        use ipld_core::ipld::Ipld;
        use std::collections::BTreeMap;
        use tonk_schema::conclusion::Conclusion;

        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("content".to_owned(), Ipld::String(content.to_owned()));
        fields.insert("height".to_owned(), Ipld::Integer(height));
        let conclusion = Conclusion {
            this: this.to_owned(),
            fields,
        };
        serde_wasm_bindgen::to_value(&conclusion).expect("serialize conclusion")
    }

    /// Mount a `<tonk-view>` carrying the canonical portal template and
    /// drive it with one conclusion. Returns the view host.
    fn render_via_canonical_view(content: &str, height: i128) -> Element {
        use js_sys::{Function, Reflect};

        tonk_display::register();
        register();
        let document = document();
        let view = document.create_element("tonk-view").expect("create view");
        // Mirrors the `basic` portal view in bootstrap.yaml.
        view.set_inner_html("<tonk-portal html:content={content} html:height={height} />");
        document
            .body()
            .expect("body")
            .append_child(&view)
            .expect("attach view");

        let detail = portal_conclusion("did:key:zPortal", content, height);
        let render = Reflect::get(view.as_ref(), &"render".into()).expect("render method");
        let render_fn: Function = render.dyn_into().expect("render is a function");
        render_fn
            .call1(view.as_ref(), &detail)
            .expect("call render");
        view
    }

    #[dialog_common::test]
    fn it_drives_one_iframe_with_both_attributes_through_the_canonical_view() {
        let view = render_via_canonical_view("<p>painted</p>", 400);

        let iframes = view.query_selector_all("iframe").unwrap();
        assert_eq!(
            iframes.length(),
            1,
            "canonical view should yield exactly one iframe; got: {}",
            view.inner_html(),
        );
        let iframe = view
            .query_selector("iframe")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlIFrameElement>()
            .unwrap();
        assert_eq!(
            iframe.get_attribute("srcdoc").as_deref(),
            Some("<p>painted</p>"),
            "content should reach srcdoc through the html:-forced binding",
        );
        assert_eq!(
            iframe.style().get_property_value("height").ok().as_deref(),
            Some("400px"),
            "integer height should reach the iframe as a pixel style, not a JS property",
        );
    }

    #[dialog_common::test]
    fn it_patches_the_canonical_iframe_in_place_across_a_content_change() {
        use js_sys::{Function, Reflect};

        let view = render_via_canonical_view("<p>before</p>", 400);
        let iframe_before = view
            .query_selector("iframe")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlIFrameElement>()
            .unwrap();

        // Re-render with new content; height unchanged.
        let detail = portal_conclusion("did:key:zPortal", "<p>after</p>", 400);
        let render = Reflect::get(view.as_ref(), &"render".into()).expect("render method");
        let render_fn: Function = render.dyn_into().expect("render is a function");
        render_fn.call1(view.as_ref(), &detail).expect("re-render");

        assert_eq!(
            view.query_selector_all("iframe").unwrap().length(),
            1,
            "still exactly one iframe after a content change",
        );
        let iframe_after = view
            .query_selector("iframe")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlIFrameElement>()
            .unwrap();
        assert_eq!(
            iframe_after.get_attribute("srcdoc").as_deref(),
            Some("<p>after</p>"),
        );
        assert!(
            iframe_before.is_same_node(Some(iframe_after.unchecked_ref())),
            "the renderer patches the portal's attributes in place; the iframe survives",
        );
    }
}
