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
