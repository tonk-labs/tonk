//! Transport-agnostic SSE subscription.
//!
//! [`open_sse`] selects a transport at runtime — the iframe bridge
//! (`globalThis.tonk`, via [`crate::bridge`]) or a direct `fetch()`
//! SSE reader (via [`crate::http`]) — and drives it through one
//! [`EventSource`] abstraction.
//!
//! Each transport exposes its output as a uniform **frame stream**
//! (`Stream<Item = Result<String, ErrorDetail>>`, one already-framed
//! JSON conclusion batch per item) plus a teardown action. The
//! abstraction owns the single drop-safety invariant:
//!
//! > **Dropping a [`EventSource`] never invokes `on_error`.**
//!
//! Teardown (an unmount or a re-subscribe) is a graceful close, not a
//! transport failure. The reader races the frame stream against a
//! drop signal, so the moment the handle is dropped the reader stops
//! on the drop branch — before any abort-rejected read can reach
//! `on_error`. There is no per-transport error suppression: the rule
//! lives here, once.

use futures::StreamExt as _;
use futures::future::{Either, select};
use futures::stream::LocalBoxStream;
use ipld_core::ipld::Ipld;

use crate::error::{ErrorDetail, ErrorKind};

/// A live subscription. The reader task runs until the frame stream
/// ends, errors, or this handle is dropped. Dropping it stops the
/// reader cleanly (no `on_error`) and runs the transport teardown.
pub struct EventSource {
    /// Held only for its `Drop`: dropping the sender resolves the
    /// reader's drop-signal receiver, so the reader exits on the
    /// drop branch instead of seeing the teardown as a read error.
    _shutdown: futures::channel::oneshot::Sender<()>,
    /// Transport-specific teardown (abort the fetch / cancel the
    /// bridge reader), run on drop alongside the shutdown signal.
    teardown: Option<Box<dyn FnOnce()>>,
}

impl EventSource {
    /// Spawn a reader over `frames`, dispatching each `Ok` frame to
    /// `on_frame` and each `Err` to `on_error`. `teardown` runs when
    /// the returned handle is dropped.
    ///
    /// The reader races `frames.next()` against the drop signal, so a
    /// dropped handle stops the reader on the drop branch — the
    /// teardown-induced read rejection is never observed, hence never
    /// reported. This is the one place the "drop ⇒ no error"
    /// invariant is enforced.
    fn spawn(
        mut frames: LocalBoxStream<'static, Result<String, ErrorDetail>>,
        on_frame: impl Fn(&str) + 'static,
        on_error: impl Fn(ErrorDetail) + 'static,
        on_close: impl Fn() + 'static,
        teardown: impl FnOnce() + 'static,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
        wasm_bindgen_futures::spawn_local(async move {
            // `select` isn't biased: when the handle is dropped and a
            // teardown-induced error become ready in the same tick, it
            // can pick the error arm. So when a stream item wins we
            // re-check the shutdown receiver and let a fired shutdown
            // override the item — a dropped handle never surfaces an
            // error, which is exactly the spurious error this
            // abstraction exists to prevent.
            let mut shutdown = shutdown_rx;
            loop {
                let next = frames.next();
                futures::pin_mut!(next);
                match select(next, &mut shutdown).await {
                    // Shutdown fired — the handle was dropped. Exit
                    // cleanly without touching `on_error`.
                    Either::Right(_) => break,
                    // A stream item resolved first. But the drop and a
                    // teardown-induced error can become ready in the
                    // same tick and `select` isn't biased, so re-check
                    // shutdown before reporting any error: a fired
                    // shutdown always wins.
                    Either::Left((item, shutdown_fut)) => {
                        // Shutdown has fired if the sender was dropped
                        // (`Err(Canceled)`) or signalled (`Ok(Some)`);
                        // only `Ok(None)` means "still live". If it
                        // fired, exit clean regardless of `item`.
                        let shutdown_fired = !matches!(shutdown_fut.try_recv(), Ok(None));
                        match item {
                            _ if shutdown_fired => break,
                            Some(Ok(frame)) => on_frame(&frame),
                            Some(Err(e)) => {
                                on_error(e);
                                break;
                            }
                            // Clean upstream end — the server closed the
                            // stream (e.g. the SW releasing in-flight
                            // streams on update). Not an error, but the
                            // subscription is over: tell the owner so it
                            // can reconnect instead of freezing.
                            None => {
                                on_close();
                                break;
                            }
                        }
                    }
                }
            }
        });
        EventSource {
            _shutdown: shutdown_tx,
            teardown: Some(Box::new(teardown)),
        }
    }
}

