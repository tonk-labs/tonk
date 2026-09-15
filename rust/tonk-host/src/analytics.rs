//! Closed transport for privacy-safe analytics from sealed guests.

use js_sys::{JSON, Object, Reflect};
use wasm_bindgen::JsValue;
use web_sys::{CustomEvent, CustomEventInit, window};

/// Carry an already-validated event to the top page.
///
/// Each sealed parent forwards the same JSON envelope. Only the top-level UI
/// owns the PostHog sink and re-validates typed event properties before capture.
pub fn capture(name: &str, properties: &serde_json::Value) {
    let envelope = serde_json::json!({ "name": name, "props": properties }).to_string();
    relay(&envelope);
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Array, Function};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn clear_tonk() {
        if let Some(win) = window() {
            let _ = Reflect::delete_property(&win, &"tonk".into());
        }
    }

    #[dialog_common::test]
    async fn guest_relay_forwards_the_unchanged_envelope() {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        Reflect::set(
            &tonk,
            &"analytics".into(),
            recorder.as_ref().unchecked_ref::<Function>(),
        )
        .unwrap();
        Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();
        let envelope = r#"{"name":"product_event","props":{"schema_version":1}}"#;
        relay(envelope);
        assert_eq!(calls.length(), 1);
        assert_eq!(calls.get(0).as_string().as_deref(), Some(envelope));
        clear_tonk();
        recorder.forget();
    }

    #[dialog_common::test]
    async fn top_relay_dispatches_one_structured_event() {
        clear_tonk();
        let seen = Rc::new(RefCell::new(None));
        let listener = {
            let seen = seen.clone();
            Closure::wrap(Box::new(move |event: CustomEvent| {
                *seen.borrow_mut() = Some(event.detail());
            }) as Box<dyn FnMut(CustomEvent)>)
        };
        let win = window().unwrap();
        win.add_event_listener_with_callback("tonk:analytics", listener.as_ref().unchecked_ref())
            .unwrap();
        relay(r#"{"name":"product_event","props":{"schema_version":1}}"#);
        let detail = seen.borrow().clone().expect("one event dispatched");
        assert_eq!(
            Reflect::get(&detail, &"name".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("product_event")
        );
        assert_eq!(
            Reflect::get(
                &Reflect::get(&detail, &"props".into()).unwrap(),
                &"schema_version".into()
            )
            .unwrap()
            .as_f64(),
            Some(1.0)
        );
        win.remove_event_listener_with_callback(
            "tonk:analytics",
            listener.as_ref().unchecked_ref(),
        )
        .unwrap();
    }
}

/// Relay one analytics envelope from a child portal toward the page.
pub fn relay(envelope: &str) {
    if crate::page_effect::forward("analytics", envelope) {
        return;
    }
    let Ok(value) = JSON::parse(envelope) else {
        return;
    };
    let Some(name) = Reflect::get(&value, &"name".into())
        .ok()
        .and_then(|value| value.as_string())
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let Ok(props) = Reflect::get(&value, &"props".into()) else {
        return;
    };
    if !props.is_object() {
        return;
    }
    let detail = Object::new();
    let _ = Reflect::set(&detail, &"name".into(), &JsValue::from_str(&name));
    let _ = Reflect::set(&detail, &"props".into(), &props);
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    if let Ok(event) = CustomEvent::new_with_event_init_dict("tonk:analytics", &init)
        && let Some(win) = window()
    {
        let _ = win.dispatch_event(&event);
    }
}
