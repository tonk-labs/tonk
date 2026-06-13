# dialog-arboretum

A framework-agnostic `<dialog-arboretum>` web component for visualizing
and inspecting a dialog index tree — the search/prolly tree that backs a
branch. Vanilla custom elements, zero runtime dependencies.

It knows nothing about tonk or any worker. Drive it by setting its
`.loader` property to a `TreeLoader`:

```ts
import "dialog-arboretum"; // registers <dialog-arboretum> + <dialog-tree-key>

const el = document.querySelector("dialog-arboretum");
el.loader = {
  root: () => /* TreeNode | null */,
  children: (hash) => /* TreeNode[] */,
  entries: (hash) => /* TreeEntry[] */,
  decodeKey: (key) => /* TreeKey */,
};
```

The component walks the tree lazily as the user expands rows, rendering
each node's kind, byte size (with a size bar), and child/entry count, and
decoding leaf entries' composite keys into colored component bars
(Tag · Entity · Attribute · ValueType · ValueRef).

In tonk it is embedded by a `tonk-display` view whose loader issues
`tree/*` formula queries against the worker (see
`plan/tree-inspector.md`). The `TreeLoader` shapes mirror those formula
results, but any source producing them works.

## Build

```sh
npm install
npm run build        # -> assets/dialog-arboretum.js
npm run watch        # rebuild on change
```

`demo.html` renders the component against a small mock loader — open it
over any static server (no worker needed) to see the visuals.
