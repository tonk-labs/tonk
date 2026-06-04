//! `<tonk-concept>` custom-element implementation.
//!
//! Wires the static-side modules ([`crate::resolve`],
//! [`crate::template`], [`crate::render`]) into a lifecycle:
//! snapshot the row template once, resolve `source` into a wire
//! query via a `tonk-query` event, open a subscription via a
//! `tonk-subscribe` event, and diff each frame into the live DOM.
//!
//! IO is owned by the nearest `<tonk-host>` ancestor. The element
//! itself has no `space` / `branch` attributes — routing comes
//! from `<tonk-repository>` / `<tonk-branch>` annotators that
//! decorate the operation events on the way up.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use ipld_core::ipld::Ipld;
use js_sys::{Function, Reflect};
use tonk_host::DepthAnnotator;
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

use crate::error::{ErrorDetail, ErrorKind};
use crate::render::Renderer;
use crate::resolve::{ParsedSource, parse_source, phase1_query, phase2_query};
use crate::template::{BindingPlan, extract_row_plan, snapshot_template};

/// Internal lifecycle state shared across async closures.
struct Inner {
    /// Snapshot from the host's child template — `None` until
    /// `connected_callback` ran successfully.
    plan: Option<BindingPlan>,
    /// The cloneable template fragment.
    template: Option<web_sys::DocumentFragment>,
    /// Where rendered rows are appended.
    container: Option<Element>,
    /// Active diff renderer once the subscription has opened.
    renderer: Option<Renderer>,
    /// Active subscription handle — dropping it cancels the
    /// upstream via the host's registry.
    subscription: Option<HostSubscription>,
    /// Depth annotator listeners installed at connect.
    depth_annotator: Option<DepthAnnotator>,
}

impl Inner {
    fn new() -> Self {
        Self {
            plan: None,
            template: None,
            container: None,
            renderer: None,
            subscription: None,
            depth_annotator: None,
        }
    }
}

/// The custom element. `custom-elements` requires `Default` so we
/// hold no fields directly — the real state lives in `Inner` and
/// is only allocated on `connected_callback` (the only point at
/// which we have a host element to attach to).
#[derive(Default)]
pub struct TonkConcept {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkConcept {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["source"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host_element: Element = this.clone().into();
        let snapshot = match snapshot_template(&host_element) {
            Ok(s) => s,
            Err(err) => {
                dispatch_error(&host_element, err);
                return;
            }
        };
        let plan = extract_row_plan(&snapshot.fragment);

        let state = Rc::new(RefCell::new(Inner::new()));
        {
            let mut s = state.borrow_mut();
            s.plan = Some(plan);
            s.template = Some(snapshot.fragment);
            s.container = Some(snapshot.container);
            s.depth_annotator = Some(tonk_host::install_depth_annotator(&host_element));
        }
        install_method_delegates(&host_element, &state);
        *self.inner.borrow_mut() = Some(state.clone());

        start_subscription(&host_element, state);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take() {
            // Drop the subscription (cancels upstream via host).
            // Drop the depth annotator (detaches listeners).
            state.borrow_mut().subscription.take();
            state.borrow_mut().depth_annotator.take();
            // Notify host explicitly so it drops the registry
            // entry without waiting for `isConnected === false`
            // detection.
            let el: Element = this.clone().into();
            host_consumer::dispatch_unsubscribe(&el);
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host_element: Element = this.clone().into();
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        {
            let mut s = state.borrow_mut();
            s.subscription.take();
            if let Some(mut renderer) = s.renderer.take() {
                renderer.clear();
            }
        }
        start_subscription(&host_element, state);
    }
}

/// Public entry point — registers the element with the page.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkConcept::define("tonk-concept");
    install_method_shims();
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-concept").is_undefined()
}

fn start_subscription(host: &Element, state: Rc<RefCell<Inner>>) {
    let host = host.clone();
    spawn_local(async move {
        if let Err(err) = subscribe(&host, state).await {
            dispatch_error(&host, err);
        }
    });
}

async fn subscribe(host: &Element, state: Rc<RefCell<Inner>>) -> Result<(), ErrorDetail> {
    let source_attr = host.get_attribute("source").unwrap_or_default();
    if source_attr.is_empty() {
        return Err(ErrorDetail::new(
            ErrorKind::Descriptor,
            "<tonk-concept> requires a `source` attribute",
        ));
    }
    let parsed: ParsedSource = parse_source(&source_attr);

    // Phase-1 lookup via the host. Context (space, branch) is
    // annotated on the way up by ancestors.
    let phase1_q = phase1_query(&parsed);
    let phase1_body = serde_wasm_bindgen::to_value(&phase1_q)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase1 body: {e}")))?;
    let result = host_consumer::query(host, &phase1_body)
        .await
        .map_err(|e| ErrorDetail::new(map_kind(e.kind), e.message))?;
    let descriptor_json = extract_descriptor(&result)?;

    let phase2 = phase2_query(&descriptor_json, &parsed.filters)
        .map_err(|e| ErrorDetail::new(ErrorKind::Descriptor, format!("phase2: {e}")))?;

    // Initialise the renderer now that we know the descriptor.
    {
        let mut s = state.borrow_mut();
        let plan = s
            .plan
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no template plan"))?;
        let template = s
            .template
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no template fragment"))?;
        let container = s
            .container
            .clone()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no render container"))?;
        s.renderer = Some(Renderer::new(plan, template, container));
    }

    // Open the subscription via the host. Frames arrive through
    // our `reset` method (single subscription — no tag needed).
    let phase2_body = serde_wasm_bindgen::to_value(&phase2)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase2 body: {e}")))?;
    let sub = host_consumer::subscribe(host, &phase2_body, None)
        .map_err(|e| ErrorDetail::new(map_kind(e.kind), e.message))?;
    state.borrow_mut().subscription = Some(sub);
    dispatch_event(host, "tonk-concept:connected", None);
    Ok(())
}

/// Decode the host's phase-1 result into the descriptor JSON
/// from the first row's `source` field.
fn extract_descriptor(value: &JsValue) -> Result<String, ErrorDetail> {
    let conclusions: Vec<Conclusion> = serde_wasm_bindgen::from_value(value.clone())
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("phase1 result: {e}")))?;
    let first = conclusions
        .into_iter()
        .next()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::UnknownSource, "no concept matched"))?;
    match first.fields.get("source") {
        Some(Ipld::String(s)) => Ok(s.to_owned()),
        _ => Err(ErrorDetail::new(
            ErrorKind::Descriptor,
            "phase1 row missing `source` field",
        )),
    }
}

