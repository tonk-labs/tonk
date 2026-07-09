# `<tonk-upload>` Web-UI Blob Upload — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a web-UI path to ingest a picked file into the branch's blob store: a worker `POST …/blob` route, and a headless native `<tonk-upload>` element with a nice default UI that ingests → previews → emits the resulting `blob:<hash>`.

**Architecture:** Two parts. (1) A `POST /api/repository/{repo}/branch/{branch}/blob` handler in `tonk-worker` that buffers the body, writes it via `Blob::import`, asserts the `xyz.tonk.blob/*` facts, and returns JSON. (2) A native `<tonk-upload>` custom element (the `custom-elements` crate + `CustomElement` trait, mirroring `<tonk-tree>`) that runs in the sealed guest: it reads the file to bytes, POSTs via the relayed `window.fetch`, swaps its preview to the read route, and dispatches a `tonk-upload` event. Shadow DOM holds the mechanism; the author restyles via slots/parts/state.

**Tech Stack:** Rust; axum-in-wasm (`tonk-worker`); `wasm-bindgen`/`web-sys` + the `custom-elements` crate (`tonk-display`); `dialog-repository::Blob` for the store.

**Spec:** `docs/superpowers/specs/2026-07-09-blob-upload-design.md`.

## Global Constraints

- Always `#[dialog_common::test]` (never `#[test]`/`#[tokio::test]`). Worker route tests: `#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))] mod tests` + `wasm_bindgen_test_configure!(run_in_service_worker)`. Element tests: `run_in_browser`.
- Lint gate is the workspace `nix develop .#ci -c cargo clippy --all-targets --all-features -- -D warnings` **plus** `cargo fmt --all -- --check`. Per-crate clippy without `--all-features` is NOT sufficient.
- No `mod.rs`; no phase/RFC references in code/comments.
- dialog-db dep stays on `branch = "main"` (the checkout with the blob API).
- Route URL shapes, exact: ingest `POST /api/repository/{repo}/branch/{branch}/blob` (no entity segment); read (already exists) `GET /api/repository/{repo}/branch/{branch}/blob/{entity}`.
- Ingest request: raw bytes body; `Content-Type` header = the blob MIME; `X-Tonk-Blob-Name` header = filename (optional).
- Success event: bubbling, composed `CustomEvent('tonk-upload')`, `detail = { blob, contentType, name, size }`. No event on failure.
- Custom elements are defined via the `custom-elements` crate: `impl CustomElement for T { … }` + `T::define("tag")`. Do NOT hand-roll `customElements.define`. Mirror `rust/tonk-tree/src/web.rs`.

---

### Task 1: Worker `POST …/blob` ingest route

**Files:**
- Modify: `rust/tonk-worker/src/router/blob.rs` (add `upload` handler + a `BlobUploadResponse` struct + test)
- Modify: `rust/tonk-worker/src/router.rs:266-273` (add the `post` route)

**Interfaces:**
- Consumes: `super::AppState`, `crate::TonkWorkerError` (`Router`/`NotFound`/`Internal`); the existing `serve` handler's branch-open pattern; `dialog_repository::{Blob, RepositoryExt}`, `dialog_effects::blob::BlobError`, `dialog_artifacts::{Attribute, Value}`, `futures_util::stream`.
- Produces: `pub async fn upload(...) -> Result<Response, TonkWorkerError>` at `POST …/blob`, responding JSON `{ entity, contentType, name, size }`.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `rust/tonk-worker/src/router/blob.rs` (alongside `it_serves_blob_bytes_with_the_asserted_content_type`):

```rust
#[dialog_common::test]
async fn it_uploads_bytes_and_serves_them_back() {
    let tonk = test_state().await;
    let app_state = Arc::new(RwLock::new(tonk));
    let (app, _lsp) = api_router_from_state(app_state.clone());
    let repo = put_repo(&app, "blob-upload").await;

    let payload = b"\x89PNG\r\n\x1a\nupload".to_vec();

    // Upload via the new POST route.
    let up = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/repository/{repo}/branch/main/blob"))
                .method("POST")
                .header("content-type", "image/png")
                .header("x-tonk-blob-name", "shot.png")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(up.status(), StatusCode::OK);
    let body = axum::body::to_bytes(up.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entity = json["entity"].as_str().unwrap().to_string();
    assert!(entity.starts_with("blob:"), "entity is a blob ref: {entity}");
    assert_eq!(json["contentType"], "image/png");
    assert_eq!(json["name"], "shot.png");
    assert_eq!(json["size"], payload.len());

    // The GET route serves the same bytes + Content-Type from the asserted fact.
    let got = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/repository/{repo}/branch/main/blob/{entity}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    assert_eq!(got.headers().get("content-type").unwrap(), "image/png");
    let got_body = axum::body::to_bytes(got.into_body(), usize::MAX).await.unwrap();
    assert_eq!(got_body.as_ref(), payload.as_slice());

    // Idempotent: same bytes → same entity.
    let up2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/repository/{repo}/branch/main/blob"))
                .method("POST")
                .header("content-type", "image/png")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body2 = axum::body::to_bytes(up2.into_body(), usize::MAX).await.unwrap();
    let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(json2["entity"], entity, "content-addressed: re-upload yields same entity");

    // Empty body → 400.
    let empty = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/repository/{repo}/branch/main/blob"))
                .method("POST")
                .header("content-type", "image/png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run it and confirm it fails**

Run: `nix develop .#ci -c bash -c 'cargo nextest run -p tonk-worker -E "test(it_uploads_bytes)" 2>&1 | tail -20'` (or the full `nix develop -c test:web:debug`).
Expected: FAIL — the crate doesn't compile (`blob::upload`, the route, don't exist) or the POST 404s.

