//! Consumer-side helpers for dispatching the four operation
//! events and reading their results.
//!
//! Consumers call these from Rust without thinking about the
//! event-construction boilerplate. Each helper:
//!
//! 1. Builds a `CustomEvent` with `bubbles: true, composed: true,
//!    cancelable: true` and the appropriate detail shape.
//! 2. Dispatches on the consumer element.
//! 3. Reads `event.defaultPrevented` to detect whether an
//!    installed host (or the guest relay) handled the event.
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
    query_with_route(consumer, query_body, None, None, false).await
}

/// Like [`query`], but with an explicit cross-repo route: `space` (a
/// repository name/DID) and `branch`. When `space` is `Some`, the dispatched
/// event's `detail` is stamped with that route (and `profile = false`) so it
/// wins over any `with` ancestor context — see [`apply_route`]. With
/// `space = None` this is identical to [`query`].
pub async fn query_with_route(
    consumer: &Element,
    query_body: &JsValue,
    space: Option<&str>,
    branch: Option<&str>,
    profile: bool,
) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"query".into(), query_body).ok();
    apply_route(&detail, space, branch, profile);
    let result_promise = dispatch_one_shot(consumer, events::QUERY, &detail)?;
    let result = JsFuture::from(result_promise)
        .await
        .map_err(|e| js_to_error(&e))?;
    Ok(result)
}

/// Stamp an explicit cross-repo route onto a one-shot/subscribe `detail`.
///
/// Routing context normally resolves at handle time from the consumer's
/// nearest `with` ancestor. A caller with an explicit route (e.g. the portal
/// bridge relaying a sealed guest's forwarded `with` context, which the
/// guest's own relay already resolved) pre-fills the detail fields, and the
/// host honors those over the `with` ancestry. `profile` is forced to
/// `false` because an explicit repository route must override a
/// `…@profile` ancestor context (which would otherwise re-target the
/// profile endpoint). When `space` is `None` the detail is left bare so
/// `with` resolution proceeds as usual.
fn apply_route(detail: &Object, space: Option<&str>, branch: Option<&str>, profile: bool) {
    // A profile route targets the profile-as-repository endpoint: there is no
    // named `space`, just the branch. Stamp the flag and branch and return.
    if profile {
        Reflect::set(detail, &"profile".into(), &JsValue::TRUE).ok();
        if let Some(branch) = branch {
            Reflect::set(detail, &"branch".into(), &JsValue::from_str(branch)).ok();
        }
        return;
    }
    let Some(space) = space else {
        return;
    };
    Reflect::set(detail, &"space".into(), &JsValue::from_str(space)).ok();
    Reflect::set(
        detail,
        &"branch".into(),
        &JsValue::from_str(branch.unwrap_or("main")),
    )
    .ok();
    Reflect::set(detail, &"profile".into(), &JsValue::FALSE).ok();
}

/// Dispatch `tonk-claim` on `consumer` with the given structured
/// transact request, await `detail.result`.
pub async fn claim(consumer: &Element, request: &JsValue) -> Result<JsValue, ErrorDetail> {
    claim_with_route(consumer, request, None, None, false).await
}

/// Like [`claim`], but with an explicit cross-repo route (`space`/`branch`/
/// `profile`). See [`query_with_route`] / `apply_route` for the routing
/// semantics. A transact relayed from a sealed guest's `<tonk-repository
/// profile>` context needs the profile flag stamped or it falls back to the
/// bare `/transact` endpoint (405). With `space = None` and `profile = false`
/// this is identical to [`claim`].
pub async fn claim_with_route(
    consumer: &Element,
    request: &JsValue,
    space: Option<&str>,
    branch: Option<&str>,
    profile: bool,
) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"request".into(), request).ok();
    apply_route(&detail, space, branch, profile);
    let result_promise = dispatch_one_shot(consumer, events::CLAIM, &detail)?;
    let result = JsFuture::from(result_promise)
        .await
        .map_err(|e| js_to_error(&e))?;
    Ok(result)
}

/// Dispatch `tonk-evaluate` on `consumer` with the given raw
/// asserted-notation text, await `detail.result`.
///
/// `transact` controls the worker's commit step (mirrors the
/// `/evaluate?transact=` query). Pass `false` to project what the
/// document *would* do — queries run, mutations are dropped — so a
/// half-typed buffer can preview results without committing. Pass
/// `true` for an explicit submit that should land.
pub async fn evaluate(
    consumer: &Element,
    document: &str,
    transact: bool,
) -> Result<JsValue, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"document".into(), &JsValue::from_str(document)).ok();
    Reflect::set(&detail, &"transact".into(), &JsValue::from_bool(transact)).ok();
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
    subscribe_with_route(consumer, query_body, tag, None, None, false)
}

/// [`subscribe`], retrying while the host handshake is not established yet.
///
/// A subscription is installed by dispatching a DOM event some host's
/// document-level listener must claim, and boots race: a guest coming
/// up while its host installs — or while a service-worker swap restarts
/// everything — can dispatch into silence or reach a host that claims
/// the event before it writes the subscription handle. Either one-shot
/// failure left the view subscribed to NOTHING. That is the wedge a
/// reload "fixed": the element sat on its loading state forever while a
/// calmer boot subscribed fine. Bounded, so a genuinely hostless or
/// persistently incomplete host still fails, loudly, after a few seconds.
pub async fn subscribe_claimed(
    consumer: &Element,
    query_body: &JsValue,
    tag: Option<&JsValue>,
) -> Result<Subscription, ErrorDetail> {
    subscribe_claimed_with_route(consumer, query_body, tag, None, None, false).await
}

