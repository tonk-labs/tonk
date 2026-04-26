//! Bridge between [`broadcast::Subscription`] and Leptos signals.
//!
//! [`watch`] opens a `BroadcastChannel`, deserializes every message
//! as `T`, and exposes the latest payload as a `ReadSignal`. The
//! subscription's lifetime is tied to the current Leptos owner —
//! when the owner is disposed, the channel closes.
//!
//! Broadcast is best-effort: channel-open failures and payload
//! parse errors are logged but don't bubble up, so callers always
//! get a signal back. A missed message manifests as a momentarily
//! stale UI, never as a crash.
//!
//! [`broadcast::Subscription`]: crate::broadcast::Subscription

use leptos::logging::log;
use leptos::prelude::*;
use send_wrapper::SendWrapper;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::broadcast::Subscription;

/// Open a `BroadcastChannel` on `name` and expose the latest
/// message as `ReadSignal<Option<T>>`. The signal starts as `None`
/// and transitions to `Some(value)` each time a message lands.
///
/// Must be called inside a Leptos owner — the returned signal and
/// the underlying `Subscription` are both bound to the owner's
/// cleanup queue.
pub fn watch<T>(name: &str) -> ReadSignal<Option<T>>
where
    T: DeserializeOwned + Send + Sync + 'static,
{
    let (read, write) = signal(None::<T>);

    let subscription = Subscription::new(name, move |event| {
        let text = match event.data().as_string() {
            Some(text) => text,
            None => {
                log!("Broadcast payload was not a JSON string, ignoring");
                return;
            }
        };
        match serde_json::from_str::<T>(&text) {
            Ok(value) => write.set(Some(value)),
            Err(err) => log!("Failed to parse broadcast payload: {err}"),
        }
    });

    match subscription {
        Ok(sub) => {
            // `Subscription` is `!Send`; wrap it so Leptos's cleanup
            // queue (which requires `Send + Sync`) accepts it.
            // Cleanup runs on the same thread in CSR so the wrapper
            // never observes a cross-thread access.
            let sub = SendWrapper::new(sub);
            on_cleanup(move || drop(sub));
        }
        Err(err) => {
            // Browser-side failure means the signal will never
            // update. We still return a valid signal so callers
            // don't need an error branch — worst case is a stale
            // view that a manual refresh can fix.
            log!(
                "Failed to subscribe to broadcast channel '{name}': {:?}",
                JsValue::from(err.to_string()),
            );
        }
    }

    read
}
