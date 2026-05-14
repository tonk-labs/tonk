//! `<tonk-inspector>` — an Observable-style value-renderer.
//!
//! Authoring shape:
//!
//! ```html
//! <tonk-inspector></tonk-inspector>
//! ```
//!
//! Owners drive it by calling `el.render(value)` with any JS value.
//! The element rebuilds its DOM to show the value:
//!
//! - Strings render quoted: `"hello"`
//! - Numbers and booleans render bare: `42`, `true`
//! - `null` and `undefined` render italicized
//! - Arrays render `Array(n) [ … ]`, expanded one level by default
//! - Objects render `Object { k: v, k: v, … }`, same depth rule
//! - Deeper nesting collapses behind a `<details>`/`<summary>`
//!   disclosure
//!
//! v1 skips `Date`, `Map`, `Set`, `Symbol`, `BigInt` — they render
//! via their JS `String(value)` representation.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlElement, window};

/// How deep to render objects/arrays expanded before collapsing
/// the rest under `<details>`. Tuned to match Observable's feel —
/// the top-level structure is visible at a glance, but you don't
/// drown in nested data.
const EXPAND_DEPTH: u32 = 1;

struct Inner {
    host: Element,
}

#[derive(Default)]
pub struct TonkInspector {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkInspector {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let state = Rc::new(RefCell::new(Inner { host: host.clone() }));
        *self.inner.borrow_mut() = Some(state.clone());

        // Attach a per-instance `draw(value)` closure. The
        // prototype `render` method (installed once by `register`)
        // looks this up via `Reflect.get` and calls it — that's
        // how a single prototype method reaches the right
        // instance's host.
        let draw = Closure::wrap(Box::new(move |value: JsValue| {
            let s = state.borrow();
            render_into(&s.host, &value);
        }) as Box<dyn FnMut(JsValue)>);
        let draw_fn: &Function = draw.as_ref().unchecked_ref();
        let _ = Reflect::set(this.as_ref(), &"draw".into(), draw_fn);
        draw.forget();
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.inner.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Register the element and install a `render` prototype method
/// that delegates to the per-instance `draw` closure attached
/// during `connected_callback`. Idempotent.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkInspector::define("tonk-inspector");
    install_render_method();
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-inspector").is_undefined()
}

fn install_render_method() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-inspector");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    // Real JS function so `this` follows method-call semantics —
    // see view.rs `install_render_method` for the rationale.
    let render_fn = Function::new_with_args(
        "value",
        "if (typeof this.draw === 'function') this.draw(value);",
    );
    let _ = Reflect::set(&proto, &"render".into(), &render_fn);
}

/// Replace the host's contents with a fresh rendering of `value`.
fn render_into(host: &Element, value: &JsValue) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    host.set_inner_html("");
    let rendered = render_value(&document, value, 0);
    let _ = host.append_child(&rendered);
}

/// Render a single value to a DOM node. `depth` controls whether
/// nested objects/arrays show expanded or collapse behind a
/// disclosure.
fn render_value(document: &Document, value: &JsValue, depth: u32) -> Element {
    if value.is_null() {
        return styled_span(document, "null", "tonk-inspector-null");
    }
    if value.is_undefined() {
        return styled_span(document, "undefined", "tonk-inspector-undefined");
    }
    if let Some(s) = value.as_string() {
        return styled_span(
            document,
            &format!("\"{}\"", escape_for_text(&s)),
            "tonk-inspector-string",
        );
    }
    if let Some(n) = value.as_f64() {
        return styled_span(document, &format_number(n), "tonk-inspector-number");
    }
    if let Some(b) = value.as_bool() {
        return styled_span(
            document,
            if b { "true" } else { "false" },
            "tonk-inspector-boolean",
        );
    }
    if Array::is_array(value) {
        return render_array(document, &Array::from(value), depth);
    }
    if value.is_object() {
        // Plain object — falls through to here after Array check.
        let obj: &Object = value.unchecked_ref();
        return render_object(document, obj, depth);
    }
    // Fallback for things we don't model explicitly (BigInt,
    // Symbol, Date, Map, Set, functions): show their toString.
    let repr = format!("{:?}", value);
    styled_span(document, &repr, "tonk-inspector-other")
}

