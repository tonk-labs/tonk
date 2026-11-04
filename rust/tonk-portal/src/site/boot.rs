//! What a `<tonk-site>` shows while its guest starts, and when it cannot.
//!
//! A guest frame paints only the site's surface colour until the guest
//! answers the bridge handshake, and before this a failed `tonk:load`
//! claim was only logged, so the guest's quiet pulse ran forever. Both
//! now say what is happening:
//!
//! - **booting**: from the moment a guest frame is built until the guest
//!   answers (`tonk:guest-ready`), the frame pulses and the site is
//!   `aria-busy`. A re-route inside a live guest boots nothing and shows
//!   nothing.
//! - **failed**: a guest that has not answered within
//!   [`GUEST_BOOT_LIMIT_MS`], or a `tonk:load` claim that fails twice,
//!   replaces the frame with a short notice and a button that builds the
//!   guest again.
//!
//! The state is reflected as `data-site-state` (`booting`, `ready`,
//! `failed`) so an embedder can style it.

use js_sys::{Function, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlElement, window};

use super::{StateCell, Trigger, resolve_and_render, teardown};

/// The attribute that reflects where the guest is.
pub(super) const SITE_STATE: &str = "data-site-state";

/// Marks the notice a failed site shows in place of its frame.
const FAILURE_ATTR: &str = "data-site-failure";

/// The property that holds the running pulse on a booting frame.
const PULSE_PROP: &str = "__tonkBootPulse";

/// How long a guest may take to answer the handshake before the site says
/// it did not start. The guest fetches nothing on its own (its runtime
/// comes from this page), so one that has not answered by then is stuck,
/// not slow.
pub(super) const GUEST_BOOT_LIMIT_MS: i32 = 15_000;

/// Words for a guest that did not answer in time.
pub(super) const DID_NOT_START: &str = "This page did not start.";

/// Words for a `tonk:load` claim that failed.
pub(super) const DID_NOT_LOAD: &str = "This page could not be loaded.";

/// A guest frame was just built for `host`: show that it is starting, and
/// give up on it after [`GUEST_BOOT_LIMIT_MS`].
pub(super) fn begin(host: &HtmlElement, cell: &StateCell) {
    remove_failure(host);
    let _ = host.set_attribute(SITE_STATE, "booting");
    let _ = host.set_attribute("aria-busy", "true");
    let Some(frame) = current_frame(cell) else {
        return;
    };
    pulse(&frame);
    watch_boot(host.clone(), cell.clone(), frame);
}

/// Fail the boot of `frame` if it has not answered within
/// [`GUEST_BOOT_LIMIT_MS`]. A hidden page may not run the guest at all, so
/// the limit only counts while the page is visible: a check that lands on
/// a hidden page looks again a limit later.
fn watch_boot(host: HtmlElement, cell: StateCell, frame: Element) {
    let expired = Closure::once_into_js(move || {
        let still_booting = host.get_attribute(SITE_STATE).as_deref() == Some("booting");
        let same_frame = current_frame(&cell).is_some_and(|current| current == frame);
        if !host.is_connected() || !still_booting || !same_frame {
            return;
        }
        let hidden = window()
            .and_then(|w| w.document())
            .is_some_and(|document| document.hidden());
        if hidden {
            watch_boot(host, cell, frame);
        } else {
            fail(&host, &cell, DID_NOT_START);
        }
    });
    if let Some(win) = window() {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            expired.unchecked_ref(),
            GUEST_BOOT_LIMIT_MS,
        );
    }
}

/// The guest behind `host` answered the handshake.
pub(super) fn ready(host: &Element) {
    if host.get_attribute(SITE_STATE).as_deref() != Some("booting") {
        return;
    }
    let _ = host.set_attribute(SITE_STATE, "ready");
    let _ = host.remove_attribute("aria-busy");
    if let Ok(Some(frame)) = host.query_selector(":scope > iframe") {
        stop_pulse(&frame);
    }
}

