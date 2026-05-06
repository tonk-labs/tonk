# tonk-portals

`<tonk-portals>` — a React-backed grid-of-iframes panel UI
packaged as a custom element, plus a tiny Rust shim that injects
the bundle's loader script into the page.

This is a prototype. React/grid-select code is hosted inside a
custom element so the Leptos shell can own the page chrome
(banner, share, push/pull) while the panel grid reuses the
existing TS port verbatim — no Leptos rewrite of the grid
mechanics.

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | Rust crate. The `web` feature exposes [`install`] which appends `<script type="module">` for the bundle. |
| `src-js/index.tsx` | Custom element implementation. Mounts the React grid into a scoped container. |
| `src-js/grid/` | Grid, Square, SelectionRect — ported from `prototypes/grid-select`. |
| `src-js/lib/` | snap, pack, drag/resize hooks — ported. |
| `scripts/build.mjs` | esbuild driver. |
| `assets/` | Build output. **Committed** so consumers don't need a Node toolchain to build the Rust workspace. |

## Building

```bash
cd rust/tonk-portals
bun install
bun run build      # production build → assets/
bun run watch      # rebuild on source changes
```

Regenerate `assets/` whenever `src-js/` changes and commit alongside.

## Element API

```html
<tonk-portals repo="myspace" host="<host-id>"></tonk-portals>
```

Attributes:

| Name | Notes |
| --- | --- |
| `repo` | Repository name. Forms the `repo` segment of every artifact tile's data URL. |
| `host` | The hosting document's service-worker Client ID, echoed by the SW on every response (`X-Tonk-Client-Id`). Required for iframe URLs to carry a guest binding. |

Branch is *not* a top-level attribute. Portals are a UI layer
over a repo; individual artifact tiles inside the grid carry
their own `branch` when composing the data URL they load
(`/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`).
Defaulting tile branch to `main` keeps the common case ergonomic.
