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
    use wasm_bindgen::JsCast;
    use web_sys::ServiceWorkerGlobalScope;

    let millis = duration.as_millis() as i32;

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        // let window = web_sys::window().expect("no global window exists");
        let global = js_sys::global()
            .dyn_into::<ServiceWorkerGlobalScope>()
            .unwrap();

        if let Err(error) =
            global.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
        {
            let _ = reject.call0(&error);
        };
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
