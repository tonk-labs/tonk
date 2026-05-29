//! Consumer-side helpers for dispatching the four operation
//! events and reading their results.
//!
//! Consumers call these from Rust without thinking about the
//! event-construction boilerplate. Each helper:
//!
//! 1. Builds a `CustomEvent` with `bubbles: true, composed: true,
//!    cancelable: true` and the appropriate detail shape.
//! 2. Dispatches on the consumer element.
//! 3. Reads `event.defaultPrevented` to detect whether a
//!    `<tonk-host>` ancestor handled the event.
//! 4. For one-shots, awaits `detail.result`; for subscribe,
//!    returns the `detail.subscription` handle.
//!
//! These helpers are pure event dispatchers — they do not touch
//! the worker directly. The host element owns all IO.

use js_sys::{Function, Object, Promise, Reflect};

use crate::error::{ErrorDetail, ErrorKind};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CustomEvent, CustomEventInit, Element, window};

use crate::events;

/// Handle to an open subscription. Hold it for the subscription's
/// lifetime; call `.cancel()` to tear down, or drop it (no-op —
/// the host will detect detach via `consumer.isConnected`).
pub struct Subscription {
    cancel_fn: Option<Function>,
    consumer: Element,
}

impl Subscription {
    /// Cancel the subscription. Idempotent — subsequent calls
    /// do nothing.
    pub fn cancel(&mut self) {
        if let Some(f) = self.cancel_fn.take() {
            let _ = f.call0(&JsValue::UNDEFINED);
        }
    }

    /// Dispatch `tonk-unsubscribe` on the consumer element. Use
    /// this from `disconnected_callback` as a backstop — the host
    /// also detects detach via `isConnected`.
    pub fn dispatch_unsubscribe(&self) {
        dispatch_unsubscribe(&self.consumer);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Dispatch `tonk-query` on `consumer` with the given query body,
/// then await `detail.result`. Returns the parsed JSON response
/// as a `JsValue`.
pub async fn query(consumer: &Element, query_body: &JsValue) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"query".into(), query_body).ok();
    let result_promise = dispatch_one_shot(consumer, events::QUERY, &detail)?;
    let result = JsFuture::from(result_promise)
        .await
        .map_err(|e| js_to_error(&e))?;
    Ok(result)
}

/// Dispatch `tonk-claim` on `consumer` with the given structured
/// transact request, await `detail.result`.
pub async fn claim(consumer: &Element, request: &JsValue) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"request".into(), request).ok();
    let result_promise = dispatch_one_shot(consumer, events::CLAIM, &detail)?;
    let result = JsFuture::from(result_promise)
        .await
        .map_err(|e| js_to_error(&e))?;
    Ok(result)
}

/// Dispatch `tonk-evaluate` on `consumer` with the given raw
/// asserted-notation text, await `detail.result`.
pub async fn evaluate(consumer: &Element, document: &str) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"document".into(), &JsValue::from_str(document)).ok();
    let result_promise = dispatch_one_shot(consumer, events::EVALUATE, &detail)?;
    let result = JsFuture::from(result_promise)
        .await
        .map_err(|e| js_to_error(&e))?;
    Ok(result)
}

/// Dispatch `tonk-subscribe` on `consumer` with the given query
/// body and optional tag. Returns a subscription handle. Frames
/// arrive via `consumer.reset(conclusions, { tag })` /
/// `consumer.update(delta, { tag })` / `consumer.error(detail,
/// { tag })` method calls.
pub fn subscribe(
    consumer: &Element,
    query_body: &JsValue,
    tag: Option<&JsValue>,
) -> Result<Subscription, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"query".into(), query_body).ok();
    if let Some(t) = tag {
        Reflect::set(&detail, &"tag".into(), t).ok();
    }
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(true);
    let ev = CustomEvent::new_with_event_init_dict(events::SUBSCRIBE, &init).map_err(|e| {
        ErrorDetail::new(
            ErrorKind::Network,
            format!("tonk-subscribe event construction: {e:?}"),
        )
    })?;
    let _ = consumer.dispatch_event(&ev);
    if !ev.default_prevented() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            "tonk-subscribe: no <tonk-host> ancestor",
        ));
    }
    let subscription_obj = Reflect::get(&detail, &"subscription".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Network,
                "tonk-subscribe: host did not write detail.subscription",
            )
        })?;
    let cancel_fn = Reflect::get(&subscription_obj, &"cancel".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok());
    Ok(Subscription {
        cancel_fn,
        consumer: consumer.clone(),
    })
}

/// Dispatch `tonk-unsubscribe` on `consumer`, no detail. Used
/// from a consumer's `disconnected_callback`.
pub fn dispatch_unsubscribe(consumer: &Element) {
    let init = CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(ev) = CustomEvent::new_with_event_init_dict(events::UNSUBSCRIBE, &init) {
        let _ = consumer.dispatch_event(&ev);
    }
}

/// Dispatch a one-shot operation event (`tonk-query` /
/// `tonk-claim` / `tonk-evaluate`) and return the `Promise` the
/// host wrote into `detail.result`.
fn dispatch_one_shot(
    consumer: &Element,
    event_name: &str,
    detail: &Object,
) -> Result<Promise, ErrorDetail> {
    // Make sure the window is alive — if not, we can't dispatch.
    let _ =
        window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no `window` available"))?;

    let init = CustomEventInit::new();
    init.set_detail(detail);
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(true);
    let ev = CustomEvent::new_with_event_init_dict(event_name, &init).map_err(|e| {
        ErrorDetail::new(
            ErrorKind::Network,
            format!("{event_name} event construction: {e:?}"),
        )
    })?;
    let _ = consumer.dispatch_event(&ev);
    if !ev.default_prevented() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("{event_name}: no <tonk-host> ancestor"),
        ));
    }
    Reflect::get(detail, &"result".into())
        .ok()
        .and_then(|v| v.dyn_into::<Promise>().ok())
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Network,
                format!("{event_name}: host did not write detail.result"),
            )
        })
}

/// Best-effort conversion of a rejected promise's value into an
/// `ErrorDetail`. The host serializes `ErrorDetail` via
/// `serde-wasm-bindgen`; here we read back the `kind` and
/// `message` fields by reflection (`ErrorDetail` only implements
/// `Serialize`, not `Deserialize`, so a round-trip via serde is
/// not possible).
fn js_to_error(value: &JsValue) -> ErrorDetail {
    let message = Reflect::get(value, &"message".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| format!("{value:?}"));
    ErrorDetail::new(ErrorKind::Network, message)
}
