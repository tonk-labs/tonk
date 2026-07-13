# Web Blob Images (inline) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a content-addressed image blob (`blob:<hash>`) render as an inline `<img>` in the tonk web frontend, wherever a view embeds it.

**Architecture:** The blob substrate (storage, sync, the `tonk:blob` concept, the `tonk blob` CLI) already exists on this branch. This plan adds the missing *web display* path in two grounded pieces: (1) a new tonk-worker HTTP route that streams a blob's bytes from the branch's blob store with the right `Content-Type`; (2) a `<tonk-display>` dispatch that, when its `model` is `tonk:blob`, mounts a single `<img>` whose `src` points at that route (URL built from the display's own `with` + `entity` attributes). No new concept, no route-table change, no change to the base view predicate.

**Tech Stack:** Rust; axum-in-wasm (tonk-worker service worker); wasm-bindgen + web-sys custom elements (tonk-display); `dialog-repository` blob API (`Blob::from(entity).read(branch.blobs())`, `branch.write_blob(...)`).

## Global Constraints

- **Dialog dep pin:** the blob API (`Branch::blobs`, `Blob::from`, `branch.write_blob`, `Entity::from_blob`, `Entity::blob_hash`) exists only on the `dialog-db` `branch = "main"` checkout (`Cargo.lock` currently pins `d2af7b7`). Do not repin `dialog-*` to an older tag — the blob API disappears on `tonk-2026-07-06` and earlier.
- **Tests:** always `#[dialog_common::test]` (never `#[test]`/`#[tokio::test]`). tonk-worker test mod is gated `#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]` with `wasm_bindgen_test_configure!(run_in_service_worker)`; tonk-display test mod uses `run_in_browser`. Both run via `nix develop -c test:web:debug`.
- **No `mod.rs`:** use `foo.rs` + `foo/` form.
- **No phase/RFC references** in code or comments; code stands on its own.
- **Scope:** images only, inline-in-views only. A dedicated blob *route/page*, non-image MIME types (PDF via `<embed>`), a headless-renderer (tonk-render) mirror, and an in-browser blob *upload* route are explicitly out of scope — see "Deferred" at the end.
- **URL shape (used by both tasks, must match exactly):** `/api/repository/{repo}/branch/{branch}/blob/{entity}` where `{entity}` is the full `blob:<hash>` string.

---

### Task 1: Worker route that serves blob bytes

Serve a blob's raw bytes from the branch's content-addressed store, with `Content-Type` taken from the blob's `xyz.tonk.blob/content-type` fact. Mirrors the existing fact-serving handler `rust/tonk-worker/src/router/host.rs::guest`, but reads the *blob store* (`branch.blobs()`) instead of selecting a fact value.

**Files:**
- Create: `rust/tonk-worker/src/router/blob.rs`
- Modify: `rust/tonk-worker/src/router.rs` (add `mod blob;` near `mod host;` at line 74; register the route near line 263)
- Test: in `rust/tonk-worker/src/router/blob.rs` (`#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))] mod tests`)

**Interfaces:**
- Consumes: `super::AppState` (= `Arc<RwLock<TonkState>>`); `crate::TonkWorkerError` (variants `Router`, `NotFound`, `Internal` — same as `host.rs`); test helpers `crate::router::tests::{test_state, put_repo}` and `crate::api_router_from_state`.
- Produces: `pub async fn serve(...) -> Result<Response, TonkWorkerError>` registered at `GET /api/repository/{repo}/branch/{branch}/blob/{entity}`.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-worker/src/router/blob.rs` with only the test module for now (the handler comes in Step 3):

```rust
//! `GET /api/repository/{repo}/branch/{branch}/blob/{entity}` —
//! serve content-addressed blob bytes from a branch's blob store.

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dialog_artifacts::Entity;
    use dialog_repository::RepositoryExt as _;
    use futures_util::stream;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::api_router_from_state;
    use crate::router::tests::{put_repo, test_state};

    #[dialog_common::test]
    async fn it_serves_blob_bytes_with_the_asserted_content_type() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-serve").await;

        // Write a blob straight into the branch store, then derive its
        // `blob:<hash>` entity (the same value `tonk blob add` returns).
        let payload = b"\x89PNG\r\n\x1a\nhello".to_vec();
        let entity: Entity = {
            let guard = app_state.read().await;
            let repository = guard
                .profile
                .repository(&repo)
                .load()
                .perform(&guard.operator)
                .await
                .unwrap();
            let branch = repository
                .branch("main")
                .open()
                .perform(&guard.operator)
                .await
                .unwrap();
            let chunks = vec![Ok::<_, dialog_effects::blob::BlobError>(payload.clone())];
            let hash = branch
                .write_blob(stream::iter(chunks))
                .perform(&guard.operator)
                .await
                .unwrap();
            Entity::from_blob(hash.as_bytes()).unwrap()
        };

        // Assert its content-type fact through the HTTP claim route.
        let assert = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{repo}/branch/main/claim/assert/{entity}/xyz.tonk.blob/content-type"
                    ))
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("image/png"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(assert.status(), StatusCode::OK);

        // GET the bytes back.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob/{entity}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "image/png",
            "Content-Type comes from the xyz.tonk.blob/content-type fact",
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), payload.as_slice());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c test:web:debug 2>&1 | rg -i 'blob-serve|it_serves_blob_bytes|error\['`
Expected: FAIL — the route isn't registered yet, so `GET .../blob/{entity}` returns 404 (or the crate fails to compile because `blob::serve` / `mod blob` don't exist). Either way, not a pass.

- [ ] **Step 3: Write the handler**

Prepend the handler above the test module in `rust/tonk-worker/src/router/blob.rs`:

```rust
use ::axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Entity};
use dialog_effects::blob::BlobError;
use dialog_repository::{Blob, CommitError, RepositoryExt as _};
use futures_util::StreamExt as _;
use serde::Deserialize;

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for the blob route.
#[derive(Debug, Deserialize)]
pub struct BlobPath {
    /// The repository the blob's branch lives in.
    pub repo: String,
    /// The branch whose blob store holds the bytes.
    pub branch: String,
    /// The `blob:<hash>` entity URI to serve.
    pub entity: String,
}

