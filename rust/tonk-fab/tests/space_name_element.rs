//! `<ui-space-name>` in a real DOM.
//!
//! The FAB reads a space's name from that space's own branch without any
//! seeded view. That rests on three behaviours no native test can reach: the
//! element registers, it stamps its OWN routing context (`resolve_with` never
//! walks ancestors), and it dispatches a subscribe carrying a RAW ATTRIBUTE
//! query — naming a concept would reintroduce the frozen-descriptor
//! dependency the whole design removes.
//!
//! No host is installed here, so nothing answers the event and no frame
//! arrives. That is deliberate: this pins what the ELEMENT does. Host
//! delivery is proven in production by `<ui-sync-status>`.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, window};

wasm_bindgen_test_configure!(run_in_browser);

const SPACE: &str = "did:key:z6MkTestSpace";

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
}

/// Yield to the event loop for `ms` milliseconds using the native
/// `setTimeout`/`Promise` bridge already available through this crate's
/// `web-sys`/`js-sys`/`wasm-bindgen-futures` dependencies — no extra
/// third-party crate needed just to await a tick.
async fn yield_for(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = window().expect("window");
        win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("set_timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("timeout resolves");
}

/// Mount a `<ui-space-name space=SPACE>` and return it.
fn mount() -> web_sys::HtmlElement {
    tonk_fab::register();
    let el = document()
        .create_element("ui-space-name")
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    el.set_attribute("space", SPACE).expect("set space");
    document()
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");
    el
}

#[dialog_common::test]
async fn it_registers_as_a_custom_element() {
    tonk_fab::register();
    let defined = window()
        .expect("window")
        .custom_elements()
        .get("ui-space-name");
    assert!(
        !defined.is_undefined(),
        "tonk_fab::register() must define <ui-space-name>"
    );
}

#[dialog_common::test]
async fn it_stamps_its_own_routing_context_from_the_space_attribute() {
    let el = mount();
    // `resolve_with` reads THIS element's own `with` and never walks
    // ancestors, so the element must stamp it itself — unlike
    // <ui-sync-status>, which receives `with` from a view template.
    assert_eq!(
        el.get_attribute("with").as_deref(),
        Some("main@did:key:z6MkTestSpace"),
        "element must stamp its own with= from `space`"
    );
}

#[dialog_common::test]
async fn it_dispatches_a_subscribe_carrying_the_raw_attribute_query() {
    // Capture `tonk-subscribe` before mounting; it bubbles and is composed,
    // so a document-level listener sees it.
    let seen: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = seen.clone();
    let cb = Closure::<dyn FnMut(CustomEvent)>::new(move |ev: CustomEvent| {
        let detail = ev.detail();
        let json = js_sys::JSON::stringify(&detail)
            .map(|s| String::from(s))
            .unwrap_or_default();
        *sink.borrow_mut() = Some(json);
    });
    document()
        .add_event_listener_with_callback("tonk-subscribe", cb.as_ref().unchecked_ref())
        .expect("listen");

    mount();

    // The element subscribes from a spawn_local on connect; yield to let it run.
    yield_for(50).await;

    let captured = seen.borrow().clone();
    let detail = captured.expect("element must dispatch tonk-subscribe on connect");

    // The RAW attribute URI — nothing seeded is consulted.
    assert!(
        detail.contains("xyz.tonk.repo/name"),
        "subscribe must query the raw attribute: {detail}"
    );
    // Naming a concept would reintroduce the frozen-descriptor dependency.
    assert!(
        !detail.contains("tonk:repository"),
        "subscribe must NOT name a concept: {detail}"
    );
    // Bound to this space's subject.
    assert!(detail.contains(SPACE), "subscribe must bind the subject: {detail}");

    drop(cb);
}