/// [`subscribe_with_route`], with the same claim-retry as
/// [`subscribe_claimed`].
pub async fn subscribe_claimed_with_route(
    consumer: &Element,
    query_body: &JsValue,
    tag: Option<&JsValue>,
    space: Option<&str>,
    branch: Option<&str>,
    profile: bool,
) -> Result<Subscription, ErrorDetail> {
    let mut establishment_error = None;
    for attempt in 0..12u32 {
        if attempt > 0 {
            crate::ops::wait_ms(250 * attempt.min(4) as i32).await;
        }
        match subscribe_with_route(consumer, query_body, tag, space, branch, profile) {
            Ok(subscription) => return Ok(subscription),
            Err(error) if is_establishment_error(&error) => establishment_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(establishment_error
        .unwrap_or_else(|| ErrorDetail::new(ErrorKind::Network, "tonk-subscribe: never claimed")))
}

fn is_establishment_error(error: &ErrorDetail) -> bool {
    error.message.contains("no host claimed")
        || error
            .message
            .contains("host did not write detail.subscription")
}

/// Like [`subscribe`], but with an explicit cross-repo route (`space`/`branch`).
/// See [`query_with_route`] / `apply_route` for the routing semantics. With
/// `space = None` this is identical to [`subscribe`].
pub fn subscribe_with_route(
    consumer: &Element,
    query_body: &JsValue,
    tag: Option<&JsValue>,
    space: Option<&str>,
    branch: Option<&str>,
    profile: bool,
) -> Result<Subscription, ErrorDetail> {
    let detail = Object::new();
    Reflect::set(&detail, &"query".into(), query_body).ok();
    if let Some(t) = tag {
        Reflect::set(&detail, &"tag".into(), t).ok();
    }
    apply_route(&detail, space, branch, profile);
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
            format!(
                "tonk-subscribe: no host claimed the event ({})",
                dispatch_diagnostics(consumer)
            ),
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
            format!(
                "{event_name}: no host claimed the event ({})",
                dispatch_diagnostics(consumer)
            ),
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

/// Diagnostic snapshot for an unclaimed dispatch. An event only reaches the
/// document listeners from a CONNECTED element (`connected=false` means the
/// consumer dispatched from a detached tree — the event bubbled to its
/// detached root and died there); `root` names that root
/// (`#document` / `#document-fragment` / an element tag).
fn dispatch_diagnostics(consumer: &Element) -> String {
    format!(
        "consumer=<{}> connected={} root={}",
        consumer.local_name(),
        consumer.is_connected(),
        consumer.get_root_node().node_name().to_lowercase(),
    )
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_limits_subscription_establishment_retries_to_boot_races() {
        assert!(is_establishment_error(&ErrorDetail::new(
            ErrorKind::Network,
            "tonk-subscribe: no host claimed the event",
        )));
        assert!(is_establishment_error(&ErrorDetail::new(
            ErrorKind::Network,
            "tonk-subscribe: host did not write detail.subscription",
        )));
        assert!(!is_establishment_error(&ErrorDetail::new(
            ErrorKind::Network,
            "tonk-subscribe: rejected by repository authorization",
        )));
    }

    /// A host can claim the event before it has written the subscription
    /// handle. If that is a transient boot race, the claimed helper must make
    /// another establishment attempt rather than leaving the consumer with no
    /// live subscription.
    #[dialog_common::test]
    async fn it_retries_when_a_claimed_subscription_omits_its_handle_once() {
        let document = window().expect("window").document().expect("document");
        let container = document.create_element("div").expect("container");
        let consumer = document
            .create_element("tonk-test-consumer")
            .expect("consumer");
        container.append_child(&consumer).expect("append consumer");
        document
            .body()
            .expect("body")
            .append_child(&container)
            .expect("attach container");

        let attempts = Rc::new(Cell::new(0u32));
        let cancels = Rc::new(Cell::new(0u32));
        let cancel_slot = Rc::new(RefCell::new(None::<Closure<dyn FnMut()>>));

        let attempts_for_listener = attempts.clone();
        let cancels_for_listener = cancels.clone();
        let cancel_slot_for_listener = cancel_slot.clone();
        let listener = Closure::wrap(Box::new(move |event: CustomEvent| {
            event.stop_propagation();
            event.prevent_default();
            let attempt = attempts_for_listener.get() + 1;
            attempts_for_listener.set(attempt);

            if attempt == 1 {
                return;
            }

            let detail: Object = event.detail().dyn_into().expect("detail object");
            let subscription = Object::new();
            let cancels = cancels_for_listener.clone();
            let cancel = Closure::wrap(Box::new(move || {
                cancels.set(cancels.get() + 1);
            }) as Box<dyn FnMut()>);
            Reflect::set(&subscription, &"cancel".into(), cancel.as_ref()).expect("install cancel");
            Reflect::set(&detail, &"subscription".into(), &subscription)
                .expect("install subscription");
            *cancel_slot_for_listener.borrow_mut() = Some(cancel);
        }) as Box<dyn FnMut(CustomEvent)>);
        container
            .add_event_listener_with_callback(events::SUBSCRIBE, listener.as_ref().unchecked_ref())
            .expect("listen");

        let query = Object::new();
        let subscription = subscribe_claimed(&consumer, query.as_ref(), None)
            .await
            .expect("the second establishment attempt should succeed");

        assert_eq!(attempts.get(), 2, "one retry should establish the handle");
        assert_eq!(cancels.get(), 0, "the live handle is not canceled early");
        drop(subscription);
        assert_eq!(cancels.get(), 1, "dropping the handle cancels it once");

        container.remove();
    }
}