/// Serve a blob's bytes. The `Content-Type` is read from the blob's
/// `xyz.tonk.blob/content-type` fact (as asserted by `tonk blob add`),
/// defaulting to `application/octet-stream` when none is recorded.
///
/// The whole blob is buffered before responding — fine for images;
/// streaming is a later refinement (see the plan's "Deferred" notes).
#[wasm_compat]
pub async fn serve(
    State(state): State<AppState>,
    Path(params): Path<BlobPath>,
) -> Result<Response, TonkWorkerError> {
    let entity: Entity = params.entity.parse().map_err(|e| {
        TonkWorkerError::Router(format!("Invalid blob entity '{}': {}", params.entity, e))
    })?;
    if entity.blob_hash().is_none() {
        return Err(TonkWorkerError::Router(format!(
            "Not a blob reference: {}",
            params.entity
        )));
    }

    let tonk = state.read().await;
    let repo = tonk
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", params.repo, e))
        })?;
    let branch = repo
        .branch(params.branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", params.branch, e))
        })?;

    // Content type from the blob's metadata fact, if asserted.
    let ct_attr: Attribute = "xyz.tonk.blob/content-type"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {}", e)))?;
    let ct_stream = branch
        .claims()
        .select(ArtifactSelector::new().the(ct_attr).of(entity.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("content-type query: {}", e)))?;
    tokio::pin!(ct_stream);
    let content_type = match ct_stream.next().await {
        Some(Ok(artifact)) => {
            String::try_from(artifact.is).unwrap_or_else(|_| "application/octet-stream".to_string())
        }
        _ => "application/octet-stream".to_string(),
    };

    // Blob bytes from the branch's content-addressed store.
    let mut reader = match Blob::from(entity)
        .read(branch.blobs())
        .perform(&tonk.operator)
        .await
    {
        Ok(reader) => reader,
        Err(CommitError::Blob(BlobError::NotFound(_))) => {
            return Err(TonkWorkerError::NotFound(format!(
                "blob not available: {}",
                params.entity
            )));
        }
        Err(e) => return Err(TonkWorkerError::Internal(format!("read blob: {}", e))),
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = reader
        .next()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("read blob chunk: {}", e)))?
    {
        bytes.extend_from_slice(&chunk);
    }

    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    Ok(response)
}
```

Then register it in `rust/tonk-worker/src/router.rs`. Add the module declaration next to `mod host;` (line 74):

```rust
mod host;
pub use host::{ClientId, ViewBinding, ViewBindings};

