//! `<font-family>` — a font face a view carries, declared in markup.
//!
//! A stylesheet cannot carry bytes, so `@font-face` points at a file
//! with `src:`. We can carry bytes, so the same declaration is written
//! as an element and the face is built directly:
//!
//! ```html
//! <font-family name="Body" with:src=body></font-family>
//! ```
//!
//! Which is `@font-face { font-family: Body; src: … }` with the same
//! two parts and the same words — the family, and where its bytes come
//! from. A stylesheet then names the family and never needs a `url()`,
//! which is what keeps it working at the opaque origin of a sealed
//! guest, where a relative URL resolves against nothing.
//!
//! The element does not fetch anything itself. Its `with:src` is
//! resolved with every other embed a view declares — at lowering, into
//! the view's `embeds` artifact, and then against the inert template
//! before any row is cloned from it (see [`crate::embed`]). So by the
//! time an instance connects, its face is already registered on the
//! document. What is left for the element is to exist, so the tag
//! upgrades rather than sitting unknown, and to render nothing.
//!
//! `name` is optional: without it the family is the embed's own name,
//! so `<font-family with:src=body>` registers `body`. Naming it is for
//! when a stylesheet wants a different spelling than the key.

#[cfg(target_arch = "wasm32")]
use custom_elements::CustomElement;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlElement;

/// The custom element. Stateless: the registration happens in the
/// embed pass, and an instance carries nothing of its own.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct FontFamily;

#[cfg(target_arch = "wasm32")]
impl CustomElement for FontFamily {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // A declaration, not a box: it occupies no space wherever an
        // author puts it. Set on the element rather than in a
        // stylesheet so it holds in a guest whose CSS has not arrived
        // yet — an unstyled beat must not push the page around.
        let _ = this.style().set_property("display", "none");
    }
}

/// Register the `<font-family>` custom element. Idempotent.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    if already_registered() {
        return;
    }
    FontFamily::define("font-family");
}

#[cfg(target_arch = "wasm32")]
fn already_registered() -> bool {
    let Some(win) = web_sys::window() else {
        return false;
    };
    !win.custom_elements().get("font-family").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    /// The tag upgrades and takes no space.
    ///
    /// `font-face` would read better, but it is on the custom-element
    /// reserved list (with `color-profile`, `missing-glyph` and the
    /// other SVG/MathML legacy names), so defining it throws. This is
    /// the nearest legal name, and it is arguably the better one: it
    /// names what the element produces, which is what a stylesheet
    /// then asks for by `font-family:`.
    #[dialog_common::test]
    fn it_upgrades_and_renders_nothing() {
        register();
        let document = web_sys::window().unwrap().document().unwrap();
        let element = document.create_element("font-family").unwrap();
        element.set_attribute("name", "Body").unwrap();
        document.body().unwrap().append_child(&element).unwrap();

        let html: HtmlElement = element.clone().dyn_into().unwrap();
        assert_eq!(
            html.style()
                .get_property_value("display")
                .unwrap_or_default(),
            "none",
            "a declaration occupies no space",
        );
        assert_eq!(html.offset_height(), 0, "and contributes no layout",);
        element.remove();
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
use wasm_bindgen::JsCast as _;