- [ ] **Step 3: Add the handler**

In `rust/tonk-worker/src/router/blob.rs`, add above the test module. Reuse the existing imports; add `axum::http::HeaderMap`, `dialog_artifacts::{Attribute, Value}`, `futures_util::stream`, `serde::Serialize` as needed.

```rust
/// JSON body of a successful `POST …/blob`.
#[derive(Debug, Serialize)]
pub struct BlobUploadResponse {
    /// The content-addressed `blob:<hash>` entity the bytes were stored under.
    pub entity: String,
    /// MIME type recorded for the blob (from the request `Content-Type`).
    #[serde(rename = "contentType")]
    pub content_type: String,
    /// File name recorded for the blob, if the `X-Tonk-Blob-Name` header was set.
    pub name: Option<String>,
    /// Size of the stored bytes.
    pub size: usize,
}

/// Handler for `POST /api/repository/{repo}/branch/{branch}/blob`.
///
/// Buffers the request body, writes it into the branch's content-addressed
/// blob store, and asserts the blob's `xyz.tonk.blob/content-type` (and
/// `xyz.tonk.blob/name`, when the `X-Tonk-Blob-Name` header is present)
/// facts — so the read route and `<tonk-display model=tonk:blob>` work
/// immediately. Idempotent by content address. Buffered, not streamed.
#[wasm_compat]
pub async fn upload(
    State(state): State<AppState>,
    Path((repo, branch)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, TonkWorkerError> {
    if body.is_empty() {
        return Err(TonkWorkerError::Router("empty upload body".to_string()));
    }
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();
    let name = headers
        .get("x-tonk-blob-name")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let size = body.len();

    let tonk = state.read().await;
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("Repository '{repo}' not found: {e}")))?;
    let branch_handle = repository
        .branch(branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open branch '{branch}': {e}")))?;

    // Ingest bytes into the content-addressed store.
    let bytes = body.to_vec();
    let source = stream::once(async move { Ok::<_, BlobError>(bytes) });
    let entity = Blob::import(source)
        .write(branch_handle.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("write blob: {e}")))?;

    // Assert extrinsic metadata as ordinary facts on the blob entity, mirroring
    // `tonk blob add`. `RawClaim` + reactor commit so subscriptions re-poll.
    let ct_attr: Attribute = "xyz.tonk.blob/content-type"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
    let mut tx = branch_handle
        .transaction()
        .assert(crate::router::claim::RawClaim {
            of: entity.clone(),
            the: ct_attr,
            is: Value::String(content_type.clone()),
        });
    if let Some(n) = &name {
        let name_attr: Attribute = "xyz.tonk.blob/name"
            .parse()
            .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
        tx = tx.assert(crate::router::claim::RawClaim {
            of: entity.clone(),
            the: name_attr,
            is: Value::String(n.clone()),
        });
    }
    tx.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("assert metadata: {e}")))?;

    let payload = BlobUploadResponse {
        entity: entity.to_string(),
        content_type,
        name,
        size,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|e| TonkWorkerError::Internal(format!("serialize: {e}")))?;
    let mut response = (StatusCode::OK, json).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    Ok(response)
}
```

> **Implementer note:** the exact `transaction().assert(...)` / `RawClaim` shape and the `commit().perform(...)` call must match the reactor API used by `router/claim.rs::assert_claim` (`RawClaim` is defined at `router/claim.rs:96-118`) and the CLI's `rust/tonk-cli/src/blob.rs::add` (which asserts the same two attributes via `ContentType::of(entity).is(...)`). If `branch_handle.transaction()` isn't the right entry point in the worker (the worker commits through the reactor session, not a bare branch), use the same commit path `assert_claim`/`transfer::import` use and adapt — the *behaviour* (assert content-type[+name], then broadcast) is the contract, not this exact call. Read `router/claim.rs:206-286` and `router/transfer.rs:114-158` before writing this, and follow whichever commit+broadcast path they use.

- [ ] **Step 4: Register the route**

In `rust/tonk-worker/src/router.rs`, change the blob route block (currently only `get(blob::serve)` at ~line 270) so the collection path also takes a POST:

```rust
        // Content-addressed blob bytes: GET serves an entity's bytes; POST
        // ingests a new blob into the branch store and returns its ref.
        .route(
            "/api/repository/{repo}/branch/{branch}/blob",
            post(blob::upload),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/blob/{entity}",
            get(blob::serve),
        )
```

Ensure `post` is imported (the file already imports `get`; add `post` to the `use ::axum::{…, routing::post}` group if absent).