fn render_array(document: &Document, array: &Array, depth: u32) -> Element {
    let len = array.length();
    let summary = format!("Array({len})");
    let render_children = |container: &Element| {
        let bracket_open = styled_span(document, " [", "tonk-inspector-punct");
        let _ = container.append_child(&bracket_open);
        for i in 0..len {
            if i > 0 {
                let sep = styled_span(document, ", ", "tonk-inspector-punct");
                let _ = container.append_child(&sep);
            }
            let item = array.get(i);
            let node = render_value(document, &item, depth + 1);
            let _ = container.append_child(&node);
        }
        let bracket_close = styled_span(document, "]", "tonk-inspector-punct");
        let _ = container.append_child(&bracket_close);
    };
    render_container(document, &summary, depth, render_children)
}

fn render_object(document: &Document, obj: &Object, depth: u32) -> Element {
    let keys = Object::keys(obj);
    let len = keys.length();
    let summary = format!("Object({len})");
    let render_children = |container: &Element| {
        let brace_open = styled_span(document, " {", "tonk-inspector-punct");
        let _ = container.append_child(&brace_open);
        for i in 0..len {
            if i > 0 {
                let sep = styled_span(document, ", ", "tonk-inspector-punct");
                let _ = container.append_child(&sep);
            }
            let key = keys.get(i);
            let key_str = key.as_string().unwrap_or_default();
            let key_node = styled_span(document, &key_str, "tonk-inspector-key");
            let _ = container.append_child(&key_node);
            let colon = styled_span(document, ": ", "tonk-inspector-punct");
            let _ = container.append_child(&colon);
            let val = Reflect::get(obj, &key).unwrap_or(JsValue::UNDEFINED);
            let node = render_value(document, &val, depth + 1);
            let _ = container.append_child(&node);
        }
        let brace_close = styled_span(document, "}", "tonk-inspector-punct");
        let _ = container.append_child(&brace_close);
    };
    render_container(document, &summary, depth, render_children)
}

/// Wrap a collection in either an inline expanded view (depth ≤
/// [`EXPAND_DEPTH`]) or a `<details>` disclosure (deeper).
fn render_container<F>(document: &Document, summary: &str, depth: u32, fill: F) -> Element
where
    F: FnOnce(&Element),
{
    if depth <= EXPAND_DEPTH {
        let span: Element = document
            .create_element("span")
            .unwrap_or_else(|_| placeholder(document));
        let _ = span.set_attribute("class", "tonk-inspector-expanded");
        let label = styled_span(document, summary, "tonk-inspector-meta");
        let _ = span.append_child(&label);
        fill(&span);
        span
    } else {
        let details: Element = document
            .create_element("details")
            .unwrap_or_else(|_| placeholder(document));
        let _ = details.set_attribute("class", "tonk-inspector-collapsed");
        let summary_el = document
            .create_element("summary")
            .unwrap_or_else(|_| placeholder(document));
        summary_el.set_text_content(Some(summary));
        let _ = details.append_child(&summary_el);
        let body = document
            .create_element("span")
            .unwrap_or_else(|_| placeholder(document));
        fill(&body);
        let _ = details.append_child(&body);
        details
    }
}

fn styled_span(document: &Document, text: &str, class: &str) -> Element {
    let span = document
        .create_element("span")
        .unwrap_or_else(|_| placeholder(document));
    let _ = span.set_attribute("class", class);
    span.set_text_content(Some(text));
    span
}

fn placeholder(document: &Document) -> Element {
    // Fallback when create_element fails — extremely unlikely on
    // valid browsers, but keeps the type chain total.
    document
        .create_element("span")
        .unwrap_or_else(|_| panic!("document.createElement always succeeds"))
}

/// Render numbers the way JS would, without locale-specific
/// formatting. Integers keep no decimals; floats use the shortest
/// `f64` round-trip form Rust provides.
fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_owned();
    }
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    if n == n.trunc() && n.abs() < 1e16 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Minimal escape for the *text content* of a string label —
/// `set_text_content` already handles HTML escaping, so we only
/// need to render embedded quotes/backslashes in a JS-ish way.
fn escape_for_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}
