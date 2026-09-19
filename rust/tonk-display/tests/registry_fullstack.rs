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

/// The table library, whose `element!: &tonk-table` is the shell of a
/// real, shipped component authored as branch data.
const TABLE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/table.yaml");

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
  description: "A running tally"
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

/// An element's attribute defaults come off the real branch through
/// their own query and reach the instance before `connected` runs.
///
/// Its own full-stack test because the defaults are a SEPARATE wire
/// query from the methods, and the ways that query can be silently
/// wrong — a `the:` written as an attribute where the schema declares
/// a domain, a keyed collection missing its key operand — all read as
/// "this element declares no defaults" rather than as an error.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_applies_attribute_defaults_from_a_real_branch() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;
    evaluate(
        &container,
        r#"element!: &themed-widget
  description: "Carries a default"
  method:
    connected: |
      (self) => { self.textContent = `tone ${self.getAttribute('tone')}`; }
  attribute:
    tone: "quiet"
"#,
    )
    .await;

    let host = document()
        .create_element("themed-widget")
        .expect("create themed-widget");
    container.append_child(&host).expect("attach");

    settle_until(|| host.text_content().as_deref() == Some("tone quiet")).await;
    assert_eq!(
        host.text_content().as_deref(),
        Some("tone quiet"),
        "the default should have been read off the branch and written \
         on before `connected` ran",
    );
    assert_eq!(
        host.get_attribute("tone").as_deref(),
        Some("quiet"),
        "the default belongs in the DOM, where CSS and getAttribute \
         can both see it",
    );

    // A supplied value still wins, on an instance added afterwards.
    let supplied = document()
        .create_element("themed-widget")
        .expect("create themed-widget");
    let _ = supplied.set_attribute("tone", "loud");
    container.append_child(&supplied).expect("attach");
    settle_until(|| supplied.text_content().as_deref() == Some("tone loud")).await;
    assert_eq!(supplied.get_attribute("tone").as_deref(), Some("loud"));
}

/// An element's getters and setters come off the real branch through
/// their own queries and land as real properties.
///
/// Its own full-stack test for the same reason the defaults have one:
/// each dictionary is a SEPARATE wire query, and the ways one can be
/// silently wrong all read as "this element declares no accessors"
/// rather than as an error. Reading through the write half — the
/// mistake the renderer's struct exists to prevent — would also pass a
/// test that only checked that `el.value` exists.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_installs_accessors_from_a_real_branch() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;
    evaluate(
        &container,
        r#"element!: &counter-widget
  description: "Counts, and says so through a property"
  method:
    connected: |
      (self) => { self.dataset.n = self.dataset.n ?? '0'; }
  getter:
    total: |
      (self) => Number(self.dataset.n ?? 0)
  setter:
    total: |
      (self, next) => { self.dataset.n = String(next); }
"#,
    )
    .await;

    let host = document()
        .create_element("counter-widget")
        .expect("create counter-widget");
    container.append_child(&host).expect("attach");
    settle_until(|| host.get_attribute("data-n").is_some()).await;

    let read = |what: &str| {
        js_sys::Reflect::get(&host, &what.into())
            .ok()
            .and_then(|value| value.as_f64())
    };
    assert_eq!(read("total"), Some(0.0), "the getter should answer");

    let _ = js_sys::Reflect::set(&host, &"total".into(), &7_f64.into());
    assert_eq!(
        host.get_attribute("data-n").as_deref(),
        Some("7"),
        "the setter should have run — a definition that read through          the write half would leave this untouched",
    );
    assert_eq!(read("total"), Some(7.0));
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
  description: "Defined after it was rendered"
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
  description: "Re-authored while live"
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

    // Re-author it. `connected` changed, so this derives a DIFFERENT
    // element and the anchor repoints — the name hop moves and the
    // method hop follows it, which is why the registry watches both.
    evaluate(
        &container,
        r#"element!: &swap-widget
  description: "Re-authored while live"
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

/// `<tonk-table>`'s shell, resolved off a real branch and mounted for
/// real, against a stand-in grid core.
///
/// The point of the port is that the shell — what mounts, what it
/// watches, what it dispatches, what properties it exposes — is now a
/// row rather than a bundled class. So this drives it the way a page
/// does: evaluate `table.yaml`, render the tag, and check the element
/// actually came up.
///
/// The grid CORE stays a built asset (an IronCalc engine and the
/// TypeScript program that drives it), and this substitutes a stand-in
/// for it through `globalThis.__tonkTableGrid` — the same override the
/// portal guest uses to hand the real core across a sealed boundary.
/// What is under test is the shell; the core has its own suite.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn it_mounts_the_table_shell_from_the_table_library() {
    let container = boot().await;
    evaluate(&container, STANDARD_LIBRARY).await;
    evaluate(&container, TABLE_LIBRARY).await;
    install_stub_grid_core();

    let host = document()
        .create_element("tonk-table")
        .expect("create tonk-table");
    host.set_text_content(Some("a,b\n1,2"));
    container.append_child(&host).expect("attach");

    // Mounted means: the shell resolved off the branch, built its
    // shadow root, loaded the core through the override, and handed it
    // a mount point.
    settle_until(|| mounted_csv(&host).is_some()).await;
    assert_eq!(
        mounted_csv(&host).as_deref(),
        Some("a,b\n1,2"),
        "the shell should have read its light-DOM text channel and \
         passed it to the core as the standalone source",
    );

    // The shadow root is the shell's, built in `state` rather than in a
    // constructor it does not have.
    assert!(host.shadow_root().is_some(), "no shadow root was attached");

    // An accessor declared in notation, answering through the live
    // definition — `value` reads the mounted grid, not the attribute.
    let value = js_sys::Reflect::get(&host, &"value".into())
        .ok()
        .and_then(|v| v.as_string());
    assert_eq!(value.as_deref(), Some("a,b\n1,2"), "the `value` getter");

    // `min-rows` / `min-cols` are attribute defaults declared on the
    // element, so they are on the instance although the markup set
    // neither.
    assert_eq!(host.get_attribute("min-rows").as_deref(), Some("100"));
    assert_eq!(host.get_attribute("min-cols").as_deref(), Some("26"));

    // And the shell reported readiness, which is the contract a page
    // waits on.
    assert!(
        js_sys::Reflect::get(&host, &"grid".into())
            .map(|grid| !grid.is_null() && !grid.is_undefined())
            .unwrap_or(false),
        "the `grid` getter should expose the mounted handle",
    );
}

