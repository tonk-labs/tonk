# Declarative Blob Renderer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Blob rendering becomes a seeded, user-overridable view + web component in core.yaml (images inline, everything else a file card with lazy download), with the native Rust image path demoted to an empty-frame fallback.

**Architecture:** A `view!:` row for `model: tonk:blob` in core.yaml carries a `tonk-blob-media` web component inline (via `<tonk-component>`'s inert script holder); the component dispatches on `content-type` and fetches bytes through the relayed `window.fetch` → `URL.createObjectURL` (native `src=/api/…` loads bypass the relay inside the sealed guest). In `tonk-display`, the hardcoded `model == "tonk:blob"` early return moves into the empty-view-frame path and registers as a default slide, so any resolved view replaces it. Both write paths always assert the `xyz.tonk.blob/name` fact, because the concept query behind the seeded view matches only rows with every field present.

**Tech Stack:** Rust (axum-on-wasm worker, wasm-bindgen custom elements), YAML-seeded declarative views, vanilla JS web component as branch data.

**Spec:** `docs/superpowers/specs/2026-07-15-declarative-blob-renderer-design.md`

## Global Constraints

- Tests use `#[dialog_common::test]` (never `#[test]`/`#[tokio::test]` in tonk-* crates); names are `it_does_x`. Wasm test mods need `wasm_bindgen_test_configure!` (see `.claude/skills/testing/SKILL.md`).
- Native tests: `nix develop -c test:native:debug`. Wasm/browser tests: `nix develop -c test:web:debug` (headless Chrome via chromedriver; on this darwin machine Chrome must be at `/Applications` with a major-matched chromedriver — see repo memory if it fails to launch).
- Lint gate before every commit: `cargo fmt --all` and `cargo clippy --workspace --all-targets --all-features` must be clean.
- Commit with jj, not raw git mutation: `jj commit -m "<type>(<scope>): <subject>" <paths>`. Conventional Commits, imperative, lowercase, no trailing period.
- No `mod.rs`; no emojis; no "per the spec/RFC" references in code or comments — code stands on its own.
- The eventual PR targets `staging` (repo default), not `main`.

---

### Task 1: Name fact totality (worker `POST /blob` + CLI `tonk blob add`)

Every uploaded blob must end up with an `xyz.tonk.blob/name` fact: the `tonk:blob` concept query behind the seeded view (Task 3) matches only rows with every field present, so a nameless blob would never render. An explicit name (header / file name) wins; a raw re-upload without a header must *not* clobber an existing name with the hash default.

**Files:**
- Modify: `rust/tonk-worker/src/router/blob.rs` (the `upload` handler, `BlobUploadResponse`, and its `tests` mod)
- Modify: `rust/tonk-cli/src/blob.rs` (the `add` function, ~line 139)

**Interfaces:**
- Consumes: existing `RawClaim`, `ArtifactSelector`, `branch_handle.claims()` (same query shape as `serve`'s content-type lookup in the same file).
- Produces: `BlobUploadResponse.name` changes from `Option<String>` to `String` (always present). Task 3's seeded view relies on the name fact existing for every uploaded blob.

- [ ] **Step 1: Write the failing worker tests**

In the `tests` mod of `rust/tonk-worker/src/router/blob.rs`, add:

```rust
    #[dialog_common::test]
    async fn it_defaults_the_name_fact_to_the_entity_when_no_header_is_sent() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-name-default").await;

        // Upload with no X-Tonk-Blob-Name header.
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .body(Body::from(b"%PDF-1.4 nameless".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        let body = axum::body::to_bytes(up.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entity = json["entity"].as_str().unwrap().to_string();
        assert_eq!(
            json["name"], entity,
            "no header: the name fact defaults to the content-addressed entity string",
        );
    }

    #[dialog_common::test]
    async fn it_preserves_an_existing_name_on_a_headerless_reupload() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-name-keep").await;
        let payload = b"%PDF-1.4 named".to_vec();

        // First upload names the blob.
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .header("x-tonk-blob-name", "report.pdf")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);

        // A headerless re-upload of the same bytes keeps the asserted name
        // rather than clobbering it with the hash default.
        let up2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body2 = axum::body::to_bytes(up2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(
            json2["name"], "report.pdf",
            "headerless re-upload preserves the existing name fact",
        );
    }
```

These tests are wasm service-worker tests (the mod is `#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]` and already has `wasm_bindgen_test_configure!(run_in_service_worker)` — new tests slot in beside `it_uploads_bytes_and_serves_them_back`).

- [ ] **Step 2: Run the wasm tests to verify they fail**

Run: `nix develop -c test:web:debug`
Expected: the two new tests FAIL (`json["name"]` is `null` today when no header is sent); everything else passes.

- [ ] **Step 3: Implement the worker change**

In `upload` in `rust/tonk-worker/src/router/blob.rs`:

1. Change the response struct field from `pub name: Option<String>` to `pub name: String` and update its doc comment to: `/// File name recorded for the blob (header value, existing fact, or the entity string).`
2. After `let entity = Blob::import(...)` succeeds and before the metadata transaction, resolve the effective name (the `name` local is currently `Option<String>` from the header):

```rust
    // Effective name: an explicit header wins; otherwise an already-
    // asserted name fact is preserved (a raw re-upload must not clobber
    // a good name with the hash default); otherwise the entity string.
    // The name fact must always land — the `tonk:blob` concept query
    // matches only rows with every field present, so a nameless blob
    // would never reach the seeded media view.
    let name_attr: Attribute = "xyz.tonk.blob/name"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
    let name = match name {
        Some(n) => n,
        None => {
            let existing = branch_handle
                .claims()
                .select(
                    ArtifactSelector::new()
                        .the(name_attr.clone())
                        .of(entity.clone()),
                )
                .perform(&tonk.operator)
                .await
                .map_err(|e| TonkWorkerError::Internal(format!("name query: {e}")))?;
            tokio::pin!(existing);
            match existing.next().await {
                Some(Ok(artifact)) => {
                    String::try_from(artifact.is).unwrap_or_else(|_| entity.to_string())
                }
                _ => entity.to_string(),
            }
        }
    };
```

3. Replace the conditional name assert (`if let Some(n) = &name { ... }`) with an unconditional one, reusing `name_attr` (delete the second `"xyz.tonk.blob/name".parse()` inside the old `if let`):

```rust
    let tx = tonk
        .reactor
        .repository(&path.repo)
        .branch(&path.branch)
        .transaction()
        .assert(RawClaim {
            the: ct_attr,
            of: entity.clone(),
            is: Value::String(content_type.clone()),
            unique: true,
        })
        .assert(RawClaim {
            the: name_attr,
            of: entity.clone(),
            is: Value::String(name.clone()),
            unique: true,
        });
    tx.commit()
```

4. The `BlobUploadResponse { name, .. }` construction now passes the `String` directly. Update the `upload` doc comment's parenthetical about the name header to say the fact is always asserted, defaulting to an existing fact or the entity string.
5. In the existing test `it_uploads_bytes_and_serves_them_back`, the second (headerless) upload's assertions gain one line — the name is preserved from the first upload:

```rust
        assert_eq!(json2["name"], "shot.png", "re-upload preserves the name fact");
```

- [ ] **Step 4: Implement the CLI change**

In `add` in `rust/tonk-cli/src/blob.rs` (~line 139), replace:

```rust
    let mut tx = session
        .handle()
        .transaction()
        .assert(ContentType::of(entity.clone()).is(content_type.clone()));
    if let Some(n) = name {
        tx = tx.assert(Name::of(entity.clone()).is(n));
    }
```

with:

```rust
    // `name` always lands: the `tonk:blob` concept query behind the
    // seeded media view matches only rows with every field present, so
    // a nameless blob would never render. A path with no file name
    // (`.`/`..`) falls back to the content-addressed entity string.
    let tx = session
        .handle()
        .transaction()
        .assert(ContentType::of(entity.clone()).is(content_type.clone()))
        .assert(Name::of(entity.clone()).is(name.unwrap_or_else(|| entity.to_string())));
```

No new CLI test: the `None` arm is unreachable through the CLI in practice (`File::open` on a directory path fails before the assert), and contorting a fixture to reach it tests nothing real. The doc comment on `add` (~line 92) changes from "(when `path` has a file name)" to "(always; defaults to the entity string when `path` has no file name)".

- [ ] **Step 5: Run tests to verify they pass**

Run: `nix develop -c test:web:debug` (worker tests) and `nix develop -c test:native:debug` (CLI tests in `rust/tonk-cli/tests/blob.rs` still pass).
Expected: PASS.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features
jj commit -m "feat(tonk-worker): always assert the blob name fact on upload" rust/tonk-worker/src/router/blob.rs rust/tonk-cli/src/blob.rs
```

---

### Task 2: Demote the native blob `<img>` to an empty-frame default slide

Today `handle_view_frame` returns early for `model == "tonk:blob"` before view resolution, so no view for that model can ever mount. Move the branch into the empty-frame path and register the `<img>` as a default slide (the `mount_notation_fallback` pattern), so a resolved view — the seeded media view or a user override — replaces it through ordinary slide reconciliation.

**Files:**
- Modify: `rust/tonk-display/src/element.rs` (`handle_view_frame` ~line 1203, `handle_blob_image_frame` ~line 1542, plus the wasm `tests` mod)

**Interfaces:**
- Consumes: `Inner.slides`, `Inner.default_slide`, `Slide` struct, `blob_image_src`, `crate::blob_url::fetch_object_url` — all already in this file.
- Produces: `mount_blob_fallback_frame(host: &Element, state: &Rc<RefCell<Inner>>)` (renamed from `handle_blob_image_frame`, gains the `state` param). Behavior contract for Task 3: a non-empty view frame for `model=tonk:blob` mounts normally and removes the fallback `<img>`.

- [ ] **Step 1: Write the failing wasm test**

In the wasm tests mod of `element.rs`, next to `it_fetches_blob_bytes_and_mounts_an_object_url_img` (~line 3327), add (reusing that test's setup verbatim — same `blob_resolve_responses` fixture with the three-field view-concept `source`):

```rust
        /// A model-specific view for `tonk:blob` (the seeded media view, or
        /// a user override) wins over the native `<img>` fallback: the
        /// fallback is registered as a default slide, so a non-empty view
        /// frame replaces it through ordinary slide reconciliation.
        #[dialog_common::test]
        async fn it_replaces_the_native_blob_img_when_a_view_resolves() {
            let requested: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
            stub_fetch_bytes(requested.clone());
            let blob_resolve_responses = vec![
                name_row("did:key:zViewConcept"),
                rows(&[(
                    "did:key:zViewConcept",
                    &[(
                        "source",
                        r#"{"with":{"model":{"the":"xyz.tonk.view/model","as":"Entity","cardinality":"one"},"display":{"the":"xyz.tonk.view/display","as":"Text","cardinality":"one"},"type":{"the":"xyz.tonk.view/type","as":"Text","cardinality":"one"}}}"#,
                    )],
                )]),
            ];
            let host =
                FakeHost::install_with_model(blob_resolve_responses, Some(model_concept_frame()));
            let display = mount_display(&host, "counter", "tonk:blob", "blob:zHASH");
            display.set_attribute("with", "main@myrepo").unwrap();
            for _ in 0..200 {
                if host.subscribe_tags().len() >= 3 {
                    break;
                }
                sleep(5).await;
            }

            // Empty view frame first: the native fallback mounts.
            host.push_frame("view", &rows(&[]));
            assert!(
                await_selector(&display, "img").await.is_some(),
                "an empty view frame mounts the native <img> fallback",
            );

            // A model-specific view lands: it replaces the fallback.
            host.push_frame(
                "view",
                &rows(&[("did:key:zBlobView", &[("display", "<p>media view</p>")])]),
            );
            assert!(
                await_selector(&display, "tonk-view").await.is_some(),
                "the non-empty view frame mounts a <tonk-view> slide",
            );
            let mut img_gone = false;
            for _ in 0..200 {
                if display.query_selector("img").unwrap().is_none() {
                    img_gone = true;
                    break;
                }
                sleep(5).await;
            }
            assert!(
                img_gone,
                "the native fallback <img> is removed once the view mounts",
            );
        }
```

- [ ] **Step 2: Run the wasm tests to verify the new test fails**

Run: `nix develop -c test:web:debug`
Expected: `it_replaces_the_native_blob_img_when_a_view_resolves` FAILS — today the blob branch returns early on every view frame, so no `<tonk-view>` ever mounts and the `<img>` never goes away. `it_fetches_blob_bytes_and_mounts_an_object_url_img` still PASSES.

- [ ] **Step 3: Implement the reshuffle**

In `handle_view_frame` (~line 1203), delete the early-return blob branch:

```rust
    // DELETE this block:
    if host.get_attribute("model").as_deref() == Some("tonk:blob") {
        drop(s);
        handle_blob_image_frame(host);
        dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
        return;
    }
```

and re-add the check inside the `conclusions.is_empty()` branch (~line 1233), before the `need_default` logic:

```rust
    if conclusions.is_empty() {
        // A `tonk:blob` model with no model-specific view falls back to
        // the native single-`<img>` renderer instead of the `_:_`
        // default view: a blob's payload is bytes, not facts, so the
        // generic fallbacks have nothing to show. Registered as a
        // default slide, so a later non-empty view frame (the seeded
        // media view, or a user override) replaces it through ordinary
        // slide reconciliation.
        if host.get_attribute("model").as_deref() == Some("tonk:blob") {
            drop(s);
            mount_blob_fallback_frame(host, state);
            dispatch_event(host, "tonk-display:template", Some(JsValue::from_str("ok")));
            return;
        }
        let need_default = !s.default_slide;
        drop(s);
        if need_default {
            spawn_default_view(host, state);
        }
        return;
    }
```

Rename `handle_blob_image_frame` to `mount_blob_fallback_frame`, add the `state` parameter, and register the `<img>` as the default slide right after the create-or-find block (before the `data-blob-url` dedupe check):

```rust
/// Mount (or refresh) a single `<img>` whose `src` is the worker's
/// content-addressed blob route for this display's `entity`, registered
/// as a default slide (key `"__blob__"`) so a model-specific view frame
/// replaces it through ordinary reconciliation. Idempotent: the route
/// URL is stable for a given `(with, entity)`, so re-running on a later
/// empty frame is a no-op once the element exists.
fn mount_blob_fallback_frame(host: &Element, state: &Rc<RefCell<Inner>>) {
    let Some(url) = blob_image_src(host) else {
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
    {
        let mut s = state.borrow_mut();
        if s.disposed {
            return;
        }
        if !s.slides.contains_key("__blob__") {
            s.slides.insert(
                "__blob__".to_owned(),
                Slide {
                    display: String::new(),
                    item: img.clone(),
                    view_el: img.clone(),
                },
            );
        }
        s.default_slide = true;
    }
    // ... rest of the function body unchanged (data-blob-url dedupe,
    // relayed fetch → object URL, state::set(host, State::Ready)) ...
}
```

The existing removal loops in `handle_view_frame` do the rest: a non-empty frame clears `default_slide` (~line 1245) and drops `"__blob__"` as a vanished key, removing the `<img>` from the DOM. (The removed `<img>`'s object URL is not revoked — one object URL per replaced fallback leaks until reload; accepted, matching how replaced slides are dropped generally.)

- [ ] **Step 4: Run the wasm tests to verify both blob tests pass**

Run: `nix develop -c test:web:debug`
Expected: PASS, including the pre-existing `it_fetches_blob_bytes_and_mounts_an_object_url_img` (its empty-frame push now reaches the fallback via the new path).

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features
jj commit -m "feat(tonk-display): demote native blob img to overridable default slide" rust/tonk-display/src/element.rs
```

---

### Task 3: Seed the declarative media view + `tonk-blob-media` component (core.yaml)

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml` — insert immediately after the `tonk:blob` concept block (ends ~line 1735, `as: text`)
- Test: `rust/tonk-worker/tests/standard_library.rs` (existing lowering tests cover the new rows automatically)

**Interfaces:**
- Consumes: `<tonk-component>` inert script-holder execution (`rust/tonk-display/src/component.rs`); `{dom.host/with}` host-attribute binding (`host_attr_fields`, `element.rs:2298`); the always-present `content-type`/`name` facts from Task 1; the worker blob route `GET /api/repository/{repo}/branch/{branch}/blob/{entity}`.
- Produces: view entity `tonk:blob/media-view` (stable `this:` — re-asserting replaces it; that is the user-override mechanism) defining the `tonk-blob-media` custom element with `::part` hooks `media`, `card`, `name`, `badge`, `action`.

- [ ] **Step 1: Add the view to core.yaml**

Insert after the `tonk:blob` concept block:

```yaml
# ============================================================
# BLOB MEDIA VIEW
# ============================================================

# The standard renderer for `tonk:blob` entities, seeded as ordinary
# branch data so it is overridable: re-assert a view with the same
# `this:` to replace it (the display element also keeps a native
# `<img>` fallback for branches without this row). Image content
# types render inline; every other type renders as a file card with
# a lazy download action.
#
# The renderer is author JavaScript riding the view as an inert
# `<script type="tonk/module">` holder, executed once per realm by
# `<tonk-component>`. Bytes are fetched through the relayed
# `window.fetch` and handed to the media element as an object URL —
# inside the sealed guest a native `src=/api/…` load bypasses the
# relay and the service worker. The blob route is scoped by the
# host's `with` attribute ("{branch}@{repo}"), forwarded through the
# `{dom.host/with}` binding.
view!:
  this: tonk:blob/media-view
  model: tonk:blob
  display: |
    <div subject={this}>
      <tonk-component>
        <script type="tonk/module">
          customElements.get("tonk-blob-media") ||
            customElements.define("tonk-blob-media", class extends HTMLElement {
              static observedAttributes = ["entity", "content-type", "name", "with"];
              #objectUrl = null;
              connectedCallback() { this.render(); }
              attributeChangedCallback() { if (this.isConnected) this.render(); }
              disconnectedCallback() { this.revoke(); }
              revoke() {
                if (this.#objectUrl) {
                  URL.revokeObjectURL(this.#objectUrl);
                  this.#objectUrl = null;
                }
              }
              url() {
                const scope = this.getAttribute("with") || "";
                const entity = this.getAttribute("entity") || "";
                const at = scope.indexOf("@");
                if (at < 1 || !entity || scope.includes("{")) return null;
                return "/api/repository/" + scope.slice(at + 1) +
                  "/branch/" + scope.slice(0, at) + "/blob/" + entity;
              }
              async objectUrl() {
                const url = this.url();
                if (!url) return null;
                const resp = await window.fetch(url);
                if (!resp.ok) return null;
                return URL.createObjectURL(await resp.blob());
              }
              render() {
                const key = [
                  this.getAttribute("with"),
                  this.getAttribute("entity"),
                  this.getAttribute("content-type"),
                  this.getAttribute("name"),
                ].join("|");
                if (this.dataset.rendered === key) return;
                this.dataset.rendered = key;
                const root = this.shadowRoot || this.attachShadow({ mode: "open" });
                const type = this.getAttribute("content-type") || "";
                const name = this.getAttribute("name") || this.getAttribute("entity") || "";
                this.revoke();
                if (type.startsWith("image/")) {
                  root.innerHTML = "<style>img { max-width: 100%; }</style>" +
                    "<img part=\"media\">";
                  root.querySelector("img").alt = name;
                  this.objectUrl().then((u) => {
                    if (!u) return;
                    this.#objectUrl = u;
                    root.querySelector("img").src = u;
                  });
                  return;
                }
                root.innerHTML =
                  "<style>" +
                  "[part=card] { display: flex; align-items: center; gap: 0.75rem; " +
                  "padding: 0.75rem 1rem; border: 1px solid #d0d0d0; border-radius: 8px; " +
                  "max-width: 28rem; font-family: system-ui, sans-serif; }" +
                  "[part=name] { flex: 1; overflow-wrap: anywhere; }" +
                  "[part=badge] { font-size: 0.75rem; color: #666; border: 1px solid #ddd; " +
                  "border-radius: 4px; padding: 0.1rem 0.4rem; white-space: nowrap; }" +
                  "[part=action] { cursor: pointer; }" +
                  "</style>" +
                  "<div part=\"card\"><span part=\"name\"></span>" +
                  "<span part=\"badge\"></span>" +
                  "<button part=\"action\" type=\"button\">Download</button></div>";
                root.querySelector("[part=name]").textContent = name;
                root.querySelector("[part=badge]").textContent = type || "file";
                root.querySelector("[part=action]").addEventListener("click", async () => {
                  const u = await this.objectUrl();
                  if (!u) return;
                  const a = document.createElement("a");
                  a.href = u;
                  a.download = name;
                  a.click();
                  URL.revokeObjectURL(u);
                });
              }
            });
        </script>
      </tonk-component>
      <tonk-blob-media entity={this} content-type={content-type} name={name} with={dom.host/with}></tonk-blob-media>
    </div>
```

Notes for the implementer (why the code is shaped this way — do not paraphrase into the yaml):
- The template walker never descends into `<script>` (raw-text treatment), so the JS braces and `{…}` are safe from binding interpolation; the four `{…}` bindings on `<tonk-blob-media>` are outside the script and are the only interpolation points.
- `customElements.get(...) ||` guard: the loader deduplicates by content hash but cannot redefine a name; an edited component takes effect on reload.
- The `dataset.rendered` key makes re-renders (attribute echoes, view re-mounts replaying the frame) no-ops unless an input actually changed.

- [ ] **Step 2: Run the standard-library lowering tests**

Run: `nix develop -c cargo test -p tonk-worker --test standard_library`
Expected: PASS — `it_lowers_the_standard_library` (and the template-concatenation tests) now lower the new `view!:` row; a yaml or notation error here fails loudly with a parse/analyze diagnostic.

- [ ] **Step 3: Run the full native + wasm suites**

Run: `nix develop -c test:native:debug`, then `nix develop -c test:web:debug`
Expected: PASS. In particular the Task 2 display tests still pass — they fake the view frames, so the new seeded row doesn't perturb them.

- [ ] **Step 4: Commit**

```bash
jj commit -m "feat(tonk-core): seed declarative tonk:blob media view with file card" rust/tonk-core/assets/library/core.yaml
```

---

### Task 4: Live verification in tonk-ui

The component JS is branch data — not reachable by Rust unit tests — and the one open sandbox question (does the sealed guest need `allow-downloads`?) is only answerable in a real browser. This task produces no code unless the download is blocked.

**Files:**
- None expected. Contingency: the sealed-guest iframe's `sandbox` attribute (grep `allow-scripts` under `rust/` — e.g. `rust/tonk-render/src/page/orchestrate.rs` builds `<iframe sandbox="allow-scripts" …>`; the tonk-ui shell guest has its own) gains `allow-downloads` if the download click is blocked.

- [ ] **Step 1: Start the dev stack**

Run: `nix develop -c dev:web` (dev server + local access service). Create a fresh space through the UI (any template).

- [ ] **Step 2: Upload and render an image**

Upload a PNG through the space's `<tonk-upload>` flow (or `tonk blob add <file.png>` against the same repo/branch, then navigate to the blob entity). Verify: the blob renders through the seeded view — inspect the DOM and confirm the `<img>` lives inside `<tonk-blob-media>`'s shadow root (declarative path), not as a bare `<img>` child of `<tonk-display>` (native fallback). Its `src` is a `blob:` object URL.

- [ ] **Step 3: Upload and render a PDF**

Upload a PDF the same way. Verify: the file card renders (name, `application/pdf` badge, Download button). Click Download. If the file saves — done. If the browser blocks it (console warning about sandboxed downloads), add `allow-downloads` to the guest iframe's `sandbox` attribute, rebuild, re-verify, and commit that one-line change as `fix(tonk-ui): allow downloads from the sealed guest`.

- [ ] **Step 4: Verify the override mechanism**

Re-assert the view with the same `this:` and a trivially different template (e.g. via `tonk` CLI asserted-notation or slide, changing the card's button label), reload, and confirm the change took — proving `tonk:blob/media-view` is user-replaceable data.

- [ ] **Step 5: Record results**

Note the outcome of each check (and the `allow-downloads` finding) in the PR description. If any check fails, stop and debug before proceeding to a PR — do not paper over with the native fallback.

---

## Self-Review

- **Spec coverage:** seeded view + inline component (Task 3), content-type dispatch img/card with lazy download (Task 3 JS), dispatch reshuffle to default-slide fallback (Task 2), name totality worker + CLI (Task 1), lowering + wasm tests (Tasks 1–3), live verification incl. `allow-downloads` (Task 4), out-of-scope items untouched. No gaps found.
- **Placeholder scan:** the only elision is Task 2 Step 3's "rest of the function body unchanged", which names exactly which existing lines remain (dedupe, fetch, state set) — the code exists in the file being edited.
- **Type consistency:** `mount_blob_fallback_frame(host, state)` defined in Task 2 and referenced nowhere else; `BlobUploadResponse.name: String` used consistently in Task 1's tests; part names `media/card/name/badge/action` consistent between Task 3's interface block and JS.
