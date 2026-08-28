//! Subscribing from `connectedCallback` must survive being called detached.
//!
//! The custom-element reaction queue delivers `connectedCallback` to an
//! element that is not yet in the document. The host listens on
//! `document`, so a `tonk-subscribe` dispatched from a detached element
//! reaches nothing and comes back
//! `no host claimed the event (connected=false)`.
//!
//! Nothing fails loudly. The element registers, its callback runs, and
//! it simply never hears an answer — which is how the bar's account
//! subscription came to report stale state: it never received a single
//! frame, so the share row went on offering "log in to share" to an
//! account that was already active, and clicking share waited for a link
//! that was never coming.
//!
//! The fix is always the same shape: defer a microtask, then check
//! `is_connected` before subscribing. This pins that every subscribing
//! element does it, by connecting one the way the browser does and
//! asserting the subscription is open afterwards.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::window;

wasm_bindgen_test_configure!(run_in_browser);

/// Elements that open a subscription when they connect.
/// The tag the bar's account watch subscribes under.
const ACCOUNT_SUB_TAG: &str = "fabb-activation";

const SUBSCRIBING_TAGS: &[&str] = &[
    "tonk-share",
    "ui-space-name",
    "ui-profile-name",
    "ui-member-roster",
    "ui-space-switcher",
];

/// Connect `tag` the way the browser does — construct, then insert —
/// and count the `tonk-subscribe` events that reached the document.
///
/// Counting at the document is the whole point: that is where the host
/// listens, and an event dispatched from a detached element never
/// arrives there. A subscribe that fires while detached does not throw
/// either — it returns an `ErrorDetail` the element logs and swallows —
/// so "did the element stay connected" proves nothing. What proves it is
/// the host hearing the question at all.
async fn subscribes_after_connecting(tag: &str) -> Result<usize, String> {
    let window = window().expect("window");
    let document = window.document().expect("document");

    let element = document
        .create_element(tag)
        .map_err(|error| format!("create <{tag}>: {error:?}"))?
        .dyn_into::<web_sys::HtmlElement>()
        .map_err(|_| format!("<{tag}> is not an HtmlElement"))?;
    // Every one of these derives its routing context from a space.
    element
        .set_attribute("space", "did:key:z6MkTestSpace")
        .map_err(|error| format!("set space on <{tag}>: {error:?}"))?;

    // Count what reaches the document, before the element is inserted.
    let seen = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let counter = seen.clone();
    let listener = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |_: web_sys::Event| counter.set(counter.get() + 1),
    );
    document
        .add_event_listener_with_callback("tonk-subscribe", listener.as_ref().unchecked_ref())
        .map_err(|error| format!("listen for tonk-subscribe: {error:?}"))?;

    let body = document.body().expect("body");
    body.append_child(&element)
        .map_err(|error| format!("append <{tag}>: {error:?}"))?;

    // Let the deferred subscribe run. Several turns, because the
    // scaffolding defers through `spawn_local` and the element may
    // resolve its routing context first.
    for _ in 0..8 {
        let promise = js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED);
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|error| format!("await a microtask: {error:?}"))?;
    }

    let _ = document
        .remove_event_listener_with_callback("tonk-subscribe", listener.as_ref().unchecked_ref());
    let _ = body.remove_child(&element);
    Ok(seen.get())
}

/// Connecting a subscribing element must reach the host.
#[dialog_common::test]
async fn it_subscribes_only_once_the_element_is_in_the_document() {
    tonk_fab::register();

    for tag in SUBSCRIBING_TAGS {
        let reached = subscribes_after_connecting(tag)
            .await
            .unwrap_or_else(|error| panic!("<{tag}> failed to connect: {error}"));
        assert!(
            reached > 0,
            "<{tag}> subscribed where the host could not hear it — the event never \
             reached the document, which is what `connected=false` means",
        );
    }
}

/// The bar itself subscribes for the account state on connect.
///
/// `<tonk-fab>` is not on the list above — it uses no `subscribing`
/// scaffolding, and its account watch was written by hand, which is why
/// it was the one that subscribed detached.
#[dialog_common::test]
async fn it_connects_the_bar_without_a_detached_subscribe() {
    tonk_fab::register();

    let window = window().expect("window");
    let document = window.document().expect("document");
    // Count only the ACCOUNT subscription, by its tag. The bar has other
    // subscribing children that fire their own, so a bare count is
    // non-zero whether or not this one reached the host — which is how
    // the first version of this test passed against the bug it was
    // written for.
    let seen = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let counter = seen.clone();
    let listener = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |event: web_sys::Event| {
            let Some(custom) = event.dyn_ref::<web_sys::CustomEvent>() else {
                return;
            };
            let tag = js_sys::Reflect::get(&custom.detail(), &"tag".into())
                .ok()
                .and_then(|tag| tag.as_string())
                .unwrap_or_default();
            if tag == ACCOUNT_SUB_TAG {
                counter.set(counter.get() + 1);
            }
        },
    );
    document
        .add_event_listener_with_callback("tonk-subscribe", listener.as_ref().unchecked_ref())
        .expect("listen for tonk-subscribe");

    let bar = document
        .create_element("tonk-fab")
        .expect("create <tonk-fab>")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    bar.set_attribute("space", "did:key:z6MkTestSpace")
        .expect("set space");

    let body = document.body().expect("body");
    body.append_child(&bar).expect("append <tonk-fab>");

    for _ in 0..8 {
        let promise = js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED);
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .expect("await a microtask");
    }

    let _ = document
        .remove_event_listener_with_callback("tonk-subscribe", listener.as_ref().unchecked_ref());
    assert!(
        seen.get() > 0,
        "the bar's account watch subscribed while detached — the host never heard \
         it, so the share row went on offering \"log in to share\" to an account \
         that was already active",
    );
    let _ = body.remove_child(&bar);
}
