#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

//! The registry against a real worker — no fakes in the path.
//!
//! Its own test BINARY, not a module of the lib tests: it installs the
//! real `tonk-host` on the document, which claims every consumer event
//! in the realm. Sharing a page with tests that stub their own host
//! would mean answering their queries too — the same hijack the fake
//! host had to be taught to avoid, except the real host is supposed to
//! answer everything. wasm-bindgen gives each test binary its own page,
//! so the isolation is structural rather than negotiated.
//!
//! Every other test of this module supplies its own answers to the
//! registry's queries, which proves the DOM half but takes the wire
//! shapes on trust. Here nothing is stubbed between the rendered tag and
//! the branch:
//!
//! - the real `tonk-host` IO surface, dispatching consumer events and
//!   reading back `detail.result` / `detail.subscription`;
//! - a real `fetch`, carrying real `Request`/`Response` pairs through
//!   the same browser<->axum conversion the service worker runs
//!   (`tonk_worker::helpers::serve`);
//! - a real worker router over a real `TonkState` — IndexedDB storage,
//!   a profile, a repository;
//! - the real evaluate pipeline seeding the element from the same
//!   asserted notation an author would write, and the real query engine
//!   answering both of the registry's hops.
//!
//! What it cannot include is a service worker: a DOM test has no way to
//! install one. The router runs in-page instead — the same code, over
//! the same interface, one process boundary short.

use tower::ServiceExt as _;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Document, Element, window};

use tonk_worker::helpers::state::test_state;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> Document {
    window().expect("window").document().expect("document")
}

/// Boot a real worker, serve it from `fetch`, install the real host and
/// the registry, and return a container carrying the routing context for
/// the repository that was created.
async fn boot() -> Element {
    let state = test_state().await;
    let (app, _lsp) = tonk_worker::api_router(state);

    // Create the repository the test works in, through the router's own
    // lifecycle route.
    let created = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/repository/probe")
                .method("PUT")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    assert_eq!(
        created.status(),
        axum::http::StatusCode::CREATED,
        "PUT /api/repository/probe should create it",
    );
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .expect("body reads");
    let info: serde_json::Value = serde_json::from_slice(&body).expect("body is JSON");
    let space = info
        .get("name")
        .and_then(serde_json::Value::as_str)
        .expect("the response names the repository")
        .to_owned();

    tonk_worker::helpers::serve::install_fetch(app);
    // The real host's IO surface. `install_io` rather than `install`:
    // the top-page extras (navigate provider, idle-sync heartbeat) are
    // not under test and reach for a page this has not got.
    tonk_host::install_io();
    tonk_display::registry::install();

    // Routing context. The host resolves `with` off the consumer's
    // ancestors, so everything the test renders goes inside this.
    let container = document().create_element("div").expect("create container");
    container
        .set_attribute("with", &format!("main@{space}"))
        .expect("set with");
    document()
        .body()
        .expect("body")
        .append_child(&container)
        .expect("attach container");
    container
}

/// The standard library, seeded here the way the service worker seeds
/// it at repository creation — by evaluating the document. The worker
/// fetches it from `/library/core.yaml`; there is no such asset in a
/// test page, so it is embedded and handed to the same pipeline.
///
/// Seeded in full rather than reduced to the `element` concept: a
/// branch a real element lands on has the whole library on it, and a
/// declaration that only works in isolation is not worth passing.
const STANDARD_LIBRARY: &str = include_str!("../../tonk-core/assets/library/core.yaml");

/// Seed the branch by evaluating asserted notation, the way `tonk` does.
async fn evaluate(container: &Element, source: &str) {
    tonk_host::consumer::evaluate(container, source, true)
        .await
        .expect("the evaluate pipeline accepts the document");
}

async fn settle_until(done: impl Fn() -> bool) {
    for _ in 0..600 {
        if done() {
            return;
        }
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

const TALLY: &str = r#"element!: &tally-widget
  name: "tally-widget"
  method:
    connected: |
      (self) => { self.textContent = `count ${self.getAttribute('count') ?? 0}`; }
"#;

/// An element authored on a real branch registers a tag rendered in the
/// page, with nothing stubbed in between.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_registers_an_element_from_a_real_branch() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;
    evaluate(&container, TALLY).await;

    let host = document()
        .create_element("tally-widget")
        .expect("create tally-widget");
    let _ = host.set_attribute("count", "4");
    container.append_child(&host).expect("attach");

    settle_until(|| host.text_content().as_deref() == Some("count 4")).await;
    assert_eq!(
        host.text_content().as_deref(),
        Some("count 4"),
        "the element should have been resolved off the branch and upgraded",
    );
    assert!(
        !window()
            .expect("window")
            .custom_elements()
            .get("tally-widget")
            .is_undefined(),
    );
}

/// Rendered before anything defines it, defined afterwards on the real
/// branch, picked up with nothing re-rendered — driven by a real
/// subscription frame rather than a pushed one.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_picks_up_a_real_definition_that_arrives_later() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;

    let host = document()
        .create_element("late-widget")
        .expect("create late-widget");
    container.append_child(&host).expect("attach");
    // Give the registry time to look it up and find nothing.
    settle_until(|| {
        document()
            .query_selector("tonk-element-watch[data-tag=\"late-widget\"]")
            .ok()
            .flatten()
            .is_some()
    })
    .await;
    assert!(
        window()
            .expect("window")
            .custom_elements()
            .get("late-widget")
            .is_undefined(),
        "nothing defines it yet",
    );

    evaluate(
        &container,
        r#"element!: &late-widget
  name: "late-widget"
  method:
    connected: |
      (self) => { self.textContent = 'arrived'; }
"#,
    )
    .await;

    settle_until(|| host.text_content().as_deref() == Some("arrived")).await;
    assert_eq!(
        host.text_content().as_deref(),
        Some("arrived"),
        "the live name subscription should have carried the new definition",
    );
}

/// Re-authoring an element on the real branch replaces the
/// implementation for an instance already on the page.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_swaps_a_real_definition_for_live_instances() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;
    evaluate(
        &container,
        r#"element!: &swap-widget
  name: "swap-widget"
  method:
    connected: |
      (self) => { self.textContent = 'v1'; }
"#,
    )
    .await;

    let host = document()
        .create_element("swap-widget")
        .expect("create swap-widget");
    container.append_child(&host).expect("attach");
    settle_until(|| host.text_content().as_deref() == Some("v1")).await;
    assert_eq!(host.text_content().as_deref(), Some("v1"));
    let constructor = window()
        .expect("window")
        .custom_elements()
        .get("swap-widget");

    // Author `connected` alone: the other methods, and the entity, stay
    // put — which is the whole reason the method dictionary is keyed.
    evaluate(
        &container,
        r#"element!: &swap-widget
  name: "swap-widget"
  method:
    connected: |
      (self) => { self.textContent = 'v2'; }
"#,
    )
    .await;

    settle_until(|| host.text_content().as_deref() == Some("v2")).await;
    assert_eq!(
        host.text_content().as_deref(),
        Some("v2"),
        "the live method subscription should have carried the edit",
    );
    assert_eq!(
        window()
            .expect("window")
            .custom_elements()
            .get("swap-widget"),
        constructor,
        "the tag must not have been re-registered",
    );
}
