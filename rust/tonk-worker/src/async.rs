/// Asynchronously sleep for the specified duration.
///
/// This function creates a JavaScript Promise that resolves after the given
/// duration using `setTimeout`, then converts it to a Rust Future.
///
/// # Examples
///
/// ```no_run
/// use tonk_worker::sleep;
/// use web_time::Duration;
///
/// async fn example() {
///     sleep(Duration::from_secs(1)).await;
/// }
/// ```
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn sleep(duration: web_time::Duration) -> Result<(), wasm_bindgen::JsError> {
    use wasm_bindgen::{JsCast, JsValue};

    let global = js_sys::global();
    let set_timeout = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout"))
        .map_err(|error| wasm_bindgen::JsError::new(&format!("{error:?}")))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| wasm_bindgen::JsError::new("global setTimeout is not a function"))?;
    let millis = JsValue::from_f64(duration.as_millis() as f64);

    let promise = js_sys::Promise::new(&mut move |resolve, reject| {
        if let Err(error) = set_timeout.call2(&global, &resolve, &millis) {
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });

    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|error| wasm_bindgen::JsError::new(&format!("{:?}", error)))
}

/// Asynchronously sleep for the specified duration (non-wasm placeholder).
///
/// This is a placeholder implementation for non-wasm targets. The worker
/// has no use case for being used in non-wasm contexts at this time. It
/// cannot fail; the error type matches the wasm implementation so both
/// targets present one signature.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: web_time::Duration) -> Result<(), wasm_bindgen::JsError> {
    tokio::time::sleep(duration).await;
    Ok(())
}

/// Asynchronously sleep for the specified duration, on WASI's monotonic
/// clock. It cannot fail; the error type matches the browser's.
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub async fn sleep(duration: web_time::Duration) -> Result<(), wasm_bindgen::JsError> {
    dialog_common::time::sleep(duration).await;
    Ok(())
}

/// A future in a component on WASI, which has no threads: nothing can send
/// it to another thread, so it is `Send` whatever it holds. axum asks each
/// handler for a `Send` future, and Dialog DB's futures are not `Send` on
/// `wasm32` (see `tonk_macros::handler`).
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub struct Unthreaded<F>(pub F);

// SAFETY: `wasm32-wasip3` has no threads. A component instance runs on one,
// so no value is ever reached from another.
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
unsafe impl<F> Send for Unthreaded<F> {}

#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
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

/// Run `work` without waiting for it, so a caller that only needs it
/// STARTED can return.
///
/// One signature on both targets, per the target-agnostic rule: the
/// browser hands the future to the microtask queue, and native drives it
/// on the Tokio runtime. Neither returns a handle -- detached work is
/// work whose completion nothing observes, and a caller that needs the
/// outcome should await it instead.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn detach<F>(work: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(work);
}

/// Run `work` without waiting for it, as a task of the component's own
/// async runtime on WASI.
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
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
