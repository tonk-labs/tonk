//! `window.tonk.palette`: the parse, exposed to author elements.
//!
//! Installed by the sealed guest at start, next to the rest of
//! `window.tonk`. An `element!:` calls `window.tonk.palette.parse(request)`
//! synchronously on every keystroke with the rows its subscriptions
//! hold, and gets proposals back; see [`crate::propose`]. With nothing
//! typed it calls `window.tonk.palette.menu(request)`; see [`crate::menu`].

use js_sys::{Function, Object, Reflect};
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::{Request, menu, propose};

/// Answer one request with `answer`. Errors (a malformed request) come
/// back as an exception on the JS side.
fn call(
    request: JsValue,
    answer: fn(&Request) -> Vec<crate::Proposal>,
) -> Result<JsValue, JsValue> {
    let request: Request = serde_wasm_bindgen::from_value(request)?;
    answer(&request)
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
    for (name, answer) in [
        ("parse", propose as fn(&Request) -> Vec<crate::Proposal>),
        ("menu", menu),
    ] {
        let function: Function =
            Closure::<dyn Fn(JsValue) -> Result<JsValue, JsValue>>::new(move |request| {
                call(request, answer)
            })
            .into_js_value()
            .unchecked_into();
        let _ = Reflect::set(&palette, &JsValue::from_str(name), &function);
    }
    let _ = Reflect::set(&tonk, &JsValue::from_str("palette"), &palette);
}
