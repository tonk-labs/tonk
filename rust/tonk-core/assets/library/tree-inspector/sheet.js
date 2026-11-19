// The element's stylesheet: the Bauhaus palette tonk's code surfaces use,
// named by Kandinsky's pairing of color and shape. Entities take the
// circle's blue, attributes the triangle's ochre, values the square's
// brick; machinery stays in the closure's warm grey. Each color is read
// through its `--tonk-*` variable with the literal as a fallback, since a
// guest may not carry the sheet that defines them.
(self) => `
:host {
  --circle: var(--tonk-circle, #3d6da8);
  --triangle: var(--tonk-triangle, #c89a2b);
  --square: var(--tonk-square, #b94a3d);
  --closure: var(--tonk-closure, #7a7268);
  --alarm: var(--tonk-alarm, #a8302a);
  /* Its own surface and ink, light or dark as the page is: a guest's
     scheme follows its embedder's, which declares both. */
  color-scheme: light dark;
  --surface: #fcfbfb;
  --ink: #38182a;
  --text: var(--ink);
  --quiet: color-mix(in oklab, currentColor 55%, transparent);
  --rule: color-mix(in oklab, currentColor 16%, transparent);
  --loud: color-mix(in oklab, currentColor 50%, transparent);
  --fill: color-mix(in oklab, currentColor 8%, transparent);
  /* The tree's connecting lines stay in the background. */
  --guide: color-mix(in oklab, currentColor 12%, transparent);
  display: block;
  box-sizing: border-box;
  height: 100dvh;
  background: var(--surface);
  color: var(--text);
  font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 13px;
}
@media (prefers-color-scheme: dark) {
  :host { --surface: #161515; --ink: #e6e1de; }
}
* { box-sizing: border-box; }
.shell {
  display: grid;
  grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
  height: 100%;
  min-height: 0;
}
@media (max-width: 48rem) {
  :host { height: auto; }
  .shell { grid-template-columns: minmax(0, 1fr); height: auto; }
  .detail { border-left: 0; border-top: 1px solid var(--rule); }
}
.outline, .detail { overflow: auto; min-height: 0; padding: 8px 12px; }

/* Column header: the same widths as the rows, so labels sit over values. */
.head {
  display: flex; align-items: center; gap: 8px;
  padding: 0 0 4px; margin-bottom: 2px;
  border-bottom: 1px solid var(--rule);
  color: var(--quiet); font-size: 0.85em;
}
.head .label { padding-left: 1.5em; }
.head .legend { flex: 1 1 auto; text-align: center; white-space: pre; opacity: 0.8; }

.row {
  position: relative;
  display: flex; align-items: center; gap: 8px;
  height: 30px; white-space: nowrap; cursor: pointer;
}
.row:hover { background: var(--fill); }
.row[aria-selected='true']::before {
  content: ''; position: absolute; left: -10px; top: 4px; bottom: 4px;
  width: 3px; background: var(--triangle);
}
.row:focus { outline: none; }
.row:focus-visible { background: color-mix(in oklab, var(--text) 10%, transparent); }
/* A node this replica does not hold reads faint until it is fetched. */
.row.remote .keystr, .row.remote .col { opacity: 0.45; }

/* D3-style indented-tree connectors. Each ancestor level is a gutter that
   carries the spine on while that ancestor has siblings below; the row's
   own gutter forks an elbow off the spine to the node's dot, and the last
   child stops the spine at its dot. */
.indent { display: flex; align-self: stretch; flex: none; }
.gutter { position: relative; width: 1.5em; flex: none; }
.gutter.on::before, .elbow::before, .elbow::after { content: ''; position: absolute; border-color: var(--guide); }
.gutter.on::before { left: 50%; top: 0; bottom: 0; border-left: 1px solid; }
.elbow::before { left: 50%; top: 0; bottom: 0; border-left: 1px solid; }
.elbow.last::before { bottom: 50%; }
.elbow::after { left: 50%; right: -4px; top: 50%; border-top: 1px solid; }

/* A node's dot: filled when this replica holds the node, hollow when it
   does not, dotted until dialog has said. A spinning arc takes its place
   while it is read. */
.dot {
  position: relative; z-index: 1; flex: none;
  width: 9px; height: 9px; border-radius: 50%;
  border: 1.5px solid var(--loud); background: transparent;
}
.dot.read { background: var(--loud); }
.dot.remote { background: transparent; }
.dot.unknown { border-style: dotted; opacity: 0.7; }
.dot.loading { border-color: transparent; }
.dot.loading::after {
  content: ''; position: absolute; inset: -1.5px; border-radius: 50%;
  border: 1.5px solid var(--quiet); border-top-color: transparent;
  animation: spin 0.7s linear infinite;
}
@keyframes spin { to { rotate: 360deg; } }

.keystr { flex: 1 1 auto; min-width: 0; overflow: hidden; display: flex; }
.name { font-weight: 600; }

/* A separator in the outline: its parts as coloured text, a thin gap
   between, the tail past the routing pivot dimmed. */
.key { display: inline-flex; align-items: baseline; gap: 0.5ch; min-width: 0; overflow: hidden; }
.key .part { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 1 auto; min-width: 2ch; }
.key .part--index { flex: none; font-weight: 600; }
.key .part--min { color: var(--text); }
.part--entity, .part--blob, .part--spill { color: var(--circle); }
.part--attribute { color: var(--triangle); }
.part--value { color: var(--square); }
.part--vtype { color: var(--square); opacity: 0.7; }
.part--index { color: var(--text); }
.part--origin, .part--edition, .part--opaque, .part--prefix, .part--pending, .part--padding { color: var(--closure); }
.key .part--padding { opacity: 0.6; }
.key .part.dim { opacity: 0.55; }

/* Value columns, right-packed so they line up at every depth. */
.col { flex: none; color: var(--quiet); text-align: right; font-variant-numeric: tabular-nums; }
.col.size { width: 128px; margin-left: 16px; display: inline-flex; align-items: center; justify-content: flex-end; gap: 8px; }
.col.count { width: 56px; }
.col.novelty { width: 56px; display: inline-flex; justify-content: flex-end; gap: 4px; }
.cloud { flex: none; width: 18px; color: var(--quiet); display: inline-flex; justify-content: center; }
.bar { display: inline-block; height: 7px; min-width: 2px; background: var(--closure); }
.num { width: 52px; }
.quiet { color: var(--quiet); }
.nov { font-size: 0.9em; }
.nov-own { color: var(--closure); }
.nov-pending { color: var(--triangle); }
.chip { font-size: 0.85em; padding: 0 4px; line-height: 1.5; background: var(--fill); }
.chip--pending { color: var(--triangle); }

.toolbar { display: flex; align-items: center; gap: 12px; padding: 0 0 6px; }
.overview { padding: 8px 12px; grid-column: 1 / -1; }
.overview h2 { margin: 12px 0 6px; font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: var(--circle); }
.overview h2:first-child { margin-top: 0; }
.line { display: flex; align-items: center; gap: 8px; min-height: 2em; border-bottom: 1px solid var(--rule); }
.line .entity { margin: 0; flex: 1 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.line .meta { display: flex; gap: 12px; color: var(--quiet); }
.chip--spill { color: var(--circle); }
.entity { display: block; color: var(--circle); text-decoration: underline; overflow-wrap: anywhere; }
a.button { text-decoration: none; display: inline-block; }
.note { margin: 0; padding: 4px 0; color: var(--quiet); font-style: italic; white-space: normal; }
.note--error { color: var(--alarm); }
.link { color: var(--circle); text-decoration: underline; }

/* The detail pane. */
/* The space's title sits over the page's top right corner, so the pane
   starts below it; the outline, on the left, needs no such room. */
.detail { border-left: 1px solid var(--rule); font-size: 14px; padding-top: 64px; }
.detail h2 { margin: 0 0 8px; font-size: 15px; font-weight: 600; }
.detail h3 { margin: 20px 0 8px; font-size: 15px; font-weight: 600; }
.facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); column-gap: 16px; row-gap: 6px; }
.kv { display: contents; }
.kv .k { display: flex; flex-direction: column; max-width: 22em; }
.kv .k .name { font-weight: 400; }
.kv .k .about { font-size: 11px; line-height: 1.35; color: var(--quiet); opacity: 0.8; }
.kv .v { display: flex; align-items: baseline; gap: 12px; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.detail > .entity { margin: 0 0 8px; }
.storage--local { color: var(--text); }
.storage--remote { color: var(--triangle); }
.storage--unknown { color: var(--quiet); }
details.header { margin-top: 20px; }
details.header > summary { cursor: pointer; font-size: 15px; font-weight: 600; margin-bottom: 8px; }
details.header:not([open]) > summary { margin-bottom: 0; }

/* A key in the pane: its parts as solid chips that wrap as text. */
.chips { margin: 4px 0 10px; line-height: 2; }
.chips .part {
  display: inline; white-space: normal; word-break: break-all;
  -webkit-box-decoration-break: clone; box-decoration-break: clone;
  padding: 2px 5px; margin-right: 3px;
  color: light-dark(#000, #fff); background: var(--closure);
}
.chips .part--entity, .chips .part--blob, .chips .part--spill { background: var(--circle); }
.chips .part--attribute { background: var(--triangle); color: #000; }
.chips .part--value { background: var(--square); }
.chips .part--vtype { background: var(--square); opacity: 0.75; }
.chips .part--index, .chips .part--min { background: light-dark(#000, #fff); color: light-dark(#fff, #000); }

/* The entries table. */
.table { margin-top: 4px; }
table.entries { width: 100%; border-collapse: collapse; table-layout: auto; font-size: 13px; }
table.entries th { text-align: left; font-weight: 400; color: var(--text); padding: 4px 8px 4px 0; border-bottom: 1px solid var(--rule); }
table.entries td { padding: 4px 8px 4px 0; border-bottom: 1px solid var(--rule); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 20em; }
table.entries tr.entry { cursor: pointer; }
table.entries tr.entry:hover { background: var(--fill); }
table.entries tr.removed td { color: var(--quiet); text-decoration: line-through; }
.col-ent { color: var(--circle); text-decoration: underline; }
.col-attr { color: var(--triangle); }
.col-span { color: var(--quiet); text-align: right; }
table.entries .col-index { width: 1%; }
table.entries td.col-val, table.entries td.col-val .val { color: var(--square); }
table.entries .val--entity { text-decoration: underline; }
.ordering { display: inline-block; font-weight: 600; color: var(--text); }
.version { margin-left: 6px; color: var(--closure); }
tr.unfolded td { white-space: normal; max-width: none; background: var(--fill); padding: 8px; }
.entry-detail { display: grid; grid-template-columns: max-content minmax(0, 1fr); column-gap: 16px; row-gap: 2px; }
.entry-detail > .k, .entry-detail > .chips, .entry-detail > pre, .entry-detail > .button { grid-column: 1 / -1; }
.entry-detail .kv .k { color: var(--quiet); }
.value {
  margin: 8px 0 0; padding: 8px; background: var(--fill);
  white-space: pre-wrap; overflow-wrap: anywhere; max-height: 16rem; overflow: auto;
}
.button {
  margin-top: 8px; font: inherit; color: var(--text); background: var(--fill);
  border: 1px solid var(--loud); border-radius: 0; padding: 2px 10px; cursor: pointer;
}
.button:hover { background: color-mix(in oklab, var(--text) 14%, transparent); }
`
