//! `<tonk-view>` — a dumb single-template renderer.
//!
//! Authoring shape:
//!
//! ```html
//! <tonk-view>
//!   <p class="greeting">{message}</p>
//! </tonk-view>
//! ```
//!
//! Children at `connectedCallback` time become the template; the
//! element keeps a [`Renderer`] (binding plan + cloneable fragment)
//! and waits to be told what to paint. Owners drive it by calling
//! `el.render(conclusion)` — a single-argument method that accepts
//! a `{ this, fields }` JS object. The element inserts on the first
//! call and patches in place on subsequent calls; if no template
//! was supplied at mount time, calls are silently dropped.
//!
//! No network, no subscriptions, no upward events. The element is
//! purely "given X, paint" — owners (typically `<tonk-display>`)
//! are responsible for arranging the data and the lifetime.
//!
//! Internally the public `render` method on the prototype is a
//! tiny shim that reads `this.draw` — a per-instance closure
//! installed by `connectedCallback` — and calls it. The `draw`
//! closure captures the element's renderer state via
//! `Rc<RefCell<…>>`, so the prototype method doesn't need to
//! reach into any global element-to-state map.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use tonk_concept::template::snapshot_template;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, window};

use crate::render::Renderer;

/// Per-instance state. Held by the `draw` closure attached to
/// the element so the prototype `render` method can find it via
/// a plain property lookup.
struct Inner {
    renderer: Option<Renderer>,
}

/// The custom element.
#[derive(Default)]
pub struct TonkView {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkView {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();

        // Build a renderer from the child template, if there is one.
        // Empty hosts are fine — `.render()` calls then no-op until
        // someone fills children in and re-mounts.
        let renderer = snapshot_template(&host).ok().map(Renderer::from_snapshot);

        let state = Rc::new(RefCell::new(Inner { renderer }));
        *self.inner.borrow_mut() = Some(state.clone());

