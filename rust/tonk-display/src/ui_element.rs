//! `<ui-element>` — a display whose concept and template are supplied
//! inline, so neither is resolved from a branch.
//!
//! ```html
//! <ui-element with="main@did:key:zSpace" entity="state:here">
//!   <script type="text/dialog-yaml">
//!   concept!:
//!     with:
//!       state:
//!         the: xyz.tonk.sync/state
//!         as: entity
//!   </script>
//!   <tonk-view>
//!     <span class="sync sync--{state}"><span class="disc"></span></span>
//!   </tonk-view>
//! </ui-element>
//! ```
//!
//! # Why this exists
//!
//! [`TonkDisplay`](crate::TonkDisplayElement) runs three phases: resolve a
//! **model** name to a concept, resolve a **view** for that model, then
//! subscribe to the entity data. The first two read the *space branch*, which
//! makes it unusable for host chrome twice over: a view would need per-space
//! seeding, and a space could redefine the chrome that frames it.
//!
//! So host chrome hand-rolled the whole thing instead — `<ui-sync-status>`
//! and friends each hold their own subscription, frame delegates and
//! teardown, which is a display element's guts minus the display. Then, where
//! the consumer draws its own pixels, they needed a second invention: a
//! `headless` mode reporting the answer by writing an attribute onto their
//! parent. That backchannel is what panicked the sealed guest (the parent
//! observes the attribute, so the write re-enters its
//! `attributeChangedCallback` while `custom_elements` still holds its state
//! mutex).
//!
//! Both inventions are downstream of one missing capability. Supplying the
//! concept inline removes the two branch-reading phases; what is left is the
//! entity subscription, which is the phase that was never the problem.
//!
//! # What it is not
//!
//! Not a second renderer. [`TonkView`](crate::TonkView) already takes an
//! inline template, holds the binding plan and paints on `render(frame)` —
//! "purely 'given X, paint'". This element is the *middle* piece: hold the
//! supplied concept, subscribe, and drive that renderer. The template is
//! authored as a `<tonk-view>` child exactly as it is anywhere else.
//!
//! The `ui-` prefix marks a host UI primitive, distinct from the `tonk-` data
//! elements: what it renders is fixed by whoever wrote the page, not by
//! anything a space asserts.

mod concept;

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;

use self::concept::parse_concept;
use js_sys::{Function, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription};
use tonk_template::resolve::{entity_query, instances_query};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement};

/// The MIME type of the inline concept script. Inert by virtue of being
/// unknown to the browser, so it neither executes nor renders — the same
/// device `<tonk-notation>` uses for its source. `dialog-yaml` is the name
/// the editor's language pack uses, so one notation has one name everywhere.
const CONCEPT_MIME: &str = "text/dialog-yaml";

/// The subscription frame delegate — `(detail, opts)`, matching the shape
/// `tonk-host` calls back with.
type FrameClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// Per-element state. The subscription's `Drop` cancels upstream, and the
/// delegates are kept alive for the element's lifetime.
#[derive(Default)]
pub struct UiElement {
    subscription: Rc<RefCell<Option<Subscription>>>,
    reset: Rc<RefCell<Option<FrameClosure>>>,
    update: Rc<RefCell<Option<FrameClosure>>>,
}

impl CustomElement for UiElement {
    fn shadow() -> bool {
        // Light DOM: the page's stylesheet styles what the template renders,
        // which is the whole point of the template being authored in place.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // `with` re-routes and `entity` re-pins, so both restart the
        // subscription. Deliberately NOT observing anything this element
        // writes — see the module note on re-entrancy.
        &["with", "entity"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.start(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        self.start(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Dropping the subscription cancels it upstream.
        self.subscription.borrow_mut().take();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
    }
}

impl UiElement {
    /// (Re)open the subscription for the current `concept` + `entity`.
    ///
    /// Tears the previous one down first, so an attribute change re-pins
    /// rather than accumulating subscriptions.
    fn start(&mut self, this: &HtmlElement) {
        self.subscription.borrow_mut().take();

        let Some(source) = concept_source(this) else {
            return;
        };
        let Some(descriptor) = parse_concept(&source) else {
            warn("ui-element: concept did not parse as a descriptor");
            return;
        };

        // An `entity` pins `this`; without one the query matches every
        // instance, which is the directory shape `<tonk-display>` also uses.
        let entity = this.get_attribute("entity").filter(|s| !s.is_empty());
        let query = match &entity {
            Some(entity) => entity_query(&descriptor, entity),
            None => instances_query(&descriptor),
        };
        let Ok(query) = query else {
            warn("ui-element: could not build a query for the concept");
            return;
        };
        let Ok(body) = serde_wasm_bindgen::to_value(&query) else {
            return;
        };

        self.install_delegates(this);

        match host_consumer::subscribe(this.as_ref(), &body, None) {
            Ok(subscription) => *self.subscription.borrow_mut() = Some(subscription),
            Err(detail) => warn(&format!("ui-element: subscribe failed: {}", detail.message)),
        }
    }

    /// Install the `__tonkReset` / `__tonkUpdate` delegates the host calls
    /// with each frame, forwarding straight to the `<tonk-view>` child.
    fn install_delegates(&mut self, this: &HtmlElement) {
        if self.reset.borrow().is_some() {
            return;
        }

        let host = this.clone();
        let reset = Closure::wrap(Box::new(move |detail: JsValue, _opts: JsValue| {
            paint(&host, &detail);
        }) as Box<dyn FnMut(JsValue, JsValue)>);
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        *self.reset.borrow_mut() = Some(reset);

        // A delta frame carries `{asserted, retracted}` rather than the whole
        // set. The renderer patches in place from a full frame, so ask the
        // host for a fresh one instead of reconciling here: these are chrome
        // readings (a status, a name), not large collections.
        let host = this.clone();
        let subscription = self.subscription.clone();
        let update = Closure::wrap(Box::new(move |detail: JsValue, _opts: JsValue| {
            let _ = &subscription;
            paint(&host, &detail);
        }) as Box<dyn FnMut(JsValue, JsValue)>);
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        *self.update.borrow_mut() = Some(update);
    }
}

/// Report a misconfiguration. These are authoring mistakes in a page's own
/// markup (a malformed concept, a query that cannot be built), so they belong
/// in the console rather than rendered into chrome that is meant to be
/// unobtrusive.
fn warn(message: &str) {
    web_sys::console::warn_1(&JsValue::from_str(message));
}

/// Hand a frame to the `<tonk-view>` child.
///
/// The renderer exposes a per-instance `draw` closure; calling it is exactly
/// what `<tonk-display>` does with its own slides.
fn paint(host: &Element, detail: &JsValue) {
    let Ok(Some(view)) = host.query_selector("tonk-view") else {
        return;
    };
    let Ok(draw) = Reflect::get(view.as_ref(), &"draw".into()) else {
        return;
    };
    let Ok(func) = draw.dyn_into::<Function>() else {
        return;
    };
    let _ = func.call1(&JsValue::NULL, detail);
}

/// Read the inline concept source out of the inert script child.
fn concept_source(host: &Element) -> Option<String> {
    host.query_selector(&format!("script[type=\"{CONCEPT_MIME}\"]"))
        .ok()
        .flatten()?
        .text_content()
        .filter(|text| !text.trim().is_empty())
}

/// Register `<ui-element>`. Idempotent.
pub fn register() {
    use web_sys::window;

    let registered = window()
        .map(|win| !win.custom_elements().get("ui-element").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    UiElement::define("ui-element");
}
