# `<tonk-upload>` — Web-UI Blob Upload Design

**Status:** approved (2026-07-09)
**Depends on:** the blob feature on `feat/tonk-blobs` (PR #561) — the `tonk:blob` concept, the read route `GET …/blob/{entity}` (`router/blob.rs::serve`), and the `<tonk-display model=tonk:blob>` `<img>` renderer. This feature branches off `feat/tonk-blobs`.

## Goal

A native, reusable web-UI primitive that ingests a user-picked file into the current branch's content-addressed blob store and surfaces the resulting `blob:<hash>` reference. The primitive is **blob-agnostic**: it ingests and emits; it does not know what the blob is "for." Views compose it and decide what to do with the ref.

Non-goal (v1): binding the upload to a specific entity/field. That layers on later by wiring the emitted event into a `/transact` — no change to this element.

## Why native, not a yaml component

A file picker needs asynchronous JS (`FileReader`/`file.arrayBuffer()`), which declarative `<tonk-display>` views cannot run (views never execute scripts). The two homes that *can* run it are a yaml `component!:` (branch data, hot-swappable) or a native custom element. We chose **native**: it's available in every space with no per-space seeding, it's a trusted first-party primitive on par with `<tonk-display>`, and it's consistent with the native `tonk:blob` `<img>` renderer shipped in #561. Cost accepted: a Rust rebuild to change it (not hot-swappable).

## Architecture — two parts

### Part 1 — Worker ingest route

`POST /api/repository/{repo}/branch/{branch}/blob` — a new handler in `rust/tonk-worker/src/router/blob.rs`, registered next to the existing `get(blob::serve)` at `router.rs`.

- **Extractors:** `State(AppState)`, `Path { repo, branch }`, `headers: HeaderMap`, `body: Bytes` (the buffered file bytes — the established body-read pattern used by `transfer::import`, `claim::assert_claim`, `transact::transact`).
- **MIME:** taken from the request `Content-Type` header. Absent/empty → `application/octet-stream`.
- **Filename:** taken from an `X-Tonk-Blob-Name` request header. Absent → no name fact. *(Decision: a header, not a `?name=` query param or `multipart/form-data` — keeps the body a raw byte stream matching the CLI's streaming ingest, and avoids a multipart parser. The relay passes headers through.)*
- **Ingest:** `Blob::import(stream::once(async move { Ok::<_, dialog_effects::blob::BlobError>(body.to_vec()) })).write(branch.blobs()).perform(&tonk.operator)` → `blob:<hash>` `Entity`. Branch is opened exactly as `serve` does (`profile.repository(repo).load()` → `repo.branch(branch).open()`). Idempotent by content address.
- **Facts:** the route **asserts** `xyz.tonk.blob/content-type` (always) and `xyz.tonk.blob/name` (when the header is present) on the blob entity, in one reactor transaction, then broadcasts the new revision. *(Decision: the route asserts, rather than returning a bare entity for the caller to assert — this matches `tonk blob add` exactly, so the read route and `<tonk-display model=tonk:blob>` work immediately, and it's one round-trip instead of two.)* The commit path mirrors `transfer::import` (commit a body, then broadcast so subscriptions re-poll).
- **Empty body:** rejected with `400`.
- **Response:** `200` with JSON `{ "entity": "blob:<hash>", "contentType": "<mime>", "name": "<name-or-null>", "size": <bytes> }`.
- **Errors:** mirror `serve`'s mapping — malformed input → `Router`(400), branch/repo failures → `NotFound`/`Internal`, using the same `TonkWorkerError` variants.
- **Buffered, not streamed** in v1 — the same explicit limitation as the read route (`blob.rs` buffers the whole blob on read). A `BodyStream` refinement is a later pass.

### Part 2 — Native `<tonk-upload>` element

New module `rust/tonk-display/src/upload.rs`, `register()`ed into the guest bundle alongside `<tonk-display>` (via `tonk_display::register()` → guest).

- **Attributes:**
  - `with="{branch}@{repo}"` — the branch context, forwarded by the embedding view exactly like `<tonk-display>`. The element builds the ingest/read URLs from it, reusing the same `{branch}@{repo}` split logic as `blob_image_src` (factor that helper so both share it). If `with` is missing or an unsubstituted `{…}` template, the element renders a disabled state with a status note (it can't know where to POST).
  - `accept` (optional) — passed straight to the file input's `accept` attribute (e.g. `image/*`).
- **DOM:** renders a real `<input type="file">` (plus a small status/preview area). Native-element DOM runs, unlike inert view templates.
- **On pick (single file):**
  1. Read bytes: `await file.arrayBuffer()`.
  2. Show an **instant local preview** via `URL.createObjectURL(file)` (revoked after swap) so the user sees the image immediately.
  3. POST: `fetch('/api/repository/{repo}/branch/{branch}/blob', { method:'POST', body: <ArrayBuffer>, headers:{ 'Content-Type': file.type || 'application/octet-stream', 'X-Tonk-Blob-Name': file.name } })`. From a sealed guest this is relayed through the portal bridge — binary bodies survive the `postMessage` structured clone, and the request **must** use `init.body` (not a `Request` object, which the guest normalizer would `.text()`).
  4. On `2xx`: parse `{ entity, contentType, name, size }`. Swap the preview to the **read-route** URL `/api/repository/{repo}/branch/{branch}/blob/<entity>` — proving the bytes were actually stored and are retrievable (not just a local file). For non-image `contentType`, show name + size instead of an `<img>`.
  5. **Emit:** dispatch a bubbling, composed `CustomEvent('tonk-upload')` with `detail = { blob: entity, contentType, name, size }`.
- **Status line:** idle → "reading…" → "uploading…" → done / error text.

### Data flow

```
pick file
  → read bytes (arrayBuffer)
  → instant local preview (object URL)
  → POST /…/blob  (relayed through the bridge for a sealed guest)
    → worker: Blob::import → assert content-type[+name] facts → broadcast
    → 200 { entity, contentType, name, size }
  → element: swap preview to read-route URL, emit `tonk-upload` {detail}
  → (view author, LATER) binds `tonk-upload` → /transact to record the ref on some entity
```

## Error handling

- **Worker:** empty body → 400; unreadable/failed ingest → 500 JSON error; malformed repo/branch/headers → 400, matching `serve`.
- **Element:** `arrayBuffer()`/read failure, fetch rejection, or non-2xx → status line shows the message, the local preview is cleared, and **no `tonk-upload` event is emitted** (a consumer never sees a phantom ref). Missing/unsubstituted `with` → disabled input + status note.

## Testing

- **Worker route** (`#[dialog_common::test]`, wasm service-worker, in `router/blob.rs`): POST bytes with a `Content-Type` and `X-Tonk-Blob-Name` → assert `200` + a `blob:` entity in the JSON; then `GET …/blob/<entity>` through `serve` and assert the bytes round-trip and the `Content-Type` came back from the asserted fact. A second POST of the same bytes returns the same entity (idempotence). An empty-body POST → 400.
- **Element** (`#[dialog_common::test]`, browser, in `upload.rs`): mount `<tonk-upload with="main@repo">`, drive a synthetic `File` through the input, stub the relayed fetch to return a canned `{entity,…}`, and assert (a) the `tonk-upload` event fires once with the expected `detail`, (b) the preview `<img>` `src` becomes the read-route URL, (c) a failed fetch emits **no** event and shows an error status.
- **Manual e2e:** a small demo view (e.g. in `demo.yaml`) mounting `<tonk-upload>` and capturing the event, exercised in `dev:web` (which now runs a local blob-aware access service).

## Out of scope (v1)

Drag-and-drop, multi-file selection, upload progress, streaming ingest, and upload-bound-to-an-entity/field. The last one is the natural next layer: a view binds the emitted `tonk-upload` event to a `/transact` that writes `blob:<hash>` onto a target entity's attribute.

## File-level plan (informs the implementation plan)

- `rust/tonk-worker/src/router/blob.rs` — add the `POST` handler + its test; register the route in `router.rs`.
- `rust/tonk-display/src/upload.rs` — new element + test; `register()` from the crate's `register()` and the guest bundle.
- `rust/tonk-display/src/element.rs` (or a shared helper module) — factor the `{branch}@{repo}` → URL logic out of `blob_image_src` so `<tonk-upload>` reuses it.
- `rust/tonk-core/assets/library/demo.yaml` — a demo view mounting `<tonk-upload>` (manual e2e only).