        // Attach a per-instance `draw(detail)` closure. The
        // prototype `render` method (installed once by `register`)
        // looks this up via `Reflect.get` and calls it — that's
        // how a single prototype method reaches the right
        // instance's state.
        let draw = Closure::wrap(Box::new(move |detail: JsValue| {
            let conclusion: Conclusion = match serde_wasm_bindgen::from_value(detail) {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut s = state.borrow_mut();
            if let Some(renderer) = s.renderer.as_mut() {
                renderer.apply(&conclusion);
            }
        }) as Box<dyn FnMut(JsValue)>);
        let draw_fn: &Function = draw.as_ref().unchecked_ref();
        let _ = Reflect::set(this.as_ref(), &"draw".into(), draw_fn);
        // The closure has to outlive the element — leaking is
        // correct here. The browser releases the element's
        // properties (including `draw`) when it garbage-collects
        // the host, so the closure becomes unreachable naturally.
        draw.forget();
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop the state — and with it the renderer. The mounted
        // row's DOM nodes are children of the host, which the
        // browser garbage-collects when the host detaches.
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

/// Register the `<tonk-view>` element with the page and augment
/// `HTMLElement.prototype.render` so callers can write
/// `el.render(conclusion)`.
///
/// Idempotent — re-registration is a no-op.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkView::define("tonk-view");
    install_render_method();
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-view").is_undefined()
}

/// Install a single `render(detail)` method on the `<tonk-view>`
/// prototype that delegates to the per-instance `draw` closure
/// attached during `connected_callback`. Implemented as a real JS
/// function (not a `wasm_bindgen::Closure`) so that `this` follows
/// JS method-call semantics — `el.render(x)` binds `this=el`,
/// which lets us look up `this.draw` correctly. A `Closure::wrap`
/// would have given us a plain function with no `this` binding,
/// so the same call would land with `this=detail`.
fn install_render_method() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-view");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let render_fn = Function::new_with_args(
        "detail",
        "if (typeof this.draw === 'function') this.draw(detail);",
    );
    let _ = Reflect::set(&proto, &"render".into(), &render_fn);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Mount a `<tonk-view>` with the given template HTML as
    /// children and attach it to the body. `register()` runs at the
    /// top so the custom-element prototype is set up before the
    /// element connects.
    fn mount(template_html: &str) -> Element {
        register();
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document
            .create_element("tonk-view")
            .expect("create tonk-view");
        host.set_inner_html(template_html);
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    /// Build a serialized `{ this, fields }` JsValue conclusion the
    /// way `<tonk-display>` would pass it into `el.render(detail)`.
    fn detail(this: &str, fields: &[(&str, &str)]) -> JsValue {
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), serde_json::Value::String((*v).to_owned()));
        }
        let conclusion = Conclusion {
            this: this.to_owned(),
            fields: map,
        };
        serde_wasm_bindgen::to_value(&conclusion).expect("serialize conclusion")
    }

    /// Pull the `draw` closure off the element and call it
    /// directly. Mirrors how `<tonk-display>::call_render` invokes
    /// the per-instance closure; using the prototype `render`
    /// method would also work but adds an indirection that's not
    /// what we're testing here.
    fn call_draw(el: &Element, detail: &JsValue) {
        let draw = Reflect::get(el.as_ref(), &"draw".into()).expect("draw");
        let func: Function = draw.dyn_into().expect("draw is a function");
        func.call1(&JsValue::NULL, detail).expect("call draw");
    }

    #[dialog_common::test]
    fn it_installs_a_per_instance_draw_closure_on_connect() {
        let host = mount("<p>{name}</p>");
        // `draw` is the per-instance shim the prototype `render`
        // method delegates to. Without it, callers couldn't drive
        // the element at all.
        let draw = Reflect::get(host.as_ref(), &"draw".into()).expect("draw present");
        assert!(
            draw.dyn_into::<Function>().is_ok(),
            "expected draw to be a JS Function",
        );
    }

    #[dialog_common::test]
    fn it_renders_the_template_when_draw_is_called() {
        let host = mount("<article><h1>{name}</h1></article>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        let html = host.inner_html();
        assert!(html.contains("Alice"), "expected Alice in {html}");
    }

    #[dialog_common::test]
    fn it_updates_in_place_on_subsequent_draws() {
        let host = mount("<article><h1>{name}</h1></article>");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        let first = host
            .query_selector("article")
            .unwrap()
            .expect("first article");
        call_draw(&host, &detail("did:key:zX", &[("name", "Alicia")])); // same `this`
        let second = host
            .query_selector("article")
            .unwrap()
            .expect("second article");
        // Same node — patched in place rather than swapped — so
        // downstream listeners on the rendered DOM survive updates.
        assert!(first.is_same_node(Some(second.unchecked_ref())));
        assert!(host.inner_html().contains("Alicia"));
    }

    #[dialog_common::test]
    fn it_exposes_a_prototype_render_method_that_delegates_to_draw() {
        let host = mount("<p>{name}</p>");
        // Call `render` through the prototype the way external JS
        // would: `host.render(detail)`. This proves the `this`
        // binding works — without it, `render` would invoke the
        // closure with `this = detail`, which silently no-ops.
        let render = Reflect::get(host.as_ref(), &"render".into()).expect("render present");
        let render_fn: Function = render.dyn_into().expect("render is a Function");
        render_fn
            .call1(host.as_ref(), &detail("did:key:zX", &[("name", "Bob")]))
            .expect("call render");
        assert!(
            host.inner_html().contains("Bob"),
            "got: {}",
            host.inner_html()
        );
    }

    #[dialog_common::test]
    fn it_silently_drops_draws_when_no_template_was_supplied() {
        let host = mount("");
        // No template → no renderer → calling draw is a no-op.
        // Just checking that we don't panic and the host stays empty.
        call_draw(&host, &detail("did:key:zX", &[("name", "Alice")]));
        assert_eq!(host.inner_html(), "");
    }
}