impl Drop for EventSource {
    fn drop(&mut self) {
        // Dropping `_shutdown` already signalled the reader to stop;
        // now release the transport (abort fetch / cancel reader).
        if let Some(teardown) = self.teardown.take() {
            teardown();
        }
    }
}

/// Open a streaming subscription over fetch against `url`.
///
/// One transport everywhere: inside a sealed guest, `window.fetch` is
/// the portal bootstrap's override, which relays the request (and
/// streams the response back) through the host — so the same code
/// serves the top document and every guest. (`window.tonk` is app
/// sugar, not the elements' transport.)
///
/// `body` is the query as an [`Ipld`] value, encoded once as DAG-JSON
/// for the request body.
///
/// `on_frame` is called for each emitted frame (the raw JSON string
/// of a `Vec<Conclusion>`); `on_error` for genuine transport errors;
/// `on_close` when the server ends the stream cleanly (the owner
/// should reconnect — a released stream is not a cancelled
/// subscription). Dropping the returned [`EventSource`] tears the
/// stream down without reporting an error or a close.
pub async fn open_sse(
    url: &str,
    body: &Ipld,
    on_frame: impl Fn(&str) + 'static,
    on_error: impl Fn(ErrorDetail) + 'static,
    on_close: impl Fn() + 'static,
) -> Result<EventSource, ErrorDetail> {
    let body_bytes = serde_ipld_dagjson::to_vec(body)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body dag-json: {e}")))?;
    let body_str = String::from_utf8(body_bytes)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body utf8: {e}")))?;
    let (frames, teardown) = crate::http::frame_stream(url, &body_str).await?;
    Ok(EventSource::spawn(
        frames, on_frame, on_error, on_close, teardown,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Dropping an `EventSource` must never surface an error, even
    /// when the underlying frame stream would error on the next read
    /// (as an aborted `fetch` body read does). This is the
    /// regression guard for the spurious "Connection failed /
    /// AbortError" that a tab switch produced when tonk-host aborted
    /// its own subscription.
    #[dialog_common::test]
    async fn it_reports_no_error_when_dropped() {
        use futures::channel::mpsc;

        // A frame stream the test controls. The reader is parked on
        // `recv` (no frame yet) exactly like a live SSE awaiting the
        // next event; then we drop the handle.
        let (tx, rx) = mpsc::unbounded::<Result<String, ErrorDetail>>();
        let frames = rx.boxed_local();

        let errored = Rc::new(Cell::new(false));
        let saw_error = errored.clone();
        let torn_down = Rc::new(Cell::new(false));
        let did_teardown = torn_down.clone();

        let sub = EventSource::spawn(
            frames,
            |_frame| {},
            move |_e| saw_error.set(true),
            || {},
            move || did_teardown.set(true),
        );

        // Let the reader park on the stream, then drop the handle —
        // the teardown-equivalent close.
        flush().await;
        drop(sub);

        // Simulate the transport erroring on the now-abandoned read,
        // the way an aborted fetch rejects. The reader must already
        // be gone via the drop signal, so this is never observed.
        let _ = tx.unbounded_send(Err(ErrorDetail::new(
            ErrorKind::Network,
            "stream read failed: AbortError (simulated)",
        )));
        flush().await;

        assert!(
            !errored.get(),
            "dropping an EventSource must not invoke on_error",
        );
        assert!(torn_down.get(), "drop must run the transport teardown");
    }

    /// Yield to the microtask queue so spawned reader tasks make
    /// progress between steps.
    async fn flush() {
        for _ in 0..4 {
            let (tx, rx) = futures::channel::oneshot::channel::<()>();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = tx.send(());
            });
            let _ = rx.await;
        }
    }
}
