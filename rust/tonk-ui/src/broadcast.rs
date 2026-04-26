//! Thin wrapper over [`BroadcastChannel`] for listening to worker
//! notifications.
//!
//! The worker posts on named channels (one per endpoint) whenever
//! the data backing that endpoint changes. A [`Subscription`] owns
//! one open channel plus the closure that receives its messages;
//! drop it to close.
//!
//! This module is framework-agnostic — Leptos bridging lives in
//! [`watch`](crate::watch).
//!
//! [`BroadcastChannel`]: https://developer.mozilla.org/en-US/docs/Web/API/BroadcastChannel

use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{BroadcastChannel, MessageEvent};

/// Reasons [`Subscription::new`] can fail.
#[derive(Debug, Error)]
pub enum SubscribeError {
    /// The browser refused to open the channel. In practice this
    /// means the runtime doesn't expose `BroadcastChannel` (very old
    /// browsers, certain worker contexts); the `JsValue` is the raw
    /// exception for diagnostics.
    #[error("failed to open BroadcastChannel: {0:?}")]
    OpenFailed(JsValue),
}

/// An active subscription to a `BroadcastChannel`. Closes the
/// channel and drops the listener closure when dropped.
///
/// `BroadcastChannel` and `Closure` are both `!Send`, and so is
/// this type.
pub struct Subscription {
    channel: BroadcastChannel,
    // Kept alive so the JS-side callback stays valid for the
    // channel's lifetime.
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

impl Subscription {
    /// Open a `BroadcastChannel` on `name` and invoke `callback`
    /// for every message received. The subscription stays alive
    /// for as long as the returned handle is held; dropping the
    /// handle closes the channel.
    pub fn new<F>(name: &str, mut callback: F) -> Result<Self, SubscribeError>
    where
        F: FnMut(MessageEvent) + 'static,
    {
        let channel = BroadcastChannel::new(name).map_err(SubscribeError::OpenFailed)?;

        let closure = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            callback(event);
        });
        channel.set_onmessage(Some(closure.as_ref().unchecked_ref()));

        Ok(Self {
            channel,
            _on_message: closure,
        })
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // Clear the handler first so any late message delivered
        // before the browser finalizes the close doesn't invoke a
        // just-dropped closure.
        self.channel.set_onmessage(None);
        self.channel.close();
    }
}