/// The CSV the stand-in core was mounted with, read back off the host.
fn mounted_csv(host: &Element) -> Option<String> {
    let grid = js_sys::Reflect::get(host, &"grid".into()).ok()?;
    if grid.is_null() || grid.is_undefined() {
        return None;
    }
    js_sys::Reflect::get(&grid, &"csv".into()).ok()?.as_string()
}

/// Install a stand-in for the grid core at `globalThis.__tonkTableGrid`.
///
/// A `data:` module URL rather than a served file: the shell imports
/// whatever URL the override resolves to, and a data URL is the one
/// form that needs no server, no bundler and no asset copied into the
/// test fixture. It exports the same surface the real core does —
/// `createGrid`, `shell`, `hostStyles` — with the helpers the shell
/// actually calls on this path.
fn install_stub_grid_core() {
    let module = r#"
        export const hostStyles = ":host { display: block; }";
        export const shell = {
          Clock: class { tick() { return 1n; } receive(h) { return h; } },
          formatHlc: (h) => String(h),
          parseContent: (raw) => ({ hlc: null, contentType: null, value: raw }),
          formatContent: (c) => c.value,
          isWorkbookType: () => false,
          WORKBOOK_TYPE: "application/vnd.ironcalc",
          base64ToBytes: () => new Uint8Array(),
          bytesToBase64: () => "",
          readSheetRows: () => [],
          readCellRows: () => [],
          readColumnRows: () => [],
          readRowSizeRows: () => [],
          toSource: (content) => ({ kind: "csv", csv: content.value }),
        };
        export async function createGrid(parent, options) {
          const csv = options.mode.kind === "standalone" ? options.mode.source.csv : "";
          parent.textContent = csv;
          return {
            csv,
            toCsv: () => csv,
            serialize: () => new Uint8Array(),
            applyRows: () => {},
            load: () => {},
            setReadOnly: () => {},
            setMinExtent: () => {},
            focus: () => {},
            destroy: () => {},
          };
        }
    "#;
    let url = format!(
        "data:text/javascript;charset=utf-8,{}",
        js_sys::encode_uri_component(module)
    );
    let _ = js_sys::Reflect::set(&js_sys::global(), &"__tonkTableGrid".into(), &url.into());
}
