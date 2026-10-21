//! A bar stamped as an observer after it painted says so on its disc.
//!
//! The member roster learns whether the viewer is a member only once the
//! repository answers, so `data-observer` lands on a bar that has already
//! painted its disc. The custom-elements glue hands the callback a missing
//! old value as `""`, so an attribute added empty arrives with the old
//! value equal to the new one. A bar that skipped such a change as a
//! repeat kept the disc it painted before, `offline` for a signed-out
//! visitor, until some other attribute happened to change after it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

/// Let the element-upgrade reactions run before reading the subtree.
async fn settle() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        window()
            .expect("window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .expect("set timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("the timeout resolves");
}

fn disc(fab: &HtmlElement) -> String {
    fab.shadow_root()
        .expect("the bar has painted")
        .query_selector(".fab .disc")
        .expect("valid selector")
        .expect("the bar has a disc")
        .class_name()
}

#[dialog_common::test]
async fn an_observer_stamped_after_the_paint_shows_on_the_disc() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    fab.set_attribute("with", "main@profile:tonk")
        .expect("routing context");
    fab.set_attribute("space", "did:key:z6MkObserverDisc")
        .expect("space");
    // A visitor with no account: the bar paints its disc offline.
    fab.set_attribute("data-account-required", "")
        .expect("signed out");
    document
        .body()
        .expect("body")
        .append_child(&fab)
        .expect("mount the bar");
    settle().await;
    assert!(
        disc(&fab).split(' ').any(|class| class == "offline"),
        "a signed-out bar paints its disc offline: {}",
        disc(&fab)
    );

    // What the roster does once the repository says the viewer is no member.
    fab.toggle_attribute_with_force("data-observer", true)
        .expect("stamp the observer");
    settle().await;
    assert!(
        disc(&fab).split(' ').any(|class| class == "observing"),
        "the disc says observing once the bar is stamped as an observer: {}",
        disc(&fab)
    );

    fab.remove();
}
