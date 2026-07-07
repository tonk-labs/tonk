# tonk-tree

`<tonk-tree>`: a web component that visualizes and inspects the
**dialog-search-tree** index that backs a branch, in the style of
`dialog-diagnose`. Like `tonk-code`, the element is implemented in
TypeScript (`src-js/`), bundled by `scripts/build.mjs` into
`assets/tonk-tree.js`, and shipped to the app by a thin Rust crate whose
`install()` injects the `<script type="module">` tag.

Visit `/space/{repo}/dialog:diagnose` in the app to explore a branch's
tree: index / segment nodes nested in a `<wa-tree>`, each row showing its
key as color-coded components (hover for labels) with shared prefixes
dimmed so you see where keys diverge, plus byte-size bars. The node
inspector shows the selected node's detail and, for a segment, its
entries (Entity · Attribute · Type · Value).

## How it wires itself

`<tonk-tree>` resolves the repository from its `with="branch@repo"`
routing ancestor (or an explicit `repo` / `branch` attribute), registers
Web Awesome's `<wa-tree>` (whose modules the WA auto-loader can't see
inside a shadow root), builds a `WorkerTreeLoader` against the worker's
`tree/*` query formulas, and renders a `<dialog-tree-outline>` +
`<dialog-node-inspector>` over a shared `Store` / `TreeState`. Those two
inner views are framework-agnostic: they bind to a `TreeLoader` and know
nothing about tonk.

See `plan/tree-inspector.md` for the full design.

## Build

```sh
npm install
npm run build        # -> assets/tonk-tree.js
npm run watch        # rebuild on change
```

The Rust crate (`tonk_tree::install`) is wired into
`tonk-ui/src/bin/ui.rs` and the bundle is copied into the served dist by
the `copy-dir` rule in `tonk-ui/index.html`.
