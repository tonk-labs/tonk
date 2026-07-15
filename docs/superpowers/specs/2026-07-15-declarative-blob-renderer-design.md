# Declarative blob renderer

2026-07-15. Follows PR #561 (image blob rendering) and PR #590 (blob upload
web component).

## Problem

`tonk-display` renders blobs through a hardcoded branch: when the host's
`model` attribute is `tonk:blob`, `handle_view_frame` returns early and
mounts an `<img>` (`rust/tonk-display/src/element.rs`). This has two limits:

1. Every blob renders as an image. A PDF or any other file type mounts a
   broken `<img>`.
2. The branch fires before view resolution, unconditionally. A user-asserted
   `view!:` for `model: tonk:blob` never mounts, so end users cannot change
   how blobs render.

The write side is already general: `<tonk-upload>`, `tonk blob add`, and
`POST …/blob` accept any content type and assert `xyz.tonk.blob/content-type`
(and usually `xyz.tonk.blob/name`) facts. Only rendering is image-shaped.

## Decision

Blob rendering becomes a **seeded declarative view + component** in
core.yaml, user-overridable like any other view. The native Rust image path
is demoted to a fallback that mounts only when no model-specific view
resolves (old branches, slide branches that seed only `command`). Nothing is
deleted.

Rich rendering stays image-only. Every other content type renders as a file
card with a lazy download action. Inline PDF/video/audio is out of scope.

## Design

### 1. Seeded view (core.yaml)

One `view!:` row with `model: tonk:blob` and a stable `this:` (stable
identity means a re-assert overwrites rather than duplicates — that is the
override mechanism). Its display carries the renderer JS inline and mounts
the element:

```html
<div subject={this}>
  <tonk-component><script type="tonk/module">/* defines tonk-blob-media */</script></tonk-component>
  <tonk-blob-media entity={this} content-type={content-type} name={name}></tonk-blob-media>
</div>
```

The module rides the view as an inert `<script type="tonk/module">` holder
(the `<tonk-component>` loader's child-holder shape), so view and behavior
travel in one row and a user overrides the whole renderer by re-asserting
one `view!:`. Component edits take effect on reload: `customElements.define`
cannot redefine a name; the loader already documents the
`customElements.get(name) ||` guard.

A separate `component!:` row was considered and rejected for now: it makes
the component independently hot-swappable but splits the override across two
rows and depends on the component-directory loader being mounted in every
context.

### 2. The `tonk-blob-media` component

Author JS, shadow DOM, dispatching on its `content-type` attribute:

- `content-type` starts with `image/` — relayed `window.fetch` of
  `/api/repository/{repo}/branch/{branch}/blob/{entity}` (repo/branch from
  `window.tonk.context`), wrap the bytes in `URL.createObjectURL`, mount
  `<img>`. Native resource loads in the sealed guest bypass the relay and
  the service worker, so the object-URL dance is mandatory, exactly as in
  the native Rust path.
- anything else — a file card: `name`, a content-type badge, a download
  action. No byte fetch at mount. On click: relayed fetch, object URL,
  programmatic `<a download={name}>` click.

Shadow DOM exposes `::part` hooks (`card`, `name`, `badge`, `action`,
`media`) following `<tonk-upload>`'s conventions. Stale object URLs are
revoked on re-render. Re-renders with unchanged attributes are a no-op.

### 3. Dispatch reshuffle (element.rs)

The unconditional `model == "tonk:blob"` early return in `handle_view_frame`
moves into the empty-frame path, following the existing `default_slide`
pattern: the native `<img>` mounts only when no model-specific view
resolved. On seeded branches the view frame arrives non-empty and normal
slide reconciliation mounts the declarative renderer, replacing any
previously mounted native fallback. On unseeded branches the native image
path keeps working as before.

### 4. Name fact totality (worker + CLI)

A concept query matches only rows with all fields present, so a blob missing
the `xyz.tonk.blob/name` fact has no complete `tonk:blob` row and the
declarative view renders nothing for it. Both write paths therefore always
assert `name`:

- `POST …/blob` defaults the name fact to the blob hash string when the
  `X-Tonk-Blob-Name` header is absent (today it skips the fact).
- `tonk blob add` likewise defaults when the path yields no file name
  (today: `path.file_name()` misses on paths like `..`).

`<tonk-upload>` already always sends the file name.

## Testing

- wasm tests in `element.rs`: a model-specific view frame wins over the
  native blob branch; the native `<img>` mounts on an empty frame (fallback
  preserved); existing image tests keep passing against the fallback path.
- Seed validity through `analyze_local`, like other core.yaml rows.
- Worker test: `POST …/blob` without the name header asserts the defaulted
  name fact.
- The component JS is data inside core.yaml and is not unit-testable from
  Rust; it is verified live in tonk-ui: upload a PDF through
  `<tonk-upload>` and see the card, confirm an image still renders inline,
  re-assert the view and see the override take effect after reload. The
  live pass also answers the one sandbox unknown: whether the sealed guest
  needs `allow-downloads` for the download click to save the file (if
  blocked, that is a one-attribute change in the shell).

## Out of scope

- Inline PDF/video/audio rendering.
- Streaming or Range support on the worker blob route (it buffers whole
  blobs; the card's lazy fetch keeps that acceptable).
- A size fact on blob entities.
- Reseeding existing repos (they keep the native image fallback).
- Deleting the native fallback (possible once seeding is universal).