/// Map `tonk_host::error::ErrorKind` to our local
/// `crate::error::ErrorKind`. The two enums are identical but
/// distinct types (one was forked from the other when the
/// transport modules moved up to `tonk-host`).
fn map_kind(kind: tonk_host::error::ErrorKind) -> ErrorKind {
    match kind {
        tonk_host::error::ErrorKind::UnknownSource => ErrorKind::UnknownSource,
        tonk_host::error::ErrorKind::Network => ErrorKind::Network,
        tonk_host::error::ErrorKind::Parse => ErrorKind::Parse,
        tonk_host::error::ErrorKind::Descriptor => ErrorKind::Descriptor,
    }
}

/// Install the `reset` / `update` / `error` method shims on the
/// `<tonk-concept>` prototype. Each shim reads the per-instance
/// `__tonkReset` / `__tonkUpdate` / `__tonkError` property
/// (a Rust closure attached in `connected_callback`) and calls
/// it with the payload and opts.
fn install_method_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-concept");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let error_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkError === 'function') this.__tonkError(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
    let _ = Reflect::set(&proto, &"error".into(), &error_fn);
}

/// Per-instance: write closures to `__tonkReset` / `__tonkUpdate`
/// / `__tonkError` on the element. The closures capture the
/// element's state.
fn install_method_delegates(host: &Element, state: &Rc<RefCell<Inner>>) {
    let host_for_reset = host.clone();
    let state_for_reset = state.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, _opts| {
            on_reset(&host_for_reset, &state_for_reset, payload);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |_payload, _opts| {
            // V1 SW emits only `reset`. Once delta semantics
            // arrive, apply the delta to the renderer.
        }));
    let _ = Reflect::set(host, &"__tonkUpdate".into(), update.as_ref());
    update.forget();

    let host_for_error = host.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, _opts| {
            let message = Reflect::get(&payload, &"message".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| format!("{payload:?}"));
            dispatch_error(
                &host_for_error,
                ErrorDetail::new(ErrorKind::Network, message),
            );
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}

/// `reset(conclusions, opts)` — full snapshot frame. Drive the
/// renderer's diff against its prior state.
fn on_reset(host: &Element, state: &Rc<RefCell<Inner>>, payload: JsValue) {
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(e) => {
            dispatch_error(
                host,
                ErrorDetail::new(ErrorKind::Parse, format!("frame: {e}")),
            );
            return;
        }
    };
    if let Some(renderer) = state.borrow_mut().renderer.as_mut() {
        renderer.apply(&conclusions);
    }
    dispatch_event(
        host,
        "tonk-concept:result",
        Some(event_detail(&conclusions)),
    );
}

fn dispatch_error(host: &Element, err: ErrorDetail) {
    dispatch_event(host, "tonk-concept:error", Some(event_detail(&err)));
}

/// Serialize an event detail as a plain JS object (not a `Map`) so
/// consumers can read fields via dot access (`event.detail.<field>`).
/// `serde_wasm_bindgen::to_value` renders structs/maps as a JS `Map`,
/// which makes `detail.<field>` come out `undefined`; the
/// json-compatible serializer emits a plain object instead.
fn event_detail<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

fn dispatch_event(host: &Element, name: &str, detail: Option<JsValue>) {
    let init = CustomEventInit::new();
    if let Some(d) = detail {
        init.set_detail(&d);
    }
    init.set_bubbles(true);
    init.set_composed(true);
    let Ok(event) = CustomEvent::new_with_event_init_dict(name, &init) else {
        return;
    };
    let _ = host.dispatch_event(&event);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Outbound `tonk-concept:result` / `:error` details must be plain
    /// JS objects so consumers can read `event.detail.<field>`.
    /// `serde_wasm_bindgen::to_value` renders them as a `Map`, where
    /// `Reflect`/dot access reads `undefined`.
    #[dialog_common::test]
    fn it_serializes_event_detail_as_a_dot_accessible_object() {
        #[derive(serde::Serialize)]
        struct Probe {
            label: String,
        }
        let detail = event_detail(&Probe {
            label: "hi".to_owned(),
        });
        let field = Reflect::get(&detail, &JsValue::from_str("label")).expect("reflect get");
        assert_eq!(field.as_string().as_deref(), Some("hi"));
    }
}
