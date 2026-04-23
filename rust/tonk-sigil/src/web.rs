use crate::Sigil;
use custom_elements::CustomElement;
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::{Element, HtmlElement};

#[derive(Default)]
struct SigilElement;

impl CustomElement for SigilElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["value", "fill", "sprite"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        render(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        render(this);
    }
}

/// Renders the sigil into the element. The SVG is placed at the end
/// as a dedicated child (`<svg data-sigil>`) and replaced in place on
/// subsequent renders. Other children (e.g. the original text node
/// that seeds the sigil) are left untouched — crucial, because the
/// text content is also what we hash to produce the sigil.
fn render(this: &HtmlElement) {
    let bits = resolve_bits(this);

    let mut sigil = Sigil::from(bits);
    if let Some(fill) = this.get_attribute("fill") {
        sigil = sigil.fill(fill);
    }
    if let Some(sprite) = this.get_attribute("sprite") {
        sigil = sigil.sprite_href(sprite);
    }

    let rendered = sigil.render();
    let document = match this.owner_document() {
        Some(d) => d,
        None => return,
    };

    // Find or create the wrapper that holds the rendered SVG.
    // Matches the selector used on insertion so re-renders update
    // the same wrapper instead of appending another one.
    let existing = this
        .query_selector(":scope > span[data-sigil]")
        .ok()
        .flatten();

    let wrapper: Element = match existing {
        Some(el) => el,
        None => {
            let el = document.create_element("span").unwrap_throw();
            el.set_attribute("data-sigil", "").unwrap_throw();
            // `<span>` defaults to `display: inline`, which makes its
            // content-sized box collapse around the SVG rather than
            // filling the host element. Force it to be a block box
            // that fills the host so the inner SVG's `width: 100%`
            // and `height: 100%` resolve against the full host size.
            el.set_attribute(
                "style",
                "display:block;width:100%;height:100%",
            )
            .unwrap_throw();
            // Insert the wrapper *before* any existing children so CSS
            // layout ordering (sigil first, text after) works without
            // depending on DOM insertion order.
            this.insert_before(&el, this.first_child().as_ref())
                .unwrap_throw();
            el
        }
    };

    // `set_inner_html` on just this wrapper replaces only its
    // contents — the element's other children (the original text
    // node) are untouched, so re-reading `textContent` still yields
    // the seed on subsequent renders.
    wrapper.set_inner_html(&rendered);
}

/// Determines the 4 bytes driving the sigil:
///   1. If `value` attribute parses as a u32 (decimal, or `0x`-prefixed hex),
///      use it directly.
///   2. Else hash the element's text content with blake3 and take the first
///      4 bytes. The wrapper's own SVG is read-back as empty text, so this
///      stays correct across re-renders.
///   3. Fallback: zero.
fn resolve_bits(this: &HtmlElement) -> [u8; 4] {
    if let Some(attr) = this.get_attribute("value")
        && let Some(n) = parse_u32(&attr)
    {
        return n.to_be_bytes();
    }

    let text = collect_text(this);

    if text.is_empty() {
        return [0; 4];
    }

    let hash = blake3::hash(text.as_bytes());
    let bytes = hash.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// Gather text from the element's direct non-wrapper children. We
/// cannot use `textContent` because that includes the SVG wrapper's
/// inner text (empty for our sprite refs, but still a pollution
/// concern). Iterate children once and concatenate raw text nodes.
fn collect_text(this: &HtmlElement) -> String {
    let mut out = String::new();
    let children = this.child_nodes();
    let len = children.length();
    for i in 0..len {
        let Some(node) = children.get(i) else {
            continue;
        };
        // Node.TEXT_NODE = 3
        if node.node_type() == 3
            && let Some(text) = node.text_content()
        {
            out.push_str(&text);
        } else if let Some(element) = node.dyn_ref::<Element>()
            && element.tag_name().eq_ignore_ascii_case("span")
            && element.has_attribute("data-sigil")
        {
            // Skip our own wrapper; include other children's text so
            // markup like `<tonk-sigil><strong>foo</strong></tonk-sigil>`
            // still hashes `foo`.
        } else if let Some(text) = node.text_content() {
            out.push_str(&text);
        }
    }
    out.trim().to_string()
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

impl Sigil {
    /// Registers the `<tonk-sigil>` custom element with the browser. Call once
    /// at app startup. Subsequent calls are no-ops if the element is already
    /// defined.
    pub fn install() {
        if already_registered() {
            return;
        }
        inject_baseline_style();
        SigilElement::define("tonk-sigil");
    }
}

/// Injects a one-time `<style>` into `<head>` to give the custom
/// element sensible defaults. Custom elements are inline-level by
/// default in the browser's UA stylesheet; `inline-block` lets
/// callers size the host with CSS and lets slot containers (like
/// `wa-card`'s media slot) size us with `inline-size: 100%;
/// block-size: 100%` via `::slotted()`. The inner `[data-sigil]`
/// wrapper is forced to fill the host so the SVG's `100%` sizing
/// resolves against the full host box rather than collapsing.
///
/// Deliberately *not* setting `width:100%; height:100%` on the
/// host — that would over-size the element in containers that
/// have no explicit size (e.g. a narrow sidebar), stretching the
/// square sigil into a ribbon. Each consumer sizes the element.
fn inject_baseline_style() {
    const ID: &str = "tonk-sigil-baseline";
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    if document.get_element_by_id(ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else { return };
    let _ = style.set_attribute("id", ID);
    style.set_text_content(Some(
        "tonk-sigil{display:inline-block;line-height:0}\
         tonk-sigil>[data-sigil]{display:block;width:100%;height:100%}",
    ));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
    }
}

fn already_registered() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    !window.custom_elements().get("tonk-sigil").is_undefined()
}
