//! `<tonk-origin>` — supplies the page origin to declarative views.
//!
//! On connect it distributes the invite-URL base (`{origin}/join`) to
//! descendants that ask for it via a `bind:base=<target-attr>`
//! attribute, so a template can assemble an absolute invite link
//! without reading `window` itself:
//!
//! ```html
//! <tonk-origin>
//!   <tonk-display bind:base="data-base" model=invitation entity={subject}>
//!     <wa-input readonly value="{dom.host/data-base}?access={access}#{code}">
//!     </wa-input>
//!   </tonk-display>
//! </tonk-origin>
//! ```
//!
//! The element is deliberately ignorant of `<tonk-display>`, invites,
//! and URLs — it only reads `window.location.origin` and fills whatever
//! `bind:base` targets its descendants declare. The descendant markup,
//! not the element, owns the wiring. The keypair and delegation are
//! minted entirely by the worker's `tonk:invite` handler; the browser
//! supplies only the origin.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, window};

/// The `bind:` attribute prefix a descendant uses to request a value.
const BIND_PREFIX: &str = "bind:";

/// Per-element state. The element holds nothing across renders — the
/// origin is read on connect and pushed into the DOM — so it is empty.
#[derive(Default)]
pub(crate) struct TonkOrigin;

impl CustomElement for TonkOrigin {
    fn shadow() -> bool {
        // Light DOM: the element must see its descendants to read their
        // `bind:` declarations and write their target attributes.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        distribute(this, &join_base());
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}
}

/// The invite URL base — the recipient's `/join` page on this origin.
/// Provided as the `base` bind value so a template can assemble an
/// absolute invite URL (`{base}?access=…#{code}`) without reading
/// `window` itself. Empty when there is no origin.
///
/// Reads the origin from the bridge context (`window.tonk.context.origin`),
/// not `window.location`: in a sealed guest the latter is `"null"` (opaque
/// origin), so the host supplies its real origin over the bridge.
fn join_base() -> String {
    tonk_host::bridge::context_origin()
        .map(|origin| format!("{origin}/join"))
        .unwrap_or_default()
}

/// Walk the element's descendants and, for each `bind:base=<target>`
/// attribute, write the base into the named target attribute.
fn distribute(host: &HtmlElement, base: &str) {
    for element in descendants(host) {
        for (name, target) in bindings(&element) {
            if name == "base" {
                let _ = element.set_attribute(&target, base);
            }
        }
    }
}

/// Every descendant element of `host`, in document order.
fn descendants(host: &HtmlElement) -> Vec<Element> {
    let mut out = Vec::new();
    if let Ok(nodes) = host.query_selector_all("*") {
        for i in 0..nodes.length() {
            if let Some(element) = nodes.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                out.push(element);
            }
        }
    }
    out
}

/// Read an element's `bind:<name>=<target>` attributes as
/// `(name, target)` pairs. A `querySelector` for `[bind:…]` would need
/// the colon escaped, so we iterate the attribute list instead.
fn bindings(element: &Element) -> Vec<(String, String)> {
    let attrs = element.attributes();
    let mut out = Vec::new();
    for i in 0..attrs.length() {
        let Some(attr) = attrs.item(i) else { continue };
        let name = attr.name();
        if let Some(value_name) = name.strip_prefix(BIND_PREFIX) {
            // `bind:base=data-base` → value `base` into target attribute
            // `data-base`.
            out.push((value_name.to_owned(), attr.value()));
        }
    }
    out
}

/// Register `<tonk-origin>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkOrigin::define("tonk-origin");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-origin").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A connected `<tonk-origin>` fills a child `bind:base=data-base`
    /// element with `{origin}/join`.
    #[dialog_common::test]
    async fn it_binds_the_base_to_declared_targets() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let origin = document.create_element("tonk-origin").unwrap();
        let sink = document.create_element("div").unwrap();
        sink.set_attribute("bind:base", "data-base").unwrap();
        origin.append_child(&sink).unwrap();
        body.append_child(&origin).unwrap();

        let base = sink.get_attribute("data-base").unwrap_or_default();
        assert!(
            base.ends_with("/join"),
            "bind:base should fill data-base with {{origin}}/join, got {base:?}",
        );

        origin.remove();
    }

    /// Bare descendants with no `bind:` attribute are left untouched.
    #[dialog_common::test]
    async fn it_ignores_descendants_without_a_bind_attribute() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let origin = document.create_element("tonk-origin").unwrap();
        let plain = document.create_element("span").unwrap();
        origin.append_child(&plain).unwrap();
        body.append_child(&origin).unwrap();

        assert!(
            !plain.has_attribute("data-base"),
            "a descendant with no bind: attribute must not be modified",
        );

        origin.remove();
    }
}
