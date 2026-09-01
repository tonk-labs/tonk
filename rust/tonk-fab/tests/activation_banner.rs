//! Pending customer activation in a real browser DOM.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

async fn yield_for(ms: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        window()
            .expect("window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("timeout resolves");
}

async fn wait_for(selector: &str) -> Element {
    for _ in 0..100 {
        if let Ok(Some(element)) = window()
            .expect("window")
            .document()
            .expect("document")
            .query_selector(selector)
        {
            return element;
        }
        yield_for(10).await;
    }
    panic!("{selector} did not mount");
}

/// Deliver a subscription frame the way the host does: call the
/// element's own `reset` with the rows the query answered.
///
/// The condition is driven by a subscription now, not by a fetch. A
/// stubbed `fetch` used to be how this test put the bar into
/// "Registered", and stubbing it today changes nothing at all — the
/// read it stood in for is gone.
/// Deliver a frame the way the two subscriptions do.
///
/// `status` selects which fact the frame carries: a registration frame
/// names the address, an activation frame names the provider. There is no
/// status string on the wire any more — the element latches whichever
/// facts have arrived and renders from the pair.
fn deliver(bar: &HtmlElement, status: &str, email: &str) {
    let fields = Object::new();
    if status == "Active" {
        Reflect::set(
            &fields,
            &"provider".into(),
            &"https://service.example/ucan/".into(),
        )
        .expect("provider");
        Reflect::set(&fields, &"activated_at".into(), &JsValue::from_f64(1.0)).expect("at");
    } else {
        Reflect::set(&fields, &"email".into(), &email.into()).expect("email");
        Reflect::set(&fields, &"registered_at".into(), &JsValue::from_f64(1.0)).expect("at");
    }
    let row = Object::new();
    Reflect::set(&row, &"fields".into(), &fields).expect("fields");
    let rows = js_sys::Array::new();
    rows.push(&row);

    let reset = Reflect::get(bar.as_ref(), &"reset".into()).expect("reset");
    let reset: Function = reset.dyn_into().expect("reset is callable");
    reset
        .call2(bar.as_ref(), &rows.into(), &JsValue::UNDEFINED)
        .expect("frame delivered");
}

fn response(body: &str) -> JsValue {
    let constructor = Reflect::get(&window().expect("window"), &"Response".into())
        .expect("Response")
        .dyn_into::<Function>()
        .expect("Response constructor");
    let init = Object::new();
    Reflect::set(&init, &"status".into(), &JsValue::from_f64(200.0)).expect("status");
    let args = js_sys::Array::new();
    args.push(&body.into());
    args.push(&init);
    Reflect::construct(&constructor, &args).expect("response")
}

#[dialog_common::test]
async fn registered_customer_can_resend_and_retires_when_active() {
    let win = window().expect("window");
    let document = win.document().expect("document");
    let original_fetch = Reflect::get(&win, &"fetch".into()).expect("original fetch");
    let status = Rc::new(RefCell::new("Registered".to_owned()));
    let posts = Rc::new(Cell::new(0_u32));
    let status_for_fetch = status.clone();
    let posts_for_fetch = posts.clone();
    let fetch = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(
        move |url: JsValue, init: JsValue| {
            let url = url.as_string().unwrap_or_default();
            let method = Reflect::get(&init, &"method".into())
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_else(|| "GET".to_owned());
            let body = if url == "/api/customer/enroll" && method == "POST" {
                posts_for_fetch.set(posts_for_fetch.get() + 1);
                r#"{"status":"Registered"}"#.to_owned()
            } else {
                format!(
                    r#"{{"customer":"did:key:zAccount","status":"{}","email":"jack@example.test"}}"#,
                    status_for_fetch.borrow()
                )
            };
            Promise::resolve(&response(&body))
        },
    );
    Reflect::set(&win, &"fetch".into(), fetch.as_ref()).expect("stub fetch");

    tonk_fab::register();
    let bar = document
        .create_element("tonk-fab")
        .expect("bar")
        .dyn_into::<HtmlElement>()
        .expect("html bar");
    bar.set_attribute("space", "did:key:zSpace").expect("space");
    document
        .body()
        .expect("body")
        .append_child(&bar)
        .expect("mount");

    // The condition arrives as a subscription frame. `watch` wires
    // `reset`/`update` onto the element from `connectedCallback`, so
    // give that a turn before delivering one.
    yield_for(20).await;
    deliver(&bar, "Registered", "jack@example.test");

    let banner = wait_for("#fabb-activation-banner").await;
    assert!(banner.text_content().unwrap_or_default().contains(
        "jack@example.test is waiting for email confirmation — nothing syncs until you confirm it"
    ));
    banner
        .shadow_root()
        .expect("banner shadow")
        .query_selector(".door")
        .expect("door selector")
        .expect("door")
        .dyn_into::<HtmlElement>()
        .expect("door html")
        .click();

    let cluster = wait_for("#fabb-activation-cluster").await;
    let field = cluster
        .query_selector("[data-activation-email]")
        .expect("field selector")
        .expect("email field");
    assert_eq!(
        field.get_attribute("value").as_deref(),
        Some("jack@example.test")
    );
    cluster
        .query_selector("[data-resend-activation]")
        .expect("resend selector")
        .expect("resend")
        .dyn_into::<HtmlElement>()
        .expect("resend html")
        .click();
    yield_for(20).await;
    assert_eq!(posts.get(), 1);
    assert!(
        cluster
            .query_selector("[data-activation-narrator]")
            .expect("narrator selector")
            .expect("narrator")
            .text_content()
            .unwrap_or_default()
            .starts_with("Sent")
    );

    // Activating retires the condition on its own. There is no "check
    // now" button any more: the answer is a fact this element is
    // subscribed to, so it arrives rather than being asked for.
    deliver(&bar, "Active", "jack@example.test");
    yield_for(20).await;
    assert!(
        document
            .get_element_by_id("fabb-activation-cluster")
            .is_none()
    );
    yield_for(180).await;
    assert!(
        document
            .get_element_by_id("fabb-activation-banner")
            .is_none()
    );
    let share = bar
        .query_selector("[data-share-link]")
        .expect("share selector")
        .expect("share row");
    assert!(!share.has_attribute("data-activation-blocked"));

    bar.remove();
    if let Some(cluster) = document.get_element_by_id("fabb-connect-cluster") {
        cluster.remove();
    }
    if let Some(join) = document.get_element_by_id("fab-join-first") {
        join.remove();
    }
    Reflect::set(&win, &"fetch".into(), &original_fetch).expect("restore fetch");
    drop(fetch);
}
