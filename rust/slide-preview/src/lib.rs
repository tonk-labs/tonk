//! The harness: connect back to the slide preview daemon over
//! `/ws/page`, and for each request mount a real `<tonk-view>`
//! with the candidate template, feed it the supplied conclusions,
//! and reply with the serialized rendered HTML.
//!
//! The render path is exactly the production one — same custom
//! element, same snapshot/plan/apply — so the HTML the agent gets
//! back is what `<tonk-display>` would produce.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

/// Register the tonk-display custom elements once. Idempotent —
/// `tonk_display::register` guards re-registration itself.
pub fn ensure_registered() {
    tonk_display::register();
}

/// Wire up the WebSocket client. Called from `main` on page load.
pub fn run() {
    ensure_registered();
    let window = web_sys::window().expect("window");
    let host = window.location().host().expect("location host");
    let socket = WebSocket::new(&format!("ws://{host}/ws/page")).expect("open websocket");

    set_status("connected — waiting for render requests");

    let reply_socket = socket.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(text) = event.data().as_string() else {
            return;
        };
        match handle(&text) {
            Ok(reply) => {
                let _ = reply_socket.send_with_str(&reply);
                set_status("rendered — waiting for the next request");
            }
            Err(err) => {
                web_sys::console::error_1(&err);
                set_status("render failed — see the console");
            }
        }
    });
    socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();
}

/// Handle one daemon envelope: mount, render, serialize, reply.
/// Pure with respect to the socket so the wasm test can drive it
/// directly.
pub fn handle(text: &str) -> Result<String, JsValue> {
    ensure_registered();
    let request: serde_json::Value =
        serde_json::from_str(text).map_err(|e| JsValue::from_str(&format!("bad envelope: {e}")))?;
    let id = request["id"]
        .as_u64()
        .ok_or_else(|| JsValue::from_str("envelope missing id"))?;
    let payload = &request["payload"];
    let template = payload["template"]
        .as_str()
        .ok_or_else(|| JsValue::from_str("payload missing template"))?;
    let conclusions = &payload["conclusions"];

    let document = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let host = document.create_element("tonk-view")?;
    host.set_inner_html(template);
    document
        .body()
        .ok_or_else(|| JsValue::from_str("no body"))?
        .append_child(&host)?; // connected_callback snapshots the template

    let frame = js_sys::JSON::parse(&conclusions.to_string())?;
    let render = js_sys::Reflect::get(&host, &JsValue::from_str("render"))?;
    let render: js_sys::Function = render
        .dyn_into()
        .map_err(|_| JsValue::from_str("<tonk-view> has no render method — registration failed?"))?;
    render.call1(&host, &frame)?;

    let html = host.inner_html();
    host.remove();

    let row_count = conclusions.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "id": id,
        "payload": { "html": html, "row_count": row_count },
    })
    .to_string())
}

fn set_status(text: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("status"))
    {
        element.set_text_content(Some(text));
    }
}
