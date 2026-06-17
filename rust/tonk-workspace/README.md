# tonk-workspace

Custom elements for the viewer workspace chrome.

This crate ships the web components that present the built-in `workspace` /
`artifact` / `view` concepts: the sheet/tab/canvas surface and the top-bar
controls around it. The concepts and their views live in the standard library
(`tonk-core/assets/library/core.yaml`, seeded by the service worker at repository
creation); this crate provides only the elements that render them. All elements
are light DOM (no shadow root) so the consuming workspace stylesheet can style
them, and they hold no app policy: most dispatch intent events or read/write
shared state and let the view or app shell decide what it means.

Call [`register`] once on the page to define every element. Registration is
idempotent. The DOM elements are wasm32-only; the pure sync-state and preference
logic is split out so it can be unit-tested natively.

## Sheet / tab / canvas chrome

- **`<tonk-sheet-binder active="…">`** models `<wa-tab-group>`. It accepts
  `<tonk-sheet>` children and projects the tab strip from them (one tab button per
  sheet, keyed and ordered by each sheet's `sheet` / `order` attributes), shows the
  panel named by `active`, and on tab click dispatches a bubbling `activate`
  `CustomEvent` carrying `detail.sheet`. It does not mutate state; the view wires
  the event to a command and the resulting `active` attribute flows back. A
  `MutationObserver` re-projects as sheets land asynchronously.
- **`<tonk-sheet>`** is one sheet, a `<wa-tab>` and `<wa-tab-panel>` rolled
  together. It projects its own card header (status dot, `title`, `subtitle`,
  optional `icon`) from its attributes; its element children are the card body
  shown in the canvas when the sheet is active.

## Top-bar and form controls

- **`<tonk-share>`** renders an icon button that, on click, resolves the repo from
  the nearest `<tonk-repository>` ancestor and dispatches a bubbling, composed
  `tonk:share` `CustomEvent` carrying `{ repo }` for the app shell to handle.
- **`<tonk-invite>`** mints an artifact-scoped invite link via
  `POST /api/repository/{repo}/invite` (with an artifact-targeted `base_url`,
  defaulting the link's concept to `tonk:artifact`) and renders it inline. Nothing
  is stored: the link's fragment is a private-key seed, so re-clicking mints a
  fresh one.
- **`<tonk-sync-state>`** is the background-sync status pill and pause/resume
  button in one. It resolves repo/branch from `<tonk-repository>` / `<tonk-branch>`
  ancestors, reads the `sync/status` route, and shows `synced`, `syncing`, or
  `paused`. Clicking flips the per-repo `tonk:auto-sync:{repo}` `localStorage`
  preference; for a branch with no upstream it instead reveals an "Enable sync"
  trigger that opens the `#enable-sync` dialog. It refreshes on the
  `tonk:status-refresh` and `tonk:committed` window events.
- **`<tonk-default-remote field="remote">`** renders a button (label from its own
  text, default "Use this server") that writes `location.origin + "/ucan/"` into
  the named form control within the closest `<form>`, supplying the origin that a
  static notation template cannot.
- **`<tonk-editable value="…">`** is an inline-editable single-line text control.
  It is `contenteditable`, shows `value` as its text, exposes a `.value` property,
  and dispatches a `change` event on commit (so a view drives it like a native
  input via `onchange`). Enter commits (blur), Escape cancels (restores the
  value captured on focus, then blurs without emitting `change`).
