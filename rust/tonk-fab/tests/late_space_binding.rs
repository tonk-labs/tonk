//! A bar that learns its space late hands it to every child it authored.
//!
//! The space route renders `<tonk-fab space={id}>`, and the first
//! projection can land before `{id}` resolves — the bar's own
//! `attributeChangedCallback` exists for exactly that, and re-stamps the
//! subtree when the DID arrives. The subtree is authored ONCE, so a
//! child the re-stamp skips stays pointed at nothing for the life of the
//! page.
//!
//! `<tonk-share>` was skipped. With `space=""` it opens no invite
//! subscription (an empty subject is a query error, not a wildcard) and
//! its click handler returns before dispatching, so picking "copy link"
//! from the share stack did nothing at all: no mint, no spinner, no
//! refusal to explain it. Nothing failed loudly, which is why a green
//! suite went on shipping it.
//!
//! This pins the whole table rather than that one child, because the
//! failure is the table drifting, not the element.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

const SPACE: &str = "did:key:z6MkLateBinding";

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

#[dialog_common::test]
async fn a_late_space_reaches_every_child_the_bar_authored() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    // No `space`: the unsubstituted first projection, which is the only
    // state this test is about.
    fab.set_attribute("with", "main@profile:tonk")
        .expect("routing context");
    document
        .body()
        .expect("body")
        .append_child(&fab)
        .expect("mount the bar");
    settle().await;

    for &(selector, attribute, prefix) in tonk_fab::markup::SPACE_BINDINGS {
        let child = fab
            .query_selector(selector)
            .expect("valid selector")
            .unwrap_or_else(|| panic!("the bar must author <{selector}>"));
        assert_eq!(
            child.get_attribute(attribute).as_deref(),
            Some(prefix),
            "<{selector}> starts bound to nothing",
        );
    }

    // The route resolves `{id}`.
    fab.set_attribute("space", SPACE).expect("late space");
    settle().await;

    for &(selector, attribute, prefix) in tonk_fab::markup::SPACE_BINDINGS {
        let child = fab
            .query_selector(selector)
            .expect("valid selector")
            .unwrap_or_else(|| panic!("the bar must author <{selector}>"));
        assert_eq!(
            child.get_attribute(attribute).as_deref(),
            Some(format!("{prefix}{SPACE}").as_str()),
            "<{selector}> must be re-stamped when the space lands",
        );
    }

    fab.remove();
}

/// A `latent` bar is on a page with no space: it authors only what serves
/// the profile (`<ui-site-request>`), none of the children addressed to a
/// space. What decides is `latent`, not a missing `space`, which the test
/// above shows is only a space that has not arrived yet.
#[dialog_common::test]
async fn a_latent_bar_authors_no_child_addressed_to_a_space() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    fab.set_attribute("with", "main@profile:tonk")
        .expect("routing context");
    fab.set_attribute("latent", "").expect("latent");
    document
        .body()
        .expect("body")
        .append_child(&fab)
        .expect("mount the bar");
    settle().await;

    for &(selector, _, _) in tonk_fab::markup::SPACE_BINDINGS {
        assert!(
            fab.query_selector(selector)
                .expect("valid selector")
                .is_none(),
            "a latent bar must not author <{selector}>",
        );
    }
    assert!(
        fab.query_selector("ui-site-request")
            .expect("valid selector")
            .is_some(),
        "a latent bar still performs the profile's site requests",
    );

    fab.remove();
}