- [ ] **Step 5: Run the test — confirm pass**

Run: `nix develop .#ci -c bash -c 'cargo nextest run -p tonk-worker -E "test(it_uploads_bytes)" 2>&1 | tail -20'`
Expected: PASS.

- [ ] **Step 6: Lint gate**

Run: `nix develop .#ci -c cargo clippy -p tonk-worker --all-targets --all-features -- -D warnings` and `nix develop .#ci -c cargo fmt -p tonk-worker -- --check`
Expected: both clean.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker/src/router/blob.rs rust/tonk-worker/src/router.rs
git commit -m "feat(tonk-worker): POST blob upload route ingests bytes + asserts metadata"
```

---

### Task 2: Factor the `{branch}@{repo}`→URL helper

Extract the branch/repo parse currently inline in `blob_image_src` (added in the display feature) so `<tonk-upload>` reuses it. Pure refactor — the existing `tonk:blob` `<img>` test must stay green.

**Files:**
- Modify: `rust/tonk-display/src/element.rs` (the `blob_image_src` helper)
- Create: `rust/tonk-display/src/blob_url.rs` (the shared helper) + declare `mod blob_url;` in `rust/tonk-display/src/lib.rs`

**Interfaces:**
- Produces: `pub(crate) fn branch_repo(with: &str) -> Option<(String, String)>` returning `(branch, repo)`, and `pub(crate) fn blob_read_url(with: &str, entity: &str) -> Option<String>`.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-display/src/blob_url.rs`:

```rust
//! Shared helpers for building blob route URLs from a `<tonk-display>`/
//! `<tonk-upload>` `with="{branch}@{repo}"` context attribute.

/// Parse a `with="{branch}@{repo}"` context into `(branch, repo)`. Returns
/// `None` if `with` is empty or still an unsubstituted `{…}` template. A bare
/// token with no `@` is a repo on the default branch `main`.
pub(crate) fn branch_repo(with: &str) -> Option<(String, String)> {
    if with.is_empty() || with.contains('{') {
        return None;
    }
    let (branch, repo) = match with.split_once('@') {
        Some((branch, repo)) => (branch, repo),
        None => ("main", with),
    };
    if repo.is_empty() {
        return None;
    }
    Some((branch.to_string(), repo.to_string()))
}

/// The read URL for a blob entity, scoped by `with`. `None` if `with` is unusable.
pub(crate) fn blob_read_url(with: &str, entity: &str) -> Option<String> {
    let (branch, repo) = branch_repo(with)?;
    Some(format!("/api/repository/{repo}/branch/{branch}/blob/{entity}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_parses_branch_and_repo() {
        assert_eq!(
            branch_repo("main@did:key:zX"),
            Some(("main".into(), "did:key:zX".into()))
        );
        assert_eq!(branch_repo("did:key:zX"), Some(("main".into(), "did:key:zX".into())));
        assert_eq!(branch_repo(""), None);
        assert_eq!(branch_repo("{branch}@{repo}"), None);
    }

    #[dialog_common::test]
    fn it_builds_the_read_url() {
        assert_eq!(
            blob_read_url("main@repo", "blob:zH").as_deref(),
            Some("/api/repository/repo/branch/main/blob/blob:zH"),
        );
        assert_eq!(blob_read_url("{x}", "blob:zH"), None);
    }
}
```

- [ ] **Step 2: Run it — confirm it fails**

