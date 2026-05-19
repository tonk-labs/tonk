//! Bridge-driven subscription helper.
//!
//! Replaces the previous `fetch()`-based SSE reader. All streaming
//! data now flows through `globalThis.tonk.subscribe`, which the
//! bridge module routes over postMessage to the service worker.

use std::rc::Rc;

use crate::bridge::{SubscribeHandle, subscribe};
use crate::error::{ErrorDetail, ErrorKind};

/// Open a streaming subscription via the postMessage bridge.
///
/// `body` is the wire query to send. `on_frame` is called for each
/// emitted frame (the raw JSON string of a `Vec<Conclusion>`).
/// `on_error` is called on bridge-reported transport errors.
///
/// Returns a [`SubscribeHandle`] that the caller must keep alive;
/// dropping it cancels the subscription.
pub fn open_sse(
    body: &serde_json::Value,
    on_frame: impl Fn(&str) + 'static,
    on_error: impl Fn(ErrorDetail) + 'static,
) -> Result<SubscribeHandle, ErrorDetail> {
    // Wrap on_error in Rc so it can be shared between the two bridge
    // callbacks without cloning the underlying value.
    let on_error = Rc::new(on_error);
    let on_error_for_frame = on_error.clone();

    subscribe(
        body,
        move |frame_value| {
            // Re-serialise to a string so that the call site
            // (which expects `&str`) doesn't need to change shape.
            match serde_json::to_string(&frame_value) {
                Ok(s) => on_frame(&s),
                Err(e) => {
                    on_error_for_frame(ErrorDetail::new(
                        ErrorKind::Parse,
                        format!("frame stringify: {e}"),
                    ));
                }
            }
        },
        move |message| {
            on_error(ErrorDetail::new(ErrorKind::Network, message));
        },
    )
}
