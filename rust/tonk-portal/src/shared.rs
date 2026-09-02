//! Shared portal setup logic.
//!
//! Both `<tonk-portal>` and `<tonk-fab-portal>` create a sandboxed iframe
//! and wire it to the bridge; the only difference is how they style that
//! iframe. This module provides the common setup path, parameterised by a
//! caller-supplied style function.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Function, Reflect};
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};

/// Create and wire up the portal iframe, then store the resulting state.
///
/// `apply_style` receives the newly created (but not yet appended) iframe
/// and should write all CSS properties it needs. Called after the
/// `sandbox` and `allow` attributes are set, before the iframe is
/// appended to the host.
///
/// On success `inner` is set to `Some(state)`. If the DOM is unavailable
/// (no window/document) the function returns early and `inner` stays
/// `None`.
pub(crate) fn connect_portal(
    this: &HtmlElement,
    inner: &RefCell<Option<Rc<RefCell<PortalState>>>>,
    with: Option<Location>,
    allow: Allow,
    trusted_profile_controls: bool,
    apply_style: impl Fn(&HtmlIFrameElement),
) {
    let host: Element = this.clone().into();

    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(iframe) = document.create_element("iframe") else {
        return;
    };
    let Ok(iframe) = iframe.dyn_into::<HtmlIFrameElement>() else {
        return;
    };

    // Opaque-origin sandbox: scripts run but `parent.document` is
    // unreachable. The bridge bootstrap reaches the parent only over
    // a `MessagePort` it opens and transfers in its `hello`.
    //
    // `allow-forms` lets the guest's `<form>`s fire their `submit` event
    // (so declarative `onsubmit=` bindings run); it does NOT let a form
    // navigate the guest away, because the runtime installs a global
    // capture-phase `submit` guard that `preventDefault`s every
    // submission before its native action. `allow-downloads` lets a guest
    // trigger a save (e.g. the blob file card's Download button, which
    // clicks an `<a download>` over an object URL); without it Chrome
    // silently blocks the download. We deliberately withhold
    // `allow-top-navigation` and `allow-same-origin` — the guest still
    // can't reach the parent or a real origin.
    let _ = iframe.set_attribute("sandbox", "allow-scripts allow-forms allow-downloads");

    // Delegate the `clipboard-write` Permissions Policy into the guest so
    // its copy buttons (e.g. the share dialog's invite-link copy) can call
    // `navigator.clipboard.writeText`. This is Permissions Policy, NOT a
    // sandbox grant — orthogonal to the sandbox lockdown above; without it
    // the API is blocked outright regardless of sandbox flags.
    let _ = iframe.set_attribute("allow", "clipboard-write");

    apply_style(&iframe);

    let state = Rc::new(RefCell::new(PortalState::new()));
    // The portal's routing context (`with`) and reach (`allow`) are set by
    // the trusted caller element, never by the guest: `<tonk-site>` parses
    // its attributes, `<tonk-fab-portal>` grants `*`, the generic
    // `<tonk-portal>` grants `self` — so a synced/untrusted content guest
    // can forward a route but the bridge denies anything un-listed.
    state
        .borrow_mut()
        .set_route(with, allow, trusted_profile_controls);
    bridge::register_portal(&iframe, &host, &state);
    install_method_delegates(&host, &state);

    // Append before assigning `srcdoc` so `contentWindow` exists;
    // the `hello` listener matches the live `contentWindow`, so the
    // bootstrap script resolves this portal when it posts `hello`.
    let content = host.get_attribute("content").unwrap_or_default();
    // In `runtime` mode the guest renders OUR elements (a real
    // `<tonk-display>`): the bootstrap additionally pulls in the
    // injected element runtime + CSS before `content` upgrades.
    let runtime = host.has_attribute("runtime");
    let _ = host.append_child(&iframe);
    let base = space_base(&state.borrow());
    let srcdoc = if runtime {
        bridge::bootstrap_srcdoc_with_runtime(&content, &base)
    } else {
        bridge::bootstrap_srcdoc(&content, &base)
    };
    let _ = iframe.set_attribute("srcdoc", &srcdoc);

    state.borrow_mut().iframe = Some(iframe);
    *inner.borrow_mut() = Some(state);
}

/// Reload the portal's iframe from the current `content` attribute,
/// cancelling live subscriptions first.
///
/// Shared by both portal elements; `content`/`entity`/`model` attribute
/// changes call this.
pub(crate) fn reload_portal(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let mut s = state.borrow_mut();
    s.clear_subs();
    if let Some(iframe) = s.iframe.as_ref() {
        let content = host.get_attribute("content").unwrap_or_default();
        let base = space_base(&s);
        let srcdoc = if host.has_attribute("runtime") {
            bridge::bootstrap_srcdoc_with_runtime(&content, &base)
        } else {
            bridge::bootstrap_srcdoc(&content, &base)
        };
        let _ = iframe.set_attribute("srcdoc", &srcdoc);
    }
}

/// The per-space synthetic origin (`https://{label}.tonk.network/`) for this
/// portal's routing context, or empty when the portal targets no space (the
/// profile/Hub) — where in-guest links are genuinely top-level.
fn space_base(state: &bridge::PortalState) -> String {
    state
        .route_space()
        .and_then(|space| tonk_host::space_origin::space_origin_for(&space))
        .unwrap_or_default()
}

/// Install `reset` / `update` / `error` on the named element's prototype,
/// each forwarding to the per-instance `__tonk*` closure.
///
/// Called once during `register()` / `register_fab_portal()` after
/// `CustomElement::define` so the constructor is already registered.
/// The `element_name` must match the name passed to `define`.
pub(crate) fn install_method_shims(element_name: &str) {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get(element_name);
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

/// Write the per-instance `__tonkReset` / `__tonkError` closures the
/// prototype shims forward subscription frames to. Mirrors
/// `<tonk-display>`'s method-delegate pattern.
pub(crate) fn install_method_delegates(host: &Element, state: &Rc<RefCell<PortalState>>) {
    let reset_state = state.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_reset(&reset_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkReset".into(), reset.as_ref());
    reset.forget();

    let update_state = state.clone();
    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_update(&update_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkUpdate".into(), update.as_ref());
    update.forget();

    let error_state = state.clone();
    let error: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload, opts| {
            bridge::route_error(&error_state, payload, opts);
        }));
    let _ = Reflect::set(host, &"__tonkError".into(), error.as_ref());
    error.forget();
}
