//! `window.tonk.palette`: the parse, exposed to author elements.
//!
//! Installed by the sealed guest at start, next to the rest of
//! `window.tonk`. An `element!:` calls `window.tonk.palette.parse(request)`
//! synchronously on every keystroke with the rows its subscriptions
//! hold, and gets proposals back; see [`crate::propose`].

use js_sys::{Function, Object, Reflect};
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::{Request, propose};

/// Parse one request. Errors (a malformed request) come back as an
/// exception on the JS side.
fn parse(request: JsValue) -> Result<JsValue, JsValue> {
    let request: Request = serde_wasm_bindgen::from_value(request)?;
    let proposals = propose(&request);
    proposals
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(JsValue::from)
}

/// Install `window.tonk.palette.parse`. Idempotent; a no-op when there
/// is no `window.tonk` to extend.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(tonk) = Reflect::get(&window, &JsValue::from_str("tonk")) else {
        return;
    };
    if !tonk.is_object() {
        return;
    }
    let palette = Object::new();
    let function: Function = Closure::<dyn Fn(JsValue) -> Result<JsValue, JsValue>>::new(parse)
        .into_js_value()
        .unchecked_into();
    let _ = Reflect::set(&palette, &JsValue::from_str("parse"), &function);
    let _ = Reflect::set(&tonk, &JsValue::from_str("palette"), &palette);
}