mod blob;
```

And add the route immediately after the `host::guest` route (the `.route("/api/repository/{repo}/branch/{branch}/host/{host}/{entity}", get(host::guest))` block ending at line 263):

```rust
        // Content-addressed blob bytes. Serves a `blob:<hash>` entity's
        // bytes from the branch's blob store, with `Content-Type` from
        // the blob's `xyz.tonk.blob/content-type` fact. `<tonk-display>`
        // points `<img src>` here for `tonk:blob` models.
        .route(
            "/api/repository/{repo}/branch/{branch}/blob/{entity}",
            get(blob::serve),
        )
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop -c test:web:debug 2>&1 | rg -i 'it_serves_blob_bytes|result:|passed|failed'`
Expected: `it_serves_blob_bytes_with_the_asserted_content_type` PASSES; overall run reports 0 failures.

- [ ] **Step 5: Confirm the native build still compiles (the handler is `wasm_compat`, but the crate also builds native)**

Run: `nix develop -c cargo clippy -p tonk-worker --all-targets -- -D warnings`
Expected: no warnings/errors. (The lint gate is native clippy — see the repo's lint-gate note.)

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker/src/router/blob.rs rust/tonk-worker/src/router.rs
git commit -m "feat(tonk-worker): serve content-addressed blob bytes over http"
```

---

### Task 2: `<tonk-display>` renders a `tonk:blob` model as an inline `<img>`

When a display is mounted with `model="tonk:blob"`, mount a single `<img>` whose `src` is the Task 1 URL, built from the display's own `with` (`{branch}@{repo}`) and `entity` (`blob:<hash>`) attributes. This is the "generalize the display dispatch" step, kept minimal: a new branch at the top of `handle_view_frame`, parallel to the existing `type == "text/html"` → portal branch, but keyed on the host `model` attribute so it needs no new view concept and no change to the base view predicate.

**Files:**
- Modify: `rust/tonk-display/src/element.rs` (insert a branch at the top of `handle_view_frame`, ~line 1076; add two helper fns near `mount_portal_slide` ~line 1351; add a test in the existing `#[cfg(test)] mod tests` → `mod hook`, near `it_mounts_a_portal_for_a_text_html_view_frame` ~line 2901)

**Interfaces:**
- Consumes: `host: &Element` with attributes `model`, `with`, `entity`; existing helpers `window()`, `document()`, `dispatch_event(host, ...)`, `state::set(host, State::Ready)`, `State`; the `Inner` state struct.
- Produces: `fn handle_blob_image_frame(host: &Element)` and `fn blob_image_src(host: &Element) -> Option<String>` (private to the module).

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-display/src/element.rs`, inside `mod hook` (the `#[cfg(target_arch = "wasm32")]` module that holds `it_mounts_a_portal_for_a_text_html_view_frame`), mirroring that test's harness:

```rust
        /// A display whose `model` is `tonk:blob` renders a single
        /// `<img>` pointing at the worker's blob-bytes route, with the
        /// repo/branch taken from the host's `with` attribute and the
        /// blob entity from `entity`. No inline template, no iframe.
        #[dialog_common::test]
        async fn it_mounts_an_img_for_a_blob_model() {
            let host = FakeHost::install_with_model(resolve_responses(), Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "tonk:blob", "blob:zHASH");
            // The embedding route view forwards `with="{branch}@{repo}"`;
            // here it is already substituted.
            display.set_attribute("with", "main@myrepo").unwrap();

            // Any view frame (even empty) drives handle_view_frame; the
            // blob branch fires on the host `model` before anything else.
            host.push_frame("view", &rows(&[]));

            let img = await_selector(&display, "img")
                .await
                .expect("a tonk:blob model should mount an <img>");
            assert_eq!(
                img.get_attribute("src").as_deref(),
                Some("/api/repository/myrepo/branch/main/blob/blob:zHASH"),
            );
        }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c test:web:debug 2>&1 | rg -i 'it_mounts_an_img_for_a_blob_model|error\['`
Expected: FAIL — no `<img>` is mounted (the frame falls through to the inline/default path), so `await_selector` times out.

- [ ] **Step 3: Add the dispatch branch and helpers**

In `handle_view_frame`, insert the blob branch as the *first* thing after the state borrow (immediately before the `if conclusions.is_empty()` block at line 1077):

