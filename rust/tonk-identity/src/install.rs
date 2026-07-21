//! The `window.tonkIdentity` ceremony hook.
//!
//! WebAuthn only exists on the window, so the ceremony surface installs
//! from the page main thread; the service worker never sees key
//! material. Installed as JS functions (rather than Rust-only API) so
//! WebDriver-driven tests and future non-wasm callers (the CLI linking
//! handoff page) can invoke ceremonies directly.
//!
//! This installs as its own global, `window.tonkIdentity`, and
//! deliberately never touches `window.tonk`. The top page must not carry
//! a `window.tonk` object: tonk-host's page-effect forwarding treats the
//! bare presence of `window.tonk` as the signal that the current document
//! is a portal guest with a bridge to its parent, rather than the page
//! itself. Creating `window.tonk` here would make the top page look like
//! a guest to that check, and every page effect (navigate, set title,
//! open) would silently stop working.

use js_sys::{Object, Promise, Reflect};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

fn js_error(error: anyhow::Error) -> JsValue {
    JsValue::from_str(&format!("{error:#}"))
}

async fn create(name: JsValue) -> Result<JsValue, JsValue> {
    let name = name.as_string().unwrap_or_else(|| "tonk".to_owned());
    let created = crate::passkey::create_passkey(&name)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(
        &result,
        &"credentialId".into(),
        &hex::encode(&created.id).into(),
    )?;
    Reflect::set(
        &result,
        &"prfAtCreate".into(),
        &created.prf_output.is_some().into(),
    )?;
    Ok(result.into())
}

async fn derive_root_did() -> Result<JsValue, JsValue> {
    use dialog_varsig::Principal;
    let prf = crate::passkey::prf_output().await.map_err(js_error)?;
    let signer = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    Ok(JsValue::from_str(&signer.did().to_string()))
}

/// Install `window.tonkIdentity` on the page. Idempotent; a no-op
/// outside a window context.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let identity = Object::new();

    let create_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|name: JsValue| {
        future_to_promise(create(name))
    });
    let _ = Reflect::set(
        &identity,
        &"createPasskey".into(),
        create_passkey.as_ref().unchecked_ref(),
    );
    create_passkey.forget();

    let derive = Closure::<dyn FnMut() -> Promise>::new(|| future_to_promise(derive_root_did()));
    let _ = Reflect::set(
        &identity,
        &"deriveRootDid".into(),
        derive.as_ref().unchecked_ref(),
    );
    derive.forget();

    let _ = Reflect::set(&window, &"tonkIdentity".into(), &identity.into());
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Reflect;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_installs_ceremony_functions_on_window_tonk_identity() {
        install();
        let window = web_sys::window().unwrap();
        let identity = Reflect::get(&window, &"tonkIdentity".into()).unwrap();
        for name in ["createPasskey", "deriveRootDid"] {
            let function = Reflect::get(&identity, &name.into()).unwrap();
            assert!(function.is_function(), "{name} must be a function");
        }
    }
}
