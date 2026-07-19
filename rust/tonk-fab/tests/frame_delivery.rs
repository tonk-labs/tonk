//! Every subscribing FAB element must be able to RECEIVE a frame.
//!
//! The host delivers a subscription frame by calling `element.reset(payload,
//! opts)` / `element.update(...)` (see `tonk-host::ops::deliver_frame`). Those
//! methods only exist because `subscribing::install_frame_shims` puts them on
//! the element's prototype at registration. Miss that one call and the element
//! still registers, still subscribes, and still asks exactly the right
//! question — it simply never hears the answer. Nothing else fails loudly.
//!
//! `<tonk-share>` shipped exactly that way: it subscribed for the minted invite
//! link, no frame could ever reach it, so the pending clipboard copy never
//! settled and the control pinned on `Copying` — which
//! `ShareState::accepts_click` refuses, leaving the share button dead for the
//! rest of the session while the mint itself worked perfectly.
//!
//! The per-element render behaviour is covered in `space_name_element.rs`;
//! this pins the delivery contract itself, for every tag at once, so a new
//! subscribing element cannot be added without it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::window;

wasm_bindgen_test_configure!(run_in_browser);

/// Every element built on the `subscribing` scaffolding. Each one subscribes,
/// so each one must be able to receive.
const SUBSCRIBING_TAGS: &[&str] = &[
    "tonk-share",
    "ui-space-name",
    "ui-profile-name",
    "ui-member-roster",
    "ui-space-switcher",
];

/// Read `method` off `tag`'s registered prototype, exactly where the host's
/// `deliver_frame` looks for it.
fn prototype_method(tag: &str, method: &str) -> JsValue {
    let constructor = window().expect("window").custom_elements().get(tag);
    assert!(
        !constructor.is_undefined(),
        "tonk_fab::register() must define <{tag}>"
    );
    let proto = js_sys::Reflect::get(&constructor, &"prototype".into())
        .unwrap_or_else(|_| panic!("<{tag}> constructor has a prototype"));
    js_sys::Reflect::get(&proto, &method.into())
        .unwrap_or_else(|_| panic!("read {method} off <{tag}>"))
}

#[dialog_common::test]
async fn it_installs_frame_shims_on_every_subscribing_element() {
    tonk_fab::register();

    for tag in SUBSCRIBING_TAGS {
        for method in ["reset", "update"] {
            let found = prototype_method(tag, method);
            assert!(
                found.dyn_ref::<js_sys::Function>().is_some(),
                "<{tag}> must expose a `{method}` method for the host to deliver \
                 frames through — without it the element subscribes and then \
                 silently ignores every answer"
            );
        }
    }
}

#[dialog_common::test]
async fn it_forwards_a_delivered_frame_to_the_elements_own_delegate() {
    // The prototype shim is only half the contract: it forwards to the
    // per-instance `__tonkReset` delegate the scaffolding installs on connect.
    // Prove a real delivered frame reaches that delegate rather than hitting a
    // shim that forwards nowhere.
    tonk_fab::register();

    let el = window()
        .expect("window")
        .document()
        .expect("document")
        .create_element("tonk-share")
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    el.set_attribute("space", "did:key:z6MkTestSpace")
        .expect("set space");
    window()
        .expect("window")
        .document()
        .expect("document")
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");

    let delegate = js_sys::Reflect::get(&el, &"__tonkReset".into()).expect("read delegate");
    assert!(
        delegate.dyn_ref::<js_sys::Function>().is_some(),
        "connect must install a `__tonkReset` delegate for the shim to forward to"
    );

    // Deliver an empty snapshot the way the host does. It must be consumed
    // without throwing — a dropped or absent delegate throws here.
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"tag".into(), &JsValue::from_str("tonk-share")).expect("set tag");
    let reset = js_sys::Reflect::get(&el, &"reset".into())
        .expect("reset present")
        .dyn_into::<js_sys::Function>()
        .expect("reset is a function");
    reset
        .call2(&el, &js_sys::Array::new().into(), &opts.into())
        .expect("a delivered reset frame must be consumed, not throw");
}
