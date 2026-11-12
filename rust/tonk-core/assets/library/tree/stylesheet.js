// The shadow root's stylesheet.
(self) => String.raw`
/* Two panes that scroll on their own, so the element needs a definite
   height: it fills its parent when that is a flex column, and the panes'
   min-height of 0 keeps their content from pushing past the bottom edge. */
:host {
  display: grid; grid-template-columns: 1.4fr 1fr;
  flex: 1 1 0; min-height: 0; height: 100%;
  box-sizing: border-box;
  color: var(--wa-color-text-normal);
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  font-size: var(--wa-font-size-s, 13px);
}
.pane { overflow: auto; min-height: 0; padding: var(--wa-space-m, 12px); }
.pane.left { border-right: 1px solid var(--wa-color-border-quiet); }
/* The outline runs smaller and denser, like a D3 indented tree; the
   connectors are sized in em, so they tighten with it. */
.outline { font-size: var(--wa-font-size-xs, 11px); }
/* Indented-tree connectors. The tree's own guide runs past the last child,
   so it is off, and each item draws a spine and an elbow to its dot. */
wa-tree { --indent-guide-color: transparent; }
wa-tree-item { position: relative;
  /* The selected row is marked by the accent bar alone, and focus by a
     quiet fill on its own row. */
  --wa-color-neutral-fill-quiet: transparent;
  --wa-focus-ring: 0 0 0 0 transparent; }
wa-tree-item:focus-visible, wa-tree-item:focus {
  background-image: linear-gradient(
    color-mix(in srgb, var(--wa-color-text-normal) 10%, transparent),
    color-mix(in srgb, var(--wa-color-text-normal) 10%, transparent));
  background-repeat: no-repeat; background-size: 100% 2em; background-position: 0 0; }
wa-tree-item::before, wa-tree-item::after {
  content: ''; position: absolute; z-index: 3;
  inset-inline-start: calc(0.1875em + var(--indent) - 1em);
  border-color: var(--wa-color-border-loud, #565861); }
/* The elbow stops at the dot's edge, so no line enters the circle. */
wa-tree-item::before { top: 1em; width: calc(2em - 5px);
  border-top-width: 1px; border-top-style: solid; }
/* The spine starts 1em above the row, at the parent's dot; the last child
   stops it at its own dot. */
wa-tree-item::after { top: -1em; bottom: 0; width: 0;
  border-inline-start-width: 1px; border-inline-start-style: solid; }
wa-tree-item:last-child::after { bottom: auto; height: 2em; }
wa-tree > wa-tree-item::before, wa-tree > wa-tree-item::after { display: none; }
/* Labels fill the row, so the value columns line up at every depth. */
wa-tree-item::part(label) { flex: 1 1 auto; min-width: 0; }
.row { display: inline-flex; align-items: center; gap: var(--wa-space-s, 8px); width: 100%; }
.row.remote { opacity: 0.5; }
/* The dot replaces the chevron: filled when held locally, hollow when not
   fetched. A branch's dot sits in the expand slot and is the toggle; a
   leaf's is placed on the item where the elbow ends. */
.dot { flex: none; width: 8px; height: 8px; border-radius: 50%; box-sizing: border-box;
  position: relative; z-index: 5; }
.dot-local { background: var(--wa-color-border-loud, #565861); border: 1.5px solid var(--wa-color-border-loud, #565861); }
.dot-remote { background: transparent; border: 1.5px solid var(--wa-color-border-loud, #565861); }
[slot="expand-icon"].dot, [slot="collapse-icon"].dot { rotate: none !important; }
/* While a node loads, one open arc spins in the dot's place. The tree shows
   both icon slots while loading, so only the expand dot spins. */
@keyframes tonk-dot-spin { to { rotate: 360deg; } }
.outline wa-tree-item[data-loading] [slot="collapse-icon"].dot { display: none; }
.outline wa-tree-item[data-loading] .dot { border-color: transparent; }
.outline wa-tree-item[data-loading] .dot::after {
  content: ''; position: absolute; top: 50%; left: 50%;
  width: 8px; height: 8px; margin: -4px 0 0 -4px; border-radius: 50%; box-sizing: border-box;
  border: 1.5px solid var(--wa-color-text-quiet, #94959b);
  border-top-color: transparent;
  animation: tonk-dot-spin 0.7s linear infinite; }
.outline wa-tree-item::part(spinner) { display: none; }
.outline wa-tree-item[data-loading]::before { opacity: 0; }
.outline wa-tree-item[data-loading]::after { bottom: auto; height: 1em; }
wa-tree-item > .dot-leaf { position: absolute; z-index: 5;
  inset-inline-start: calc(0.1875em + var(--indent) + 1em); top: 1em;
  transform: translate(-50%, -50%); }
.keystr { white-space: nowrap; display: inline-flex; align-items: center; gap: 3px;
  flex: 1 1 auto; min-width: 0; }
/* In the outline a key's parts are coloured text, divided by a thin dot. */
.key-seg { font-weight: var(--wa-font-weight-semibold, 600);
  display: inline-flex; align-items: center; }
.keystr .key-seg + .key-seg::before {
  content: "\2009\00B7\2009"; color: var(--wa-color-text-quiet, #94959b);
  font-weight: 400; }
.key-seg .seg-text { unicode-bidi: plaintext; }
.key-seg.seg-entity { color: var(--tonk-circle, #3d6da8); }
.key-seg.seg-attribute { color: var(--tonk-triangle, #c89a2b); }
.key-seg.seg-value { color: var(--tonk-square, #b94a3d); }
/* A value's type tag belongs to the value: its colour, quieter. */
.key-seg.seg-vtype { color: var(--tonk-square, #b94a3d); opacity: 0.7;
  font-weight: var(--wa-font-weight-normal, 400); }
.key-seg.seg-unknown { color: var(--tonk-closure, #7a7268); }
.key-seg.seg-index-type { color: var(--wa-color-text-normal); }
/* Past the routing pivot. */
.key-seg.dim { opacity: 0.6; font-weight: var(--wa-font-weight-normal, 400); }
/* Value columns: fixed widths at the pane's right edge, aligned across
   rows whatever the key or depth. */
.col { flex: none; color: var(--wa-color-text-quiet); white-space: nowrap;
  text-align: right; }
.col.size { width: 120px; margin-left: var(--wa-space-l, 24px);
  display: inline-flex; align-items: center; justify-content: flex-end;
  gap: var(--wa-space-xs, 6px); }
.col.count { width: 48px; }
.col.novelty { width: 84px; overflow: hidden; display: inline-flex; gap: 4px; justify-content: flex-end; }
.nov { font-size: 0.85em; padding: 0 4px; border-radius: 4px; line-height: 1.5;
  background: var(--wa-color-neutral-fill-quiet, #8882); }
.nov-own { color: var(--tonk-closure, #7a7268); }
.nov-pending { color: var(--tonk-triangle, #c89a2b); }
.col-header { display: flex; align-items: center; gap: var(--wa-space-s, 8px);
  padding: 0 0 4px; margin-bottom: 4px;
  color: var(--wa-color-text-quiet); border-bottom: 1px solid var(--wa-color-border-quiet);
  text-transform: uppercase; letter-spacing: 0.05em; font-size: 0.85em; opacity: 0.8; }
.col-header .keystr { flex: 1 1 auto; padding-left: 1.4em; }
.sizebar { height: 7px; background: var(--tonk-closure, #7a7268); border-radius: var(--wa-border-radius-s, 2px); min-width: 2px; }
.row.remote .sizebar { background: var(--wa-color-neutral-fill-loud, #666); }
.sizenum { width: 48px; text-align: right; }
.remote-icon { color: var(--wa-color-text-quiet, #94959b); flex: none; }
.status, .err { color: var(--wa-color-text-quiet); font-style: italic; padding: 4px 0; }
.err { color: var(--tonk-alarm, #a8302a); }

/* The detail pane. */
.inspector h2 { margin: 0 0 var(--wa-space-s, 8px); font-size: var(--wa-font-size-xs, 12px);
  font-weight: 600; color: var(--tonk-circle, #3d6da8); text-transform: uppercase; letter-spacing: 0.04em; }
.inspector .kv { display: flex; gap: var(--wa-space-s, 8px); margin: 2px 0; }
.inspector .kv .k { color: var(--wa-color-text-quiet); min-width: 64px; }
.inspector .kv .v { word-break: break-all; }
.inspector .sizebar { display: inline-block; vertical-align: middle; margin-left: 6px; }
/* A whole key as solid chips that wrap mid-chip, painted on both lines. */
.keybytes { margin: 4px 0 6px; line-height: 2; }
.keybytes .key-seg { display: inline; white-space: normal; word-break: break-all;
  -webkit-box-decoration-break: clone; box-decoration-break: clone;
  color: light-dark(#000, #fff); border-radius: var(--wa-border-radius-s, 2px);
  padding: 1px 4px; margin-right: 3px; }
.keybytes .key-seg.seg-entity { background: var(--tonk-circle, #3d6da8); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-attribute { background: var(--tonk-triangle, #c89a2b); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-value { background: var(--tonk-square, #b94a3d); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-vtype { background: var(--tonk-square, #b94a3d);
  color: light-dark(#000, #fff); opacity: 0.7; }
.keybytes .key-seg.seg-unknown { background: var(--tonk-closure, #7a7268); color: light-dark(#000, #fff); }
.keybytes .key-seg.seg-index-type { background: light-dark(#000, #fff);
  color: light-dark(#fff, #000); }
.keybytes .key-seg .seg-text { white-space: normal; word-break: break-all; }
.entries { margin-top: var(--wa-space-m, 12px); }
.entries .k { color: var(--wa-color-text-quiet); }
table { width: 100%; border-collapse: collapse; font-size: var(--wa-font-size-xs, 12px); }
th { text-align: left; color: var(--wa-color-text-quiet); font-weight: 600;
  padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-normal); }
td { padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-quiet); vertical-align: top; }
tr.entry { cursor: pointer; }
tr.entry:hover { background: var(--wa-color-surface-raised, rgba(255,255,255,0.04)); }
tr.entry.removed td { color: var(--wa-color-text-quiet); }
/* The index chip in its ordering's colour: the fact indexes take the key
   colours, the machinery (history, coverage, blob) stays neutral. */
.col-index { width: 1%; white-space: nowrap; }
.ordering { display: inline-block; padding: 0 5px; border-radius: 4px;
  font-size: 0.9em; letter-spacing: 0.02em;
  background: var(--wa-color-neutral-fill-quiet, #8882);
  color: var(--wa-color-text-quiet); }
.ordering-entity { color: var(--tonk-circle, #3d6da8); }
.ordering-attribute { color: var(--tonk-triangle, #c89a2b); }
.ordering-value { color: var(--tonk-square, #b94a3d); }
.version { margin-left: 4px; color: var(--wa-color-text-quiet); font-size: 0.9em;
  font-family: var(--wa-font-family-code, ui-monospace, monospace); }
.col-attr { color: var(--tonk-triangle, #c89a2b); }
.col-ent { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
.val-entity { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
/* Numbers, booleans and bytes stay neutral, never entity blue. */
.val-boolean, .val-unsignedint, .val-signedint, .val-float,
.val-bytes, .val-record { color: var(--tonk-closure, #7a7268); }
.val-symbol { color: var(--tonk-triangle, #c89a2b); }
tr.detail td { padding: 4px 0 8px 16px; }
.entry-detail .keybytes { margin: 4px 0; }
`