/// Show that the site could not load, with a way to build the guest again.
pub(super) fn fail(host: &HtmlElement, cell: &StateCell, message: &str) {
    tonk_common::diagnostics::count(tonk_common::diagnostics::names::SITE_LOAD_FAILED);
    let _ = host.set_attribute(SITE_STATE, "failed");
    let _ = host.remove_attribute("aria-busy");
    if let Some(frame) = current_frame(cell) {
        stop_pulse(&frame);
        let _ = Reflect::set(&frame, &"hidden".into(), &JsValue::TRUE);
        if let Some(frame) = frame.dyn_ref::<HtmlElement>() {
            let _ = frame.style().set_property("display", "none");
        }
    }
    remove_failure(host);
    let Some(notice) = failure_notice(message) else {
        return;
    };
    if let Ok(Some(button)) = notice.query_selector("button") {
        let host = host.clone();
        let cell = cell.clone();
        let retry = Closure::<dyn FnMut()>::new(move || {
            remove_failure(&host);
            teardown(&cell);
            resolve_and_render(&host, cell.clone(), Trigger::Retry);
        });
        let _ = button.add_event_listener_with_callback("click", retry.as_ref().unchecked_ref());
        retry.forget();
    }
    let _ = host.insert_before(&notice, host.first_child().as_ref());
}

fn failure_notice(message: &str) -> Option<Element> {
    let document = window()?.document()?;
    let notice = document.create_element("div").ok()?;
    let _ = notice.set_attribute(FAILURE_ATTR, "");
    let _ = notice.set_attribute("role", "alert");
    let _ = notice.set_attribute(
        "style",
        "display:flex;flex-direction:column;align-items:flex-start;gap:12px;\
         padding:24px;color:var(--wa-color-text-normal, inherit);\
         font:inherit;",
    );
    let text = document.create_element("p").ok()?;
    let _ = text.set_attribute("style", "margin:0;");
    text.set_text_content(Some(message));
    let button = document.create_element("button").ok()?;
    let _ = button.set_attribute("type", "button");
    button.set_text_content(Some("try again"));
    let _ = notice.append_child(&text);
    let _ = notice.append_child(&button);
    Some(notice)
}

fn remove_failure(host: &HtmlElement) {
    if let Ok(Some(notice)) = host.query_selector(&format!(":scope > [{FAILURE_ATTR}]")) {
        notice.remove();
    }
}

fn current_frame(cell: &StateCell) -> Option<Element> {
    let state = cell.borrow().clone()?;
    let frame = state.borrow().iframe.clone()?;
    Some(frame.into())
}

/// Pulse the frame's surface while the guest starts, unless the reader
/// asked for less motion.
fn pulse(frame: &Element) {
    let start = Function::new_with_args(
        "frame",
        "if (typeof frame.animate !== 'function') return null;\
         if (globalThis.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches) return null;\
         return frame.animate([{opacity: 1}, {opacity: 0.6}],\
           {duration: 1000, delay: 400, iterations: Infinity, direction: 'alternate', easing: 'ease-in-out'});",
    );
    if let Ok(animation) = start.call1(&JsValue::NULL, frame) {
        let _ = Reflect::set(frame, &PULSE_PROP.into(), &animation);
    }
}

fn stop_pulse(frame: &Element) {
    let Ok(animation) = Reflect::get(frame, &PULSE_PROP.into()) else {
        return;
    };
    if animation.is_object()
        && let Ok(cancel) = Reflect::get(&animation, &"cancel".into())
        && let Ok(cancel) = cancel.dyn_into::<Function>()
    {
        let _ = cancel.call0(&animation);
    }
    let _ = Reflect::set(frame, &PULSE_PROP.into(), &JsValue::NULL);
}

/// Answer every guest handshake in this document: the bridge dispatches
/// `tonk:guest-ready` on the site that owns the guest. Installed once per
/// realm.
pub(super) fn install_ready_listener() {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if INSTALLED.with(|installed| installed.replace(true)) {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let on_ready = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        if target.tag_name().eq_ignore_ascii_case("tonk-site") {
            ready(&target);
        }
    });
    let _ = document
        .add_event_listener_with_callback("tonk:guest-ready", on_ready.as_ref().unchecked_ref());
    on_ready.forget();
}

/// Wait `ms` milliseconds on the page's timer.
pub(super) async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(win) = window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
