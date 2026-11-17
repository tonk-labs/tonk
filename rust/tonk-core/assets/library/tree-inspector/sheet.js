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
  --text: currentColor;
  --quiet: color-mix(in oklab, currentColor 60%, transparent);
  --rule: color-mix(in oklab, currentColor 18%, transparent);
  --loud: color-mix(in oklab, currentColor 55%, transparent);
  --fill: color-mix(in oklab, currentColor 9%, transparent);
  display: block;
  color: var(--text);
  font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 13px;
}
* { box-sizing: border-box; }
.shell {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  min-height: 0;
  height: min(80vh, 100%);
  border-top: 1px solid var(--rule);
  border-bottom: 1px solid var(--rule);
}
@media (max-width: 48rem) {
  .shell { grid-template-columns: minmax(0, 1fr); height: auto; }
  .detail { border-left: 0; border-top: 1px solid var(--rule); }
}
.outline, .detail { overflow: auto; min-height: 0; padding: 12px; }
.outline { font-size: 11px; }

/* Column header: the same widths as the rows, so labels sit over values. */
.head {
  display: flex; align-items: center; gap: 8px;
  padding: 0 0 4px; margin-bottom: 4px;
  border-bottom: 1px solid var(--rule);
  color: var(--quiet); text-transform: uppercase; letter-spacing: 0.05em;
  font-size: 0.85em;
}
.head .label { flex: 1 1 auto; padding-left: 1.5em; }

.row {
  display: flex; align-items: center; gap: 8px;
  height: 2em; white-space: nowrap; cursor: default;
}
.row:hover { background: var(--fill); }
.row[aria-selected='true'] { box-shadow: inset 2px 0 0 var(--circle); background: var(--fill); }
.row:focus { outline: none; }
.row:focus-visible { background: color-mix(in oklab, var(--text) 10%, transparent); }
.row[data-node], .row[data-more] { cursor: pointer; }

/* D3-style indented-tree connectors. Each ancestor level is a gutter that
   carries the spine on while that ancestor has siblings below; the row's
   own gutter forks an elbow off the spine to the node's dot, and the last
   child stops the spine at its dot. */
.indent { display: flex; align-self: stretch; flex: none; }
.gutter { position: relative; width: 1.5em; flex: none; }
.gutter.on::before, .elbow::before, .elbow::after { content: ''; position: absolute; border-color: var(--loud); }
.gutter.on::before { left: 50%; top: 0; bottom: 0; border-left: 1px solid; }
.elbow::before { left: 50%; top: 0; bottom: 0; border-left: 1px solid; }
.elbow.last::before { bottom: 50%; }
.elbow::after { left: 50%; right: -4px; top: 50%; border-top: 1px solid; }

/* A node's dot: filled once it is read, hollow until then. A spinning arc
   takes its place while it is read. */
.dot {
  position: relative; z-index: 1; flex: none;
  width: 8px; height: 8px; border-radius: 50%;
  border: 1.5px solid var(--loud); background: transparent;
}
.dot.read { background: var(--loud); }
.dot.leaf { width: 5px; height: 5px; border-width: 1px; background: var(--loud); }
.dot.loading { border-color: transparent; }
.dot.loading::after {
  content: ''; position: absolute; inset: -1.5px; border-radius: 50%;
  border: 1.5px solid var(--quiet); border-top-color: transparent;
  animation: spin 0.7s linear infinite;
}
@keyframes spin { to { rotate: 360deg; } }

.name { font-weight: 600; }
.at { color: var(--quiet); min-width: 2.5em; text-align: right; font-variant-numeric: tabular-nums; }

/* A key in the outline: its parts as colored text, a thin dot between. */
.key { display: inline-flex; align-items: center; min-width: 0; flex: 1 1 auto; overflow: hidden; }
.part {
  font-weight: 600;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  flex: 0 1 auto; min-width: 3ch; max-width: 32em;
}
/* Long parts give way first: an entity before a value before an
   attribute. The ordering and the value's type never shrink. */
.key .part--entity { flex-shrink: 4; }
.key .part--value { flex-shrink: 2; }
.key .part--index, .key .part--vtype { flex: none; }
.key .part + .part::before { content: '\\2009\\00B7\\2009'; color: var(--quiet); font-weight: 400; }
.part--entity, .part--blob, .part--spill { color: var(--circle); }
.part--attribute { color: var(--triangle); }
.part--value { color: var(--square); }
.part--vtype { color: var(--square); opacity: 0.7; font-weight: 400; }
.part--index { color: var(--text); }
.part--origin, .part--edition, .part--opaque, .part--prefix, .part--min, .part--pending { color: var(--closure); font-weight: 400; }

/* Value columns, right-packed so they line up at every depth. */
.col { flex: none; color: var(--quiet); text-align: right; font-variant-numeric: tabular-nums; }
.col.size { width: 120px; margin-left: 24px; display: inline-flex; align-items: center; justify-content: flex-end; gap: 6px; }
.col.count { width: 48px; }
.col.buffered { width: 84px; overflow: hidden; display: inline-flex; justify-content: flex-end; }
.bar { height: 7px; min-width: 2px; background: var(--closure); }
.num { width: 48px; }
.chip { font-size: 0.85em; padding: 0 4px; line-height: 1.5; background: var(--fill); }
.chip--pending { color: var(--triangle); }
.chip--removed { color: var(--alarm); }
.chip--spill { color: var(--circle); }

.bar { display: flex; align-items: center; gap: 12px; padding: 0 0 8px; }
.overview { padding: 12px; grid-column: 1 / -1; }
.overview h2 { margin: 16px 0 8px; font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: var(--circle); }
.overview h2:first-child { margin-top: 0; }
.line { display: flex; align-items: center; gap: 8px; min-height: 2em; border-bottom: 1px solid var(--rule); }
.line .entity { margin: 0; flex: 1 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.line .meta { display: flex; gap: 12px; color: var(--quiet); }
a.button { text-decoration: none; display: inline-block; margin-top: 0; }
.note { margin: 0; padding: 4px 0; color: var(--quiet); font-style: italic; white-space: normal; }
.note--error { color: var(--alarm); }
.link { color: var(--circle); text-decoration: underline; }

/* The detail pane. */
.detail { border-left: 1px solid var(--rule); }
.detail h2, .detail h3 {
  margin: 0 0 8px; font-size: 12px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
}
.detail h2 { color: var(--circle); }
.detail h3 { margin-top: 16px; color: var(--quiet); }
.entity { display: block; margin-bottom: 8px; color: var(--circle); text-decoration: underline; overflow-wrap: anywhere; }
.facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 2px 8px; margin: 0; }
.facts dt { color: var(--quiet); min-width: 64px; }
.facts dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
/* A key in the pane: its parts as solid chips that wrap as text. */
.chips { margin: 4px 0 6px; line-height: 2; }
.chips .part {
  display: inline; white-space: normal; word-break: break-all; max-width: none;
  -webkit-box-decoration-break: clone; box-decoration-break: clone;
  padding: 1px 4px; margin-right: 3px; font-weight: 600;
  color: light-dark(#000, #fff); background: var(--closure);
}
.chips .part::before { content: none; }
.chips .part--entity, .chips .part--blob, .chips .part--spill { background: var(--circle); }
.chips .part--attribute { background: var(--triangle); }
.chips .part--value { background: var(--square); }
.chips .part--vtype { background: var(--square); opacity: 0.7; }
.chips .part--index { background: light-dark(#000, #fff); color: light-dark(#fff, #000); opacity: 1; }
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
