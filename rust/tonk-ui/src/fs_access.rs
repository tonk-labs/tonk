//! Thin wrappers around the File System Access API + the page-to-SW
//! postMessage handshake.
//!
//! `web_sys` provides typed bindings for `showDirectoryPicker`,
//! `queryPermission`, and `requestPermission`; this module narrows
//! their `Promise`-returning shapes into plain `async fn`s that return
//! crate-specific result types, and adds a one-call helper for
//! handing a directory handle to the active service worker.
//!
//! All helpers run in the page (Leptos `csr` build), not the SW. The
//! SW receives handles via the `message` listener in
//! `assets/service_worker.js`, which forwards to
//! `worker.registerFsHandle(id, handle)`.

use crate::error::TonkUiError;
use js_sys::{Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemHandlePermissionDescriptor, FileSystemPermissionMode,
    PermissionState,
};

fn js_error(context: &str, error: JsValue) -> TonkUiError {
    let message = error
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("{error:?}"));
    TonkUiError::other(format!("{context}: {message}"))
}

/// Prompt the user to pick a directory.
///
/// Returns `Ok(None)` when the user cancels the picker; any other
/// failure (FS Access unsupported, transient JS error, etc.) bubbles
/// up as `Err`.
pub async fn show_directory_picker() -> Result<Option<FileSystemDirectoryHandle>, TonkUiError> {
    let window = web_sys::window().ok_or_else(|| TonkUiError::other("window unavailable"))?;
    let promise = window
        .show_directory_picker()
        .map_err(|e| js_error("showDirectoryPicker (unsupported?)", e))?;
    match JsFuture::from(promise).await {
        Ok(value) => {
            let handle: FileSystemDirectoryHandle = value
                .dyn_into()
                .map_err(|_| TonkUiError::other("showDirectoryPicker returned non-handle"))?;
            Ok(Some(handle))
        }
        Err(error) => {
            // Cancellation surfaces as an AbortError DOMException — distinguish
            // from real failures so the UI can stay quiet on cancel.
            if is_abort_error(&error) {
                Ok(None)
            } else {
                Err(js_error("showDirectoryPicker", error))
            }
        }
    }
}

/// Query the current readwrite permission for a handle without
/// prompting the user.
pub async fn query_readwrite_permission(
    handle: &FileSystemDirectoryHandle,
) -> Result<PermissionState, TonkUiError> {
    let descriptor = readwrite_descriptor();
    let value = JsFuture::from(handle.query_permission_with_descriptor(&descriptor))
        .await
        .map_err(|e| js_error("queryPermission", e))?;
    permission_state_from_js(&value)
}

/// Prompt the user to grant readwrite permission for a handle. Must
/// be invoked from a user-gesture handler (click, keypress, etc.) —
/// the browser silently rejects otherwise.
pub async fn request_readwrite_permission(
    handle: &FileSystemDirectoryHandle,
) -> Result<PermissionState, TonkUiError> {
    let descriptor = readwrite_descriptor();
    let value = JsFuture::from(handle.request_permission_with_descriptor(&descriptor))
        .await
        .map_err(|e| js_error("requestPermission", e))?;
    permission_state_from_js(&value)
}

/// Send a handle to the active service worker so the worker stashes
/// it in `dialog-remote-fs`'s thread-local registry under `id`. The
/// SW shim listens for `{type: "register-fs-handle", id, handle}` and
/// forwards to `worker.registerFsHandle`.
pub fn register_handle_with_worker(
    id: &str,
    handle: &FileSystemDirectoryHandle,
) -> Result<(), TonkUiError> {
    let message = Object::new();
    set(&message, "type", &JsValue::from_str("register-fs-handle"))?;
    set(&message, "id", &JsValue::from_str(id))?;
    set(&message, "handle", handle.as_ref())?;
    post_to_worker(&message)
}

/// Tell the active service worker to forget the handle previously
/// registered under `id`.
pub fn unregister_handle_with_worker(id: &str) -> Result<(), TonkUiError> {
    let message = Object::new();
    set(&message, "type", &JsValue::from_str("unregister-fs-handle"))?;
    set(&message, "id", &JsValue::from_str(id))?;
    post_to_worker(&message)
}

fn readwrite_descriptor() -> FileSystemHandlePermissionDescriptor {
    let descriptor = FileSystemHandlePermissionDescriptor::new();
    descriptor.set_mode(FileSystemPermissionMode::Readwrite);
    descriptor
}

fn permission_state_from_js(value: &JsValue) -> Result<PermissionState, TonkUiError> {
    // `query/requestPermission` resolve with a string ("granted" |
    // "prompt" | "denied"); web_sys exposes `PermissionState` as a
    // string enum that decodes via `wasm_bindgen::JsCast::dyn_into`.
    let state = value
        .as_string()
        .ok_or_else(|| TonkUiError::other("permission state was not a string"))?;
    match state.as_str() {
        "granted" => Ok(PermissionState::Granted),
        "prompt" => Ok(PermissionState::Prompt),
        "denied" => Ok(PermissionState::Denied),
        other => Err(TonkUiError::other(format!(
            "unexpected permission state '{other}'",
        ))),
    }
}

fn is_abort_error(error: &JsValue) -> bool {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|e| {
            let name = String::from(e.name());
            name == "AbortError"
        })
        .unwrap_or(false)
}

fn post_to_worker(message: &Object) -> Result<(), TonkUiError> {
    let window = web_sys::window().ok_or_else(|| TonkUiError::other("window unavailable"))?;
    let container = window.navigator().service_worker();
    let controller = container.controller().ok_or_else(|| {
        TonkUiError::other("no active service worker controller — page must reload to acquire one")
    })?;
    controller
        .post_message(message.as_ref())
        .map_err(|e| js_error("serviceWorker.controller.postMessage", e))
}

fn set(obj: &Object, key: &str, value: &JsValue) -> Result<(), TonkUiError> {
    Reflect::set(obj.as_ref(), &JsValue::from_str(key), value)
        .map_err(|e| js_error(&format!("set {key}"), e))?;
    Ok(())
}
