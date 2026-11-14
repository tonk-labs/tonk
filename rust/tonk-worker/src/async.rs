/// Asynchronously sleep for the specified duration.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: web_time::Duration) {
    tokio::time::sleep(duration).await;
}

/// Asynchronously sleep for the specified duration, on WASI's monotonic
/// clock.
#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: web_time::Duration) {
    dialog_common::time::sleep(duration).await;
}

/// A future in a component on WASI, which has no threads: nothing can send
/// it to another thread, so it is `Send` whatever it holds. axum asks each
/// handler for a `Send` future, and Dialog DB's futures are not `Send` on
/// `wasm32` (see `tonk_macros::handler`).
#[cfg(target_arch = "wasm32")]
pub struct Unthreaded<F>(pub F);

// SAFETY: `wasm32-wasip3` has no threads. A component instance runs on one,
// so no value is ever reached from another.
#[cfg(target_arch = "wasm32")]
unsafe impl<F> Send for Unthreaded<F> {}

#[cfg(target_arch = "wasm32")]
impl<F: std::future::Future> std::future::Future for Unthreaded<F> {
    type Output = F::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: the inner future is never moved out of a pinned wrapper.
        unsafe { self.map_unchecked_mut(|unthreaded| &mut unthreaded.0) }.poll(context)
    }
}

/// Run `work` without waiting for it, as a task of the component's own
/// async runtime on WASI.
#[cfg(target_arch = "wasm32")]
pub fn detach<F>(work: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    wit_bindgen::spawn_local(work);
}

/// Run `work` without waiting for it (non-wasm).
///
/// Requires `Send` because Tokio's scheduler may move the future between
/// worker threads; the wasm arm is single-threaded and does not.
#[cfg(not(target_arch = "wasm32"))]
pub fn detach<F>(work: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn(work);
}
