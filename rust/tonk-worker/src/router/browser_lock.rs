//! Profile-scoped browser serialization shared by durable worker effects.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct CrossWorkerGuard(Option<tokio::sync::oneshot::Sender<()>>);

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) struct CrossWorkerGuard;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Drop for CrossWorkerGuard {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

/// Acquire an exact named browser Web Lock.
///
/// Callers intentionally fail closed if this is unavailable: a local mutex
/// cannot serialize two concurrently alive service-worker generations.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn acquire(lock_name: &str) -> Result<CrossWorkerGuard, ()> {
    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

    let global = js_sys::global();
    let navigator =
        js_sys::Reflect::get(&global, &JsValue::from_str("navigator")).map_err(|_| ())?;
    let locks = js_sys::Reflect::get(&navigator, &JsValue::from_str("locks")).map_err(|_| ())?;
    let request: js_sys::Function = js_sys::Reflect::get(&locks, &JsValue::from_str("request"))
        .map_err(|_| ())?
        .dyn_into()
        .map_err(|_| ())?;
    let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let callback = Closure::once_into_js(move |_lock: JsValue| -> js_sys::Promise {
        let _ = acquired_tx.send(());
        future_to_promise(async move {
            let _ = release_rx.await;
            Ok(JsValue::UNDEFINED)
        })
    });
    let request_promise: js_sys::Promise = request
        .call2(&locks, &JsValue::from_str(lock_name), &callback)
        .map_err(|_| ())?
        .dyn_into()
        .map_err(|_| ())?;
    spawn_local(async move {
        let _ = JsFuture::from(request_promise).await;
    });
    acquired_rx.await.map_err(|_| ())?;
    Ok(CrossWorkerGuard(Some(release_tx)))
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn acquire(_lock_name: &str) -> Result<CrossWorkerGuard, ()> {
    Ok(CrossWorkerGuard)
}
