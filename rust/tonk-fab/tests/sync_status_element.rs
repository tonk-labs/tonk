//! `<ui-sync-status>` in a real DOM.
//!
//! The disc's contract is `with="branch@repo"`. The FAB authors it before
//! its space is known, as `with="main@"`, and stamps the space a moment
//! later. A disc whose own `with` is not yet a location must wait for that
//! stamp rather than dispatch a subscription the host can only refuse.
//!
//! No host is installed, so nothing answers the event: these pin what the
//! ELEMENT dispatches.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, window};

wasm_bindgen_test_configure!(run_in_browser);

const SPACE: &str = "main@did:key:z6MkTestSpace";

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
}

/// Yield to the event loop for `ms` milliseconds.
async fn yield_for(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        window()
            .expect("window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("set_timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("timeout resolves");
}

/// Records the `with` of every `<ui-sync-status>` that dispatches
/// `tonk-subscribe`, until dropped.
struct Subscribes {
    seen: Rc<RefCell<Vec<Option<String>>>>,
    listener: Closure<dyn FnMut(web_sys::Event)>,
}

impl Subscribes {
    fn record() -> Self {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            if target.local_name() == "ui-sync-status" {
                log.borrow_mut().push(target.get_attribute("with"));
            }
        });
        document()
            .add_event_listener_with_callback("tonk-subscribe", listener.as_ref().unchecked_ref())
            .expect("listener installs");
        Self { seen, listener }
    }

    /// The `with` of each subscribe dispatched so far. With no host to
    /// answer, a disc retries on its next callback, so the same location
    /// can appear more than once.
    fn seen(&self) -> Vec<Option<String>> {
        self.seen.borrow().clone()
    }

    /// Whether every subscribe so far named `with`, and at least one did.
    fn only(&self, with: &str) -> bool {
        let seen = self.seen();
        !seen.is_empty() && seen.iter().all(|seen| seen.as_deref() == Some(with))
    }
}

impl Drop for Subscribes {
    fn drop(&mut self) {
        let _ = document().remove_event_listener_with_callback(
            "tonk-subscribe",
            self.listener.as_ref().unchecked_ref(),
        );
    }
}

/// Mount a `<ui-sync-status>` with the given `with`, as the FAB authors it.
fn mount(with: &str) -> Element {
    tonk_fab::register();
    let disc = document().create_element("ui-sync-status").expect("create");
    disc.set_attribute("with", with).expect("set with");
    document()
        .body()
        .expect("body")
        .append_child(&disc)
        .expect("append");
    disc
}

#[dialog_common::test]
async fn it_waits_for_a_space_before_subscribing() {
    let subscribes = Subscribes::record();
    let disc = mount("main@");
    yield_for(20).await;
    assert_eq!(
        subscribes.seen(),
        Vec::<Option<String>>::new(),
        "a disc with no space yet has nothing to subscribe to"
    );

    disc.set_attribute("with", SPACE).expect("stamp the space");
    yield_for(20).await;
    assert!(subscribes.only(SPACE), "{:?}", subscribes.seen());
    disc.remove();
}

#[dialog_common::test]
async fn it_waits_for_a_placeholder_to_be_stamped() {
    let subscribes = Subscribes::record();
    let disc = mount("main@{space}");
    yield_for(20).await;
    assert_eq!(subscribes.seen(), Vec::<Option<String>>::new());
    disc.remove();
}

#[dialog_common::test]
async fn it_subscribes_at_once_when_addressed() {
    let subscribes = Subscribes::record();
    let disc = mount(SPACE);
    yield_for(20).await;
    assert!(subscribes.only(SPACE), "{:?}", subscribes.seen());
    disc.remove();
}