```rust
    let mut s = state.borrow_mut();

    // A `tonk:blob` model renders as a single media element pointing at
    // the worker's blob-bytes route — no inline template, no iframe. The
    // check is on the host `model` attribute (not a projected frame
    // field), so it fires even on an empty view frame and needs no
    // dedicated view concept.
    if host.get_attribute("model").as_deref() == Some("tonk:blob") {
        drop(s);
        handle_blob_image_frame(host);
        dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
        return;
    }

    // (existing code continues: `if conclusions.is_empty() { ... }`)
```

Add the two helpers near `mount_portal_slide` (after it, ~line 1375):

```rust
/// Mount (or refresh) a single `<img>` whose `src` is the worker's
/// content-addressed blob route for this display's `entity`. Idempotent:
/// the `src` is stable for a given `(with, entity)`, so re-running on a
/// later frame is a no-op once the element exists.
fn handle_blob_image_frame(host: &Element) {
    let Some(src) = blob_image_src(host) else {
        return;
    };
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let img = match host.query_selector("img").ok().flatten() {
        Some(existing) => existing,
        None => {
            let Ok(created) = document.create_element("img") else {
                return;
            };
            let _ = host.append_child(&created);
            created
        }
    };
    if img.get_attribute("src").as_deref() != Some(src.as_str()) {
        let _ = img.set_attribute("src", &src);
    }
    state::set(host, State::Ready);
}

/// Build the blob-bytes URL for this display's `entity`, scoped to the
/// repo/branch carried in the host's `with` attribute (`"{branch}@{repo}"`,
/// as forwarded by route views). Returns `None` if `with` is missing or
/// still an unsubstituted template, or if `entity` is absent.
fn blob_image_src(host: &Element) -> Option<String> {
    let entity = host.get_attribute("entity")?;
    let with = host.get_attribute("with")?;
    if with.contains('{') {
        return None; // unsubstituted `{branch}@{repo}` template
    }
    // `with` is `branch@repo`; a bare token (no `@`) is a repo on `main`.
    let (branch, repo) = match with.split_once('@') {
        Some((branch, repo)) => (branch, repo),
        None => ("main", with.as_str()),
    };
    Some(format!(
        "/api/repository/{repo}/branch/{branch}/blob/{entity}"
    ))
}
```

Note on the `src`: the built URL begins with `/api/...`, so the leading `blob:` in the `{entity}` path segment is never at the start of the URL and the browser will not mistake it for a `blob:` object-URL scheme. The `:` is a legal path character.

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop -c test:web:debug 2>&1 | rg -i 'it_mounts_an_img_for_a_blob_model|result:|passed|failed'`
Expected: `it_mounts_an_img_for_a_blob_model` PASSES; the existing `it_mounts_a_portal_for_a_text_html_view_frame` and `it_renders_inline_and_subscribes_to_the_entity_when_no_type` still PASS (the new branch is above them and only fires for `model == "tonk:blob"`); 0 failures overall.

- [ ] **Step 5: Native clippy (the lint gate)**

Run: `nix develop -c cargo clippy -p tonk-display --all-targets -- -D warnings`
Expected: no warnings/errors.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-display/src/element.rs
git commit -m "feat(tonk-display): render tonk:blob models as an inline img"
```

---

### Task 3: End-to-end verification against the running app

Prove the two halves connect: a real blob, added via the CLI, renders in the browser when embedded in a view. This is a verification task (no new production code) plus a short docs note.

**Files:**
- Modify: `guide/src/reference.md` (the "Blobs" section, ~line 74) — add a sentence on web rendering.

- [ ] **Step 1: Add a blob to a synced repo via the CLI**

Run (adjust the repo/site to a real synced tonk space you can open in the browser):
```bash
nix develop -c tonk blob add ./some-image.png
```
Capture the printed `blob:<hash>` (stdout). Confirm `tonk blob ls` lists it with `image/png` and a size.

- [ ] **Step 2: Embed the blob in a view and open it in the browser**

In the same space, author a view that nests a blob display, forwarding context — e.g. on any concept whose view you control, add:
```
<tonk-display with="{branch}@{repo}" entity=blob:<hash> model="tonk:blob" />
```
(Or, for a quick check, mount that element directly in a route view that already forwards `with`/`entity`, mirroring `artifact/route/view` at `rust/tonk-core/assets/library/core.yaml:1612-1616`.)

- [ ] **Step 3: Verify in the browser**

