//! Thin helper over [`BroadcastChannel`] for notifying UIs that the
//! representation of an endpoint changed.
//!
//! The convention is "channel name == endpoint path". Any code that
//! commits to data backing an endpoint calls [`broadcast`] with that
//! endpoint's path; UIs open a `BroadcastChannel` on the same path to
//! receive those notifications (cross-tab included).
//!
//! Messages are sent as JSON strings so listeners across languages
//! can parse them with a single `JSON.parse` and so the payload is
//! trivially inspectable in devtools.
//!
//! On native the call is a no-op — the helper exists for structural
//! parity so commit sites don't need their own `cfg`s.
//!
//! [`BroadcastChannel`]: https://developer.mozilla.org/en-US/docs/Web/API/BroadcastChannel

use dialog_repository::Revision;
use serde::{Deserialize, Serialize};

/// A change announcement posted on the endpoint's broadcast channel.
///
/// Carries the branch that was committed to and the [`Revision`]
/// published by that commit, so listeners can decide whether to
/// refetch. Fields are owned so the same type can be used on both
/// sides of the channel — `postMessage` structured-clones the
/// payload either way, so holding refs would only postpone a copy
/// that has to happen anyway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// The branch whose commit prompted this notification.
    pub branch: String,
    /// The revision produced by the commit.
    pub revision: Revision,
}

/// Post `message` on the broadcast channel named `channel`.
///
/// Serialization or browser-side errors are logged but not
/// propagated — a failed notification can only cause a missed UI
/// refresh, never a data integrity problem, and the caller has
/// already committed its work.
pub fn broadcast(channel: &str, message: &Notification) {
    #[cfg(target_arch = "wasm32")]
    wasm::broadcast(channel, message);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (channel, message);
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::Notification;
    use tonk_common::log;
    use wasm_bindgen::JsValue;
    use web_sys::BroadcastChannel;

    pub(super) fn broadcast(channel: &str, message: &Notification) {
        let payload = match serde_json::to_string(message) {
            Ok(text) => text,
            Err(err) => {
                log!("Failed to serialize broadcast payload: {err}");
                return;
            }
        };
        let bc = match BroadcastChannel::new(channel) {
            Ok(bc) => bc,
            Err(err) => {
                log!("Failed to open BroadcastChannel '{channel}': {err:?}");
                return;
            }
        };
        if let Err(err) = bc.post_message(&JsValue::from_str(&payload)) {
            log!("Failed to post broadcast on '{channel}': {err:?}");
        }
        bc.close();
    }
}
