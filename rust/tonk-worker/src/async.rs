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
/// has no use case for being used in non-wasm contexts at this time.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: web_time::Duration) -> Result<(), ()> {
    tokio::time::sleep(duration).await;
    Ok(())
}