Run: `nix develop .#ci -c bash -c 'cargo build -p tonk-display 2>&1 | tail'`
Expected: FAIL — `mod blob_url;` isn't declared yet, so the module/test isn't compiled in (or `cargo test` can't find it).

- [ ] **Step 3: Wire the module and reuse it in `blob_image_src`**

Add `mod blob_url;` to `rust/tonk-display/src/lib.rs` (next to the other `mod` decls). Then rewrite `blob_image_src` in `element.rs` to delegate — replace its inline `with` parse + `format!` with:

```rust
fn blob_image_src(host: &Element) -> Option<String> {
    let entity = host.get_attribute("entity")?;
    let with = host.get_attribute("with")?;
    crate::blob_url::blob_read_url(&with, &entity)
}
```

(Keep the surrounding `handle_blob_image_frame` logic unchanged.)

- [ ] **Step 4: Run tests — confirm pass**

Run: `nix develop .#ci -c bash -c 'cargo nextest run -p tonk-display -E "test(blob_url) + test(it_mounts_an_img_for_a_blob_model)" 2>&1 | tail -20'`
Expected: the two new `blob_url` tests PASS and the existing `it_mounts_an_img_for_a_blob_model` still PASSES (proving the refactor is behavior-preserving).

- [ ] **Step 5: Lint + commit**

Run: `nix develop .#ci -c cargo clippy -p tonk-display --all-targets --all-features -- -D warnings` (clean), then:

```bash
git add rust/tonk-display/src/blob_url.rs rust/tonk-display/src/lib.rs rust/tonk-display/src/element.rs
git commit -m "refactor(tonk-display): factor shared with->blob-url helper"
```

---

### Task 3: `<tonk-upload>` element — registration + default shadow UI

Create the element skeleton: registered custom element, shadow root, default UI (trigger slot + preview part + status part) + `STYLE`, `data-state=idle`. No upload logic yet.

**Files:**
- Create: `rust/tonk-display/src/upload.rs`
- Modify: `rust/tonk-display/src/lib.rs` (`mod upload;` + call `upload::register()` in the crate's `register()` fan-out at lib.rs:78-85)

**Interfaces:**
- Consumes: the `custom-elements` crate (`CustomElement`, `::define`), `web-sys` (`HtmlElement`, `ShadowRoot`, `ShadowRootInit`, `ShadowRootMode`, `Element`, `Event`, `HtmlInputElement`), `wasm_bindgen`.
- Produces: `pub fn register()` defining `<tonk-upload>`; `struct TonkUploadElement` implementing `CustomElement`.

- [ ] **Step 1: Write the failing test**

In `rust/tonk-display/src/upload.rs`, add the test module (mirrors the no-FakeHost pattern from `element.rs`'s `mod hook`):

```rust
#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    fn document() -> web_sys::Document {
        web_sys::window().unwrap().document().unwrap()
    }
    async fn sleep(ms: i32) {
        let p = js_sys::Promise::new(&mut |res, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&res, ms);
        });
        let _ = JsFuture::from(p).await;
    }

    #[dialog_common::test]
    async fn it_builds_a_default_shadow_ui() {
        super::register();
        let el = document().create_element("tonk-upload").unwrap();
        el.set_attribute("with", "main@did:key:zSpace").unwrap();
        document().body().unwrap().append_child(&el).unwrap();

        let host: web_sys::HtmlElement = el.dyn_into().unwrap();
        let mut root = None;
        for _ in 0..200 {
            if let Some(r) = host.shadow_root() {
                if r.query_selector("input[type=file]").unwrap().is_some() {
                    root = Some(r);
                    break;
                }
            }
            sleep(5).await;
        }
        let root = root.expect("shadow UI built");
        assert!(root.query_selector("input[type=file]").unwrap().is_some());
        assert!(root.query_selector("[part=button]").unwrap().is_some());
        assert!(root.query_selector("[part=preview]").unwrap().is_some());
        assert!(root.query_selector("[part=status]").unwrap().is_some());
        assert_eq!(host.get_attribute("data-state").as_deref(), Some("idle"));
    }
}
```

- [ ] **Step 2: Run it — confirm it fails**

Run: `nix develop .#ci -c bash -c 'cargo build -p tonk-display 2>&1 | tail'`
Expected: FAIL — `upload` module/`register` doesn't exist.

- [ ] **Step 3: Implement the skeleton**

Write the element above the test module in `rust/tonk-display/src/upload.rs`, mirroring `rust/tonk-tree/src/web.rs:48-208, 652-658`:

```rust
//! `<tonk-upload>` — a headless native blob-upload primitive.
//!
//! The mechanism (file input, ingest fetch, state machine, preview) lives in
//! Shadow DOM; the author restyles it via the `trigger` slot, `::part()`
//! (`base`/`button`/`preview`/`status`), `--tonk-upload-*` CSS vars, and the
//! reflected `data-state` attribute. Bytes are ingested through the worker
//! `POST …/blob` route (relayed by the guest's `window.fetch`), and a
//! successful upload dispatches a bubbling `tonk-upload` CustomEvent carrying
//! the resulting `blob:<hash>`.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, ShadowRoot};

/// Per-instance state: the shadow root and the live preview/status nodes, plus
/// the change-listener closure kept alive for the element's lifetime.
#[derive(Default)]
struct Inner {
    shadow: Option<ShadowRoot>,
    // Closures are stored to keep them alive; filled in Task 4.
    listeners: Vec<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>>,
}

type Shared = Rc<RefCell<Inner>>;

#[derive(Default)]
pub struct TonkUploadElement {
    state: Shared,
}

impl CustomElement for TonkUploadElement {
    fn shadow() -> bool {
        // We attach our own shadow root so we control build timing.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["with", "accept"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        set_state(this, "idle");
        build_shadow(this, &self.state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        *self.state.borrow_mut() = Inner::default();
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        // `accept` propagates to the input; `with` only affects the next upload.
        if name == "accept" {
            if let Some(root) = self.state.borrow().shadow.as_ref() {
                if let Ok(Some(input)) = root.query_selector("input[type=file]") {
                    let _ = input.set_attribute("accept", new.as_deref().unwrap_or(""));
                }
            }
        }
    }
}

/// Reflect the state machine onto the host for `data-state` CSS.
fn set_state(this: &HtmlElement, state: &str) {
    let _ = this.set_attribute("data-state", state);
}

fn ensure_shadow(this: &HtmlElement) -> ShadowRoot {
    if let Some(root) = this.shadow_root() {
        return root;
    }
    let init = web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open);
    this.attach_shadow(&init).unwrap()
}

fn doc() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}
fn el(tag: &str) -> Element {
    doc().create_element(tag).unwrap()
}

/// Build the default shadow UI once. Wiring of the file input is added in Task 4.
fn build_shadow(this: &HtmlElement, state: &Shared) {
    let root = ensure_shadow(this);
    // style
    let style = el("style");
    style.set_text_content(Some(STYLE));
    let _ = root.append_child(&style);

    // container
    let base = el("div");
    let _ = base.set_attribute("part", "base");

    // hidden real input
    let input = el("input");
    let _ = input.set_attribute("type", "file");
    let _ = input.set_attribute("hidden", "");
    if let Some(accept) = this.get_attribute("accept") {
        let _ = input.set_attribute("accept", &accept);
    }
    let _ = base.append_child(&input);

    // trigger slot (fallback = default button)
    let slot = el("slot");
    let _ = slot.set_attribute("name", "trigger");
    let button = el("button");
    let _ = button.set_attribute("part", "button");
    let _ = button.set_attribute("type", "button");
    button.set_text_content(Some("Choose file…"));
    let _ = slot.append_child(&button);
    let _ = base.append_child(&slot);

    // preview (element-owned part; hidden until an upload)
    let preview = el("img");
    let _ = preview.set_attribute("part", "preview");
    let _ = preview.set_attribute("hidden", "");
    let _ = base.append_child(&preview);

    // status (element-owned part)
    let status = el("span");
    let _ = status.set_attribute("part", "status");
    let _ = base.append_child(&status);

    let _ = root.append_child(&base);
    state.borrow_mut().shadow = Some(root);
    // Task 4 wires the trigger-slot click → input.click() and the input change
    // → the upload flow here, pushing closures into state.listeners.
}

const STYLE: &str = r#"
:host { display: inline-flex; }
[part=base] {
  display: inline-flex; align-items: center; gap: var(--tonk-upload-gap, 8px);
  font-family: var(--wa-font-family-body, system-ui, sans-serif);
}
[part=button] {
  padding: 6px 12px; border-radius: var(--tonk-upload-radius, 6px);
  border: 1px solid var(--wa-color-neutral-fill-loud, #ccc);
  background: var(--tonk-upload-accent, var(--wa-color-brand-fill-loud, #2563eb));
  color: var(--wa-color-brand-on-loud, #fff); cursor: pointer; font: inherit;
}
[part=button]:hover { filter: brightness(1.05); }
[part=preview] { max-width: 96px; max-height: 96px; border-radius: 4px; }
[part=preview][hidden] { display: none; }
[part=status] { font-size: 12px; color: var(--wa-color-text-quiet, #666); }
:host([data-state=error]) [part=status] { color: var(--wa-color-danger-fill-loud, #dc2626); }
"#;

/// Register the `<tonk-upload>` element. Idempotent.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkUploadElement::define("tonk-upload");
}

fn already_registered() -> bool {
    doc()
        .default_view()
        .map(|w| !w.custom_elements().get("tonk-upload").is_undefined())
        .unwrap_or(false)
}
```

Then in `rust/tonk-display/src/lib.rs`: add `mod upload;` and add `upload::register();` to the `#[cfg(target_arch = "wasm32")] pub fn register()` fan-out (the one calling `view::register()`, `element::register()`, etc.). Confirm `custom-elements = { workspace = true }` and the needed `web-sys` features (`ShadowRoot`, `ShadowRootInit`, `ShadowRootMode`, `HtmlInputElement`, `File`, `FileList`, `Blob`, `Headers`, `RequestInit`, `Request`, `Response`, `Url`, `CustomEvent`, `CustomEventInit`) are enabled for `tonk-display` in `Cargo.toml` — add any missing ones.

- [ ] **Step 4: Run the test — confirm pass**

Run: `nix develop .#ci -c bash -c 'cargo nextest run -p tonk-display -E "test(it_builds_a_default_shadow_ui)" 2>&1 | tail -20'`
Expected: PASS (browser). If it can't find Chrome, use the repo's `test:web:debug` path.

- [ ] **Step 5: Lint + commit**

`nix develop .#ci -c cargo clippy -p tonk-display --all-targets --all-features -- -D warnings` (clean), then:

```bash
git add rust/tonk-display/src/upload.rs rust/tonk-display/src/lib.rs rust/tonk-display/Cargo.toml
git commit -m "feat(tonk-display): <tonk-upload> element skeleton + default shadow UI"
```

---

### Task 4: `<tonk-upload>` — pick → read → upload → preview → emit

Wire the behavior: clicking the trigger opens the picker; a picked file is read to bytes, uploaded via the relayed `window.fetch`, the preview swaps to the read route, and a `tonk-upload` event fires. Errors set `data-state=error` and emit nothing. Missing `with` disables the trigger.

**Files:** Modify `rust/tonk-display/src/upload.rs`.

**Interfaces:**
- Consumes: `crate::blob_url::branch_repo` (Task 2); `web-sys` (`File`, `FileList`, `HtmlInputElement`, `Headers`, `RequestInit`, `Response`, `Url`, `CustomEvent`, `CustomEventInit`); `js_sys::Uint8Array`; `wasm_bindgen_futures::{spawn_local, JsFuture}`.
- Produces: the `tonk-upload` event contract (`detail = { blob, contentType, name, size }`).

- [ ] **Step 1: Write the failing test**

Add to `upload.rs`'s test module. Stub the relayed `window.fetch` so no real network is needed, then drive the input and assert the event + preview swap + state.

```rust
#[dialog_common::test]
async fn it_uploads_and_emits_on_pick() {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsValue;

    super::register();

    // Stub window.fetch → a Response with the canned JSON, so the element's
    // POST resolves deterministically. (Restore is unnecessary in the test realm.)
    let stub = Closure::wrap(Box::new(move |_url: JsValue, _init: JsValue| {
        let json = r#"{"entity":"blob:zH","contentType":"image/png","name":"a.png","size":3}"#;
        let init = web_sys::ResponseInit::new();
        init.set_status(200);
        let resp = web_sys::Response::new_with_opt_str_and_init(Some(json), &init).unwrap();
        js_sys::Promise::resolve(&resp.into())
    }) as Box<dyn FnMut(JsValue, JsValue) -> js_sys::Promise>);
    js_sys::Reflect::set(
        &web_sys::window().unwrap(),
        &"fetch".into(),
        stub.as_ref().unchecked_ref(),
    )
    .unwrap();
    stub.forget();

    let el = document().create_element("tonk-upload").unwrap();
    el.set_attribute("with", "main@repo").unwrap();
    document().body().unwrap().append_child(&el).unwrap();
    let host: web_sys::HtmlElement = el.dyn_into().unwrap();

    // Listen for the success event.
    let got: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    {
        let got = got.clone();
        let cb = Closure::wrap(Box::new(move |ev: web_sys::CustomEvent| {
            let detail = ev.detail();
            let blob = js_sys::Reflect::get(&detail, &"blob".into())
                .ok()
                .and_then(|v| v.as_string());
            *got.borrow_mut() = blob;
        }) as Box<dyn FnMut(web_sys::CustomEvent)>);
        host.add_event_listener_with_callback("tonk-upload", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Wait for the shadow, then drive a synthetic file through the flow.
    // (Helper `dispatch_test_file` is added to the element in Step 3 so tests can
    // inject a File without a real OS picker.)
    let mut input = None;
    for _ in 0..200 {
        if let Some(r) = host.shadow_root() {
            if let Ok(Some(i)) = r.query_selector("input[type=file]") {
                input = Some(i);
                break;
            }
        }
        sleep(5).await;
    }
    let input: web_sys::HtmlInputElement = input.unwrap().dyn_into().unwrap();

    // Build a File and hand it to the element's test hook.
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(&b"abc"[..]).into());
    let opts = web_sys::FilePropertyBag::new();
    opts.set_type("image/png");
    let file =
        web_sys::File::new_with_u8_array_sequence_and_options(&parts, "a.png", &opts).unwrap();
    super::__test_ingest(&host, &file);

    // The event fires, preview swaps to the read route, state is done.
    for _ in 0..200 {
        if got.borrow().is_some() {
            break;
        }
        sleep(5).await;
    }
    assert_eq!(got.borrow().as_deref(), Some("blob:zH"));
    assert_eq!(host.get_attribute("data-state").as_deref(), Some("done"));
    let preview = host
        .shadow_root()
        .unwrap()
        .query_selector("[part=preview]")
        .unwrap()
        .unwrap();
    assert_eq!(
        preview.get_attribute("src").as_deref(),
        Some("/api/repository/repo/branch/main/blob/blob:zH"),
    );
    let _ = input; // silence unused
}
```

- [ ] **Step 2: Run it — confirm it fails**

Run: `nix develop .#ci -c bash -c 'cargo build -p tonk-display 2>&1 | tail'`
Expected: FAIL — `__test_ingest` and the upload flow don't exist.

- [ ] **Step 3: Implement the flow**

In `upload.rs`, wire the trigger + input in `build_shadow` and add the ingest function. Add these pieces:

In `build_shadow`, after building the nodes, before storing state — wire the trigger-slot click and the input change:

```rust
    // Clicking anywhere on the trigger slot (default button or author-slotted
    // content) opens the native picker — unless `with` is missing.
    let with_ok = crate::blob_url::branch_repo(&this.get_attribute("with").unwrap_or_default())
        .is_some();
    if !with_ok {
        let _ = button.set_attribute("disabled", "");
        status.set_text_content(Some("no space context"));
    }
    {
        let input_for_click = input.clone();
        let click = wasm_bindgen::closure::Closure::wrap(Box::new(move |_e: web_sys::Event| {
            if with_ok {
                let _ = input_for_click.unchecked_ref::<web_sys::HtmlElement>().click();
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = slot.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        state.borrow_mut().listeners.push(click);
    }
    {
        let host = this.clone();
        let change = wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::Event| {
            if let Some(target) = e.target() {
                if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                    if let Some(file) = input.files().and_then(|l| l.get(0)) {
                        ingest(&host, &file);
                    }
                }
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = input.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        state.borrow_mut().listeners.push(change);
    }
```

Add the ingest function + a test hook:

```rust
/// Read `file` to bytes, POST it to the ingest route, swap the preview to the
/// read route, and emit `tonk-upload` on success. All async; drives `data-state`.
fn ingest(host: &HtmlElement, file: &web_sys::File) {
    let host = host.clone();
    let file = file.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let with = host.get_attribute("with").unwrap_or_default();
        let Some((branch, repo)) = crate::blob_url::branch_repo(&with) else {
            fail(&host, "no space context");
            return;
        };
        let name = file.name();
        let mime = {
            let t = file.type_();
            if t.is_empty() { "application/octet-stream".to_string() } else { t }
        };

        set_state(&host, "reading");
        // Instant local preview.
        if mime.starts_with("image/") {
            if let Ok(url) = web_sys::Url::create_object_url_with_blob(&file) {
                show_preview(&host, &url);
            }
        }

        // Read bytes.
        let buf = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
            Ok(b) => b,
            Err(_) => return fail(&host, "could not read file"),
        };
        let bytes = js_sys::Uint8Array::new(&buf);

        set_state(&host, "uploading");
        // POST via the relayed window.fetch (string URL, binary init.body).
        let url = format!("/api/repository/{repo}/branch/{branch}/blob");
        let headers = web_sys::Headers::new().unwrap();
        let _ = headers.append("content-type", &mime);
        let _ = headers.append("x-tonk-blob-name", &name);
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_headers(&headers);
        init.set_body(&bytes.into());
        let win = web_sys::window().unwrap();
        let resp_val = match wasm_bindgen_futures::JsFuture::from(
            win.fetch_with_str_and_init(&url, &init),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => return fail(&host, "upload failed"),
        };
        let resp: web_sys::Response = resp_val.dyn_into().unwrap();
        if !resp.ok() {
            return fail(&host, &format!("upload failed ({})", resp.status()));
        }
        let text = match wasm_bindgen_futures::JsFuture::from(resp.text().unwrap()).await {
            Ok(t) => t.as_string().unwrap_or_default(),
            Err(_) => return fail(&host, "bad response"),
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(j) => j,
            Err(_) => return fail(&host, "bad response"),
        };
        let entity = json["entity"].as_str().unwrap_or_default().to_string();
        let content_type = json["contentType"].as_str().unwrap_or(&mime).to_string();
        let size = json["size"].as_u64().unwrap_or(0);

        // Swap preview to the read route (proves storage). Non-image → name+size.
        if content_type.starts_with("image/") {
            if let Some(read) = crate::blob_url::blob_read_url(&with, &entity) {
                show_preview(&host, &read);
            }
        } else {
            set_status(&host, &format!("{name} · {size} bytes"));
        }

        set_state(&host, "done");
        emit(&host, &entity, &content_type, &name, size);
    });
}

fn show_preview(host: &HtmlElement, src: &str) {
    if let Some(root) = host.shadow_root() {
        if let Ok(Some(img)) = root.query_selector("[part=preview]") {
            let _ = img.set_attribute("src", src);
            let _ = img.remove_attribute("hidden");
        }
    }
}
fn set_status(host: &HtmlElement, msg: &str) {
    if let Some(root) = host.shadow_root() {
        if let Ok(Some(s)) = root.query_selector("[part=status]") {
            s.set_text_content(Some(msg));
        }
    }
}
fn fail(host: &HtmlElement, msg: &str) {
    set_state(host, "error");
    set_status(host, msg);
    if let Some(root) = host.shadow_root() {
        if let Ok(Some(img)) = root.query_selector("[part=preview]") {
            let _ = img.set_attribute("hidden", "");
        }
    }
}
fn emit(host: &HtmlElement, blob: &str, content_type: &str, name: &str, size: u64) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &"blob".into(), &blob.into());
    let _ = js_sys::Reflect::set(&detail, &"contentType".into(), &content_type.into());
    let _ = js_sys::Reflect::set(&detail, &"name".into(), &name.into());
    let _ = js_sys::Reflect::set(&detail, &"size".into(), &(size as f64).into());
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_detail(&detail);
    if let Ok(ev) = web_sys::CustomEvent::new_with_event_init_dict("tonk-upload", &init) {
        let _ = host.dispatch_event(&ev);
    }
}

/// Test-only hook: inject a `File` as if the user had picked it (browsers don't
/// let script set `input.files`, and there's no real OS picker in tests).
#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) fn __test_ingest(host: &HtmlElement, file: &web_sys::File) {
    ingest(host, file);
}
```

> **Implementer note:** the exact `web-sys` setter shapes (`RequestInit`/`ResponseInit`/`CustomEventInit`/`FilePropertyBag` — builder methods vs fields) depend on the pinned `web-sys` version; follow the shapes in `rust/tonk-tree/src/model.rs:91-126` (RequestInit/Headers/fetch) and `rust/tonk-guest/src/bin/guest.rs:75-99` (Blob/Url) — adapt setter calls to whatever compiles against the workspace `web-sys`. The `#[cfg(test)]`-gated `__test_ingest` keeps the hook out of release builds.

- [ ] **Step 4: Run tests — confirm pass**

Run: `nix develop .#ci -c bash -c 'cargo nextest run -p tonk-display -E "test(it_uploads_and_emits_on_pick) + test(it_builds_a_default_shadow_ui)" 2>&1 | tail -30'`
Expected: both PASS.

- [ ] **Step 5: Add the failure + slot-override tests**

Add two more `#[dialog_common::test]`s to `upload.rs`: (a) stub `window.fetch` to resolve a `500` Response → assert `data-state=error`, no `tonk-upload` event, preview hidden; (b) mount `<tonk-upload with="main@repo"><button slot="trigger">Pick</button></tonk-upload>`, assert the slotted `[slot=trigger]` button is present as an assigned node and the default `[part=button]` is the slot fallback (still uploads via `__test_ingest`). Run them; expect PASS.

- [ ] **Step 6: Lint + commit**

`nix develop .#ci -c cargo clippy -p tonk-display --all-targets --all-features -- -D warnings` (clean), `cargo fmt` check, then:

```bash
git add rust/tonk-display/src/upload.rs
git commit -m "feat(tonk-display): <tonk-upload> pick/read/upload/preview/emit + errors"
```

---

### Task 5: Register in the guest bundle + demo view

Make `<tonk-upload>` available in the sealed guest, and add a demo for manual e2e.

**Files:**
- Modify: `rust/tonk-guest/src/bin/guest.rs` (confirm `tonk_display::register()` is called — it already is, and it now fans out to `upload::register()` from Task 3, so `<tonk-upload>` is registered automatically; add a direct call only if it's a separate crate). Verify no separate wiring is needed.
- Modify: `rust/tonk-core/assets/library/demo.yaml` (a demo view mounting `<tonk-upload>` that logs/handles the event).

- [ ] **Step 1: Confirm guest registration**

Read `rust/tonk-guest/src/bin/guest.rs:21-56`. Since Task 3 added `upload::register()` to `tonk_display::register()`'s fan-out (`lib.rs:78-85`), and `guest.rs` already calls `tonk_display::register()`, `<tonk-upload>` is registered in the guest with no further change. Confirm this by reading the fan-out; if `<tonk-upload>` were its own crate it would need an explicit `tonk_upload::register();` line — it is not.

- [ ] **Step 2: Add a demo view**

In `rust/tonk-core/assets/library/demo.yaml`, add a small view that mounts the element inside a route context that forwards `with`, e.g. a concept + `view!` whose `display` is:

```
<tonk-upload with="{branch}@{repo}" accept="image/*"></tonk-upload>
```

(Match the surrounding demo.yaml conventions for how a view gets its `{branch}`/`{repo}` — mirror an existing route/demo view that already forwards `with`, e.g. the artifact route view.) This is manual-e2e only; no automated assertion.

- [ ] **Step 3: Validate the library still loads**

Run the tonk-core seed/analyze validation the way the repo does (mirror how other `*.yaml` library changes are validated — check `rust/tonk-core` for the analyze/seed test harness). Expected: the demo library parses/validates.

- [ ] **Step 4: Manual e2e (document, don't automate)**

In `dev:web` (now backed by a local blob-aware access service): open the demo view, pick an image, confirm (a) the preview renders, (b) the Network tab shows `POST …/blob` → 200 then `GET …/blob/blob:<hash>` → 200 `image/png`, (c) a `tonk-upload` event is observable (console log in the demo). Record the steps in the PR description.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-core/assets/library/demo.yaml
git commit -m "docs(demo): <tonk-upload> demo view for manual e2e"
```

---

## Self-Review

- **Spec coverage:** Task 1 = the POST route (asserts content-type/name via `X-Tonk-Blob-Name`, JSON response, idempotent, empty→400). Tasks 3–4 = the headless element (shadow default UI, `with`/`accept`, pick→read→POST→preview-swap→emit, `data-state`, error path, slot trigger). Task 2 = the shared URL helper the spec calls for. Task 5 = guest registration + demo. ✔
- **Deviation from spec (flagged):** the spec listed `preview`/`status` as slots; this plan makes them element-owned `::part()`s (the element writes their content) and keeps only `trigger` as a slot. Rationale: the element drives preview/status content, so slotting them cleanly is a follow-up. Restyling via `::part()` + `data-state` + CSS vars is preserved. **This is a real spec deviation — confirm it's acceptable at review, or promote preview/status to slots.**
- **Placeholder scan:** each code step is complete; two "Implementer note" blocks flag where the exact reactor-commit path (Task 1) and `web-sys` builder shapes (Task 4) must be matched to the pinned APIs rather than trusted verbatim — these are grounding instructions, not TODOs.
- **Type consistency:** the ingest URL (`…/blob`) and read URL (`…/blob/{entity}`) match Task 1's routes and the shared helper; the `tonk-upload` `detail` keys (`blob`/`contentType`/`name`/`size`) are identical across the element, the emit fn, and the tests.
- **Risk:** the biggest execution risk is Task 4's `web-sys` builder-API shapes drifting from the pinned version — mitigated by pointing at `tonk-tree/model.rs` and `guest.rs` as compile-checked references. Element tests need the browser harness (chromedriver) — same as the display tests already in CI.
