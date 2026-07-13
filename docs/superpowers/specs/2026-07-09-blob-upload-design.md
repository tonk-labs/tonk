# `<tonk-upload>` — Web-UI Blob Upload Design

**Status:** approved (2026-07-09)
**Depends on:** the blob feature on `feat/tonk-blobs` (PR #561) — the `tonk:blob` concept, the read route `GET …/blob/{entity}` (`router/blob.rs::serve`), and the `<tonk-display model=tonk:blob>` `<img>` renderer. This feature branches off `feat/tonk-blobs`.

## Goal

A native, reusable web-UI primitive that ingests a user-picked file into the current branch's content-addressed blob store and surfaces the resulting `blob:<hash>` reference. The primitive is **blob-agnostic**: it ingests and emits; it does not know what the blob is "for." Views compose it and decide what to do with the ref.

It is a **headless primitive with a nice default UI**: it ships a clean, usable appearance so it works out of the box with zero authoring, and every visible piece is overridable by the view author (slots, `::part()`, CSS custom properties, state) with no Rust changes.

Non-goal (v1): binding the upload to a specific entity/field. That layers on later by wiring the emitted event into a `/transact` — no change to this element.

## Why a native headless element (not a yaml component)

A file picker needs asynchronous JS (`FileReader`/`file.arrayBuffer()`), which declarative `<tonk-display>` views cannot run. The two homes that can are a yaml `component!:` (branch data) or a native custom element. We chose **native** because the user wants a strong primitive available in every space out of the box, with no per-space seeding, on par with `<tonk-display>` and consistent with the native `tonk:blob` `<img>` renderer shipped in #561.

The usual objection to native — "you lose author control over the UI" — is answered by making it **headless**: the mechanism lives in Shadow DOM, and presentation is exposed through the standard web-component customization surface. Shadow DOM is the hinge — `<tonk-display>` reconciles light DOM, not shadow DOM, so the element can hold live UI inside a view without being clobbered. This is exactly how the library's `wa-*` (WebAwesome) components already work: shadow-DOM primitives restyled from view CSS via `::part()` (e.g. `wa-button::part(base)`, `.fab__invite::part(button)`). The security boundary is unaffected either way: bytes only enter the store through the UCAN-authorized worker route; the UI just calls it.

## Architecture — two parts

### Part 1 — Worker ingest route

`POST /api/repository/{repo}/branch/{branch}/blob` — a new handler in `rust/tonk-worker/src/router/blob.rs`, registered alongside the existing `get(blob::serve)` in `router.rs`. (Distinct path from the read route `GET …/blob/{entity}`.)

- **Extractors:** `State(AppState)`, `Path { repo, branch }`, `headers: HeaderMap`, `body: Bytes` — the established body-read pattern (`transfer::import`, `claim::assert_claim`, `transact::transact`).
- **MIME:** from the request `Content-Type` header; absent/empty → `application/octet-stream`.
- **Filename:** from an `X-Tonk-Blob-Name` request header; absent → no name fact. *(Decision: a header, not a `?name=` query param or `multipart/form-data` — keeps the body a raw byte stream matching the CLI's streaming ingest and avoids a multipart parser; the relay passes headers through.)*
- **Ingest:** `Blob::import(stream::once(async move { Ok::<_, dialog_effects::blob::BlobError>(body.to_vec()) })).write(branch.blobs()).perform(&tonk.operator)` → `blob:<hash>` `Entity`. Branch opened exactly as `serve` does. Idempotent by content address.
- **Facts:** the route **asserts** `xyz.tonk.blob/content-type` (always) and `xyz.tonk.blob/name` (when the header is present) on the blob entity in one reactor transaction, then broadcasts the new revision. *(Decision: the route asserts rather than returning a bare entity — matches `tonk blob add`, so the read route and `<tonk-display model=tonk:blob>` work immediately, in one round-trip.)* Commit + broadcast mirror `transfer::import`.
- **Empty body:** rejected with `400`.
- **Response:** `200` with JSON `{ "entity": "blob:<hash>", "contentType": "<mime>", "name": "<name-or-null>", "size": <bytes> }`.
- **Errors:** mirror `serve`'s `TonkWorkerError` mapping (malformed → `Router`/400, repo/branch failures → `NotFound`/`Internal`).
- **Buffered, not streamed** in v1 — same explicit limitation as the read route; a `BodyStream` refinement is a later pass.

### Part 2 — Native headless `<tonk-upload>` element

New module `rust/tonk-display/src/upload.rs`, `register()`ed into the guest bundle alongside `<tonk-display>` (via `tonk_display::register()`). The mechanism (file input, ingest fetch, state machine, preview management) lives in **Shadow DOM**; the presentation is author-overridable.

**Default UI (works with zero authoring):** a cleanly-styled trigger button ("Choose file…"), an inline preview thumbnail, and a status line — styled with the shell's WebAwesome design tokens (`--wa-color-*`, radius/spacing tokens) so the default is cohesive and theme-aware. Every piece is overridable.

**Attributes:**
- `with="{branch}@{repo}"` — branch context, forwarded by the view like `<tonk-display>`. The element builds ingest/read URLs from it, via a `{branch}@{repo}`→URL helper factored out of `blob_image_src` so both elements share it. Missing/unsubstituted `{…}` → the element disables the trigger and shows a status note (it can't know where to POST).
- `accept` (optional) — passed to the internal `<input type="file">`.

**Behavior (Shadow DOM):** on pick of a single file →
1. `await file.arrayBuffer()`.
2. Instant local `URL.createObjectURL(file)` preview (revoked after swap).
3. `fetch('/api/repository/{repo}/branch/{branch}/blob', { method:'POST', body: <ArrayBuffer>, headers:{ 'Content-Type': file.type || 'application/octet-stream', 'X-Tonk-Blob-Name': file.name } })` — relayed through the bridge for a sealed guest (binary bodies survive the `postMessage` structured clone; must use `init.body`, not a `Request` object).
4. On 2xx: parse `{ entity, contentType, name, size }`, swap the preview to the **read-route** URL `/api/repository/{repo}/branch/{branch}/blob/<entity>` (proving the bytes are stored and retrievable). Non-image `contentType` → show name + size instead of an `<img>`.
5. Emit the event (below).

State machine reflected on the host: `idle → reading → uploading → done | error`.

**Customization surface (author drives from the view template + CSS, no Rust):**
- **Slots:** `trigger` (the clickable that opens the picker — default: the styled button), `preview` (default: the element-managed `<img>`/thumb), `status` (default: the status text). Slot nothing → the default UI renders.
- **`::part()`:** `base` (container), `button`, `preview`, `status` — styleable from view CSS exactly like `wa-button::part(base)`.
- **CSS custom properties:** a small `--tonk-upload-*` set for the common knobs (accent color, radius, gap), each defaulting to a shell token.
- **State:** the host reflects `data-state="idle|reading|uploading|done|error"` for per-state CSS.

**Emit:** on success, a bubbling composed `CustomEvent('tonk-upload')` with `detail = { blob, contentType, name, size }` — the data hook a view wires into `/transact`. No event on failure.

### Data flow

```
pick file
  → read bytes (arrayBuffer)
  → instant local preview (object URL)
  → POST /…/blob  (relayed through the bridge for a sealed guest)
    → worker: Blob::import → assert content-type[+name] facts → broadcast
    → 200 { entity, contentType, name, size }
  → element: swap preview to read-route URL, set data-state=done, emit `tonk-upload` {detail}
  → (view author, LATER) binds `tonk-upload` → /transact to record the ref on some entity
```

## Error handling

- **Worker:** empty body → 400; failed ingest → 500 JSON error; malformed repo/branch/headers → 400, matching `serve`.
- **Element:** `arrayBuffer()`/read failure, fetch rejection, or non-2xx → `data-state=error`, the status slot shows the message, the local preview is cleared, and **no `tonk-upload` event is emitted** (a consumer never sees a phantom ref). Missing/unsubstituted `with` → disabled trigger + status note.

## Testing

- **Worker route** (`#[dialog_common::test]`, wasm service-worker, in `router/blob.rs`): POST bytes with a `Content-Type` and `X-Tonk-Blob-Name` → assert `200` + a `blob:` entity; then `GET …/blob/<entity>` through `serve` and assert the bytes round-trip and the `Content-Type` came back from the asserted fact. Re-POST the same bytes → same entity (idempotence). Empty-body POST → 400.
- **Element** (`#[dialog_common::test]`, browser, in `upload.rs`): mount `<tonk-upload with="main@repo">`; assert (1) the **default UI** renders in the shadow root (a trigger `::part(button)`, a `::part(preview)`, a `::part(status)`) and `data-state=idle`; (2) driving a synthetic `File` through the input with a stubbed relayed fetch emits `tonk-upload` once with the expected `detail`, swaps the preview `src` to the read-route URL, and sets `data-state=done`; (3) a failed fetch → `data-state=error`, no event; (4) an **author override** — mounting `<tonk-upload><button slot="trigger">Pick</button></tonk-upload>` — projects the author's trigger into the slot (default button not used) and still uploads.
- **Manual e2e:** a small demo view (in `demo.yaml`) mounting `<tonk-upload>` and capturing the event, exercised in `dev:web` (which now runs a local blob-aware access service).

## Out of scope (v1)

Drag-and-drop, multi-file selection, upload progress bars, streaming ingest, and upload-bound-to-an-entity/field. The last is the natural next layer: a view binds the emitted `tonk-upload` event to a `/transact` that writes `blob:<hash>` onto a target entity's attribute.

## File-level plan (informs the implementation plan)

- `rust/tonk-worker/src/router/blob.rs` — add the `POST` handler + its test; register the route in `router.rs`.
- `rust/tonk-display/src/upload.rs` — new headless element (shadow DOM, default UI, slots/parts/state) + tests; `register()` from the crate and the guest bundle.
- `rust/tonk-display/src/element.rs` (or a small shared module) — factor the `{branch}@{repo}`→URL logic out of `blob_image_src` so `<tonk-upload>` reuses it.
- `rust/tonk-core/assets/library/demo.yaml` — a demo view mounting `<tonk-upload>` (manual e2e only).