Open the space in the web frontend. Confirm:
- An `<img>` appears where the blob display is embedded and the image renders (not a broken-image icon).
- In devtools Network, the request to `/api/repository/<repo>/branch/<branch>/blob/blob:<hash>` returns `200` with `Content-Type: image/png` and the correct byte length.

If the image is broken: check the Network tab's status/`Content-Type`. A `404` means Task 1's route didn't match or the blob isn't in that branch's store (it must be the branch the display's `with` names). A wrong/blank `Content-Type` means the `xyz.tonk.blob/content-type` fact wasn't asserted on that entity (`tonk blob add` asserts it; a hand-referenced blob may not have it).

- [ ] **Step 4: Document the web-rendering path**

In `guide/src/reference.md`, in the Blobs section, add a short note after the reference-from-fact example, e.g.:

```markdown
In the web frontend, a `blob:<hash>` reference renders inline: embed
`<tonk-display with="{branch}@{repo}" entity=blob:<hash> model="tonk:blob" />`
in a view and the display mounts an `<img>` served from
`/api/repository/{repo}/branch/{branch}/blob/{entity}` with the blob's
recorded content type.
```

- [ ] **Step 5: Commit**

```bash
git add guide/src/reference.md
git commit -m "docs(guide): note web inline rendering of image blobs"
```

---

## Deferred (out of scope for this plan)

Call these out so the truncation is explicit, not silent:

- **tonk-render (headless) parity.** `rust/tonk-render/src/page/orchestrate.rs:244` mirrors the browser's `type == "text/html"` → portal check into `ResolvedView { display, is_portal }` for server-side/preview rendering. A blob branch there would let headless previews (e.g. the slide daemon) show images too. Deferred because the browser path is the stated goal; the headless renderer needs the page's repo/branch/entity threaded to build the same URL, which is a separate change.
- **Non-image MIME types.** PDF (`<embed>`/`<object>`) and `text/html` blobs (route through the existing portal path). The Task 2 dispatch is image-shaped (`<img>`); extending it to branch on the blob's content-type is the natural follow-up.
- **Dedicated blob route/page.** A `/{*entity}@blob` route + full-page viewer concept (mirroring `artifact/route`) for "open this file" links. Inline-in-views was chosen first.
- **Streaming the blob body.** Task 1 buffers the whole blob into a `Vec<u8>`. Fine for images; large files want a `Stream`-backed `Body` (`futures_util::stream::unfold` over `reader.next()`).
- **In-browser blob upload.** There is no HTTP route to *add* a blob today (only the CLI's `Blob::import`). Viewing works for synced-in blobs; a browser upload route is a separate feature.

---

## Self-Review

- **Spec coverage.** Chosen scope = inline-in-views, images only, plan-first. Task 1 serves bytes; Task 2 renders inline `<img>`; Task 3 verifies end-to-end + docs. The dedicated route, other MIME types, and tonk-render parity are explicitly deferred, matching "images only / inline first." ✔
- **Type consistency.** URL is identical in both tasks: `/api/repository/{repo}/branch/{branch}/blob/{entity}` with `{entity}` = full `blob:<hash>`. Task 1's `BlobPath.entity` is parsed with `.parse::<Entity>()` then `blob_hash()`-checked; Task 2 passes the raw `entity` attribute (also `blob:<hash>`) — same string on both ends. `with` format `{branch}@{repo}` (branch before `@`) matches the yaml route views and the split in `blob_image_src`. ✔
- **No placeholders.** Every code step shows complete code; every run step gives a command + expected result. The one hand-authored step (Task 3 verification) is inherently manual and is written as concrete browser/devtools checks. ✔
- **Risks flagged.** (a) `model` attribute equals the literal `tonk:blob` URI — if a caller aliases the model to a resolved DID, add a secondary check on the resolved model entity. (b) `write_blob`/`from_blob`/`hash.as_bytes()` in Task 1's test come from the access-service integration test (`rust/tonk-access-service/tests/ucan_integration.rs:257-261,313-324`); if signatures differ on the pinned `d2af7b7`, adapt to the entity-keyed form `Blob::import(...).write(branch.blobs())` used by `rust/tonk-cli/src/blob.rs:125`. (c) tonk-worker/tonk-display tests are browser/wasm — they only run under `test:web:debug`, which needs the chromedriver/Safari automation this repo documents.
