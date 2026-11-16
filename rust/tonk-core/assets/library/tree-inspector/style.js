// The element's stylesheet. Colors come from the page's own text and
// background, so it follows the theme; a key's components take a hue
// per kind.
(self) => `
:host {
  display: block;
  color: inherit;
  font: inherit;
  --line: color-mix(in oklab, currentColor 14%, transparent);
  --faint: color-mix(in oklab, currentColor 6%, transparent);
  --muted: color-mix(in oklab, currentColor 62%, transparent);
  --mono: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
}
* { box-sizing: border-box; }
.shell {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(16rem, 24rem);
  gap: 1rem;
  align-items: start;
}
@media (max-width: 48rem) {
  .shell { grid-template-columns: minmax(0, 1fr); }
}
.outline {
  border: 1px solid var(--line);
  border-radius: 0.5rem;
  overflow: auto;
  max-height: 80vh;
  font-size: 0.8125rem;
}
.row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-height: 1.75rem;
  padding: 0.125rem 0.625rem 0.125rem calc(0.5rem + var(--depth, 0) * 1.125rem);
  border-bottom: 1px solid var(--faint);
  cursor: default;
  white-space: nowrap;
}
.row:last-child { border-bottom: 0; }
.row:hover { background: var(--faint); }
.row[aria-selected='true'] { background: color-mix(in oklab, currentColor 10%, transparent); }
.row:focus-visible { outline: 2px solid currentColor; outline-offset: -2px; }
.row[data-node], .row[data-more] { cursor: pointer; }
.twist { width: 1rem; flex: none; text-align: center; color: var(--muted); }
.at { width: 2.5rem; flex: none; text-align: right; color: var(--muted); font-family: var(--mono); font-variant-numeric: tabular-nums; }
.name { font-weight: 600; }
.meta { margin-left: auto; display: flex; gap: 0.75rem; align-items: center; color: var(--muted); font-variant-numeric: tabular-nums; padding-left: 1rem; }
.hash, code { font-family: var(--mono); font-size: 0.75rem; }
.pending { color: oklch(0.62 0.15 70); }
.badge {
  flex: none;
  font-size: 0.6875rem;
  line-height: 1;
  padding: 0.1875rem 0.375rem;
  border-radius: 999px;
  border: 1px solid var(--line);
  color: var(--muted);
}
.badge--index { color: oklch(0.6 0.14 255); border-color: currentColor; }
.badge--segment { color: oklch(0.6 0.13 155); border-color: currentColor; }
.badge--removed { color: oklch(0.6 0.19 25); border-color: currentColor; }
.badge--spill { color: oklch(0.62 0.15 70); border-color: currentColor; }
.key { display: inline-flex; gap: 0.25rem; min-width: 0; overflow: hidden; font-family: var(--mono); font-size: 0.75rem; }
.part {
  padding: 0.0625rem 0.3125rem;
  border-radius: 0.25rem;
  background: color-mix(in oklab, currentColor 12%, transparent);
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 22rem;
  flex: 0 1 auto;
}
.part--index, .part--min, .part--opaque, .part--pending, .part--prefix { color: var(--muted); }
.part--entity { color: oklch(0.6 0.14 255); }
.part--attribute { color: oklch(0.6 0.13 155); }
.part--vtype { color: oklch(0.6 0.12 300); }
.part--value { color: oklch(0.62 0.15 70); }
.part--spill, .part--blob { color: oklch(0.6 0.16 345); }
.part--origin, .part--edition { color: oklch(0.6 0.08 200); }
.note { margin: 0; padding: 0.5rem 0.75rem; color: var(--muted); white-space: normal; }
.note--error { color: oklch(0.6 0.19 25); }
.link { text-decoration: underline; }
.detail {
  border: 1px solid var(--line);
  border-radius: 0.5rem;
  padding: 0.875rem 1rem;
  font-size: 0.8125rem;
  position: sticky;
  top: 1rem;
  max-height: 80vh;
  overflow: auto;
}
.detail h2 { margin: 0 0 0.25rem; font-size: 1rem; }
.detail h3 { margin: 1rem 0 0.375rem; font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.04em; color: var(--muted); }
.detail .note { padding: 0; }
.entity { display: block; overflow-wrap: anywhere; color: var(--muted); }
.facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 0.25rem 0.75rem; margin: 0.75rem 0 0; }
.facts dt { color: var(--muted); }
.facts dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.wrap { overflow-wrap: anywhere; }
.parts { list-style: none; margin: 0; padding: 0; display: grid; gap: 0.25rem; }
.parts li { display: flex; gap: 0.5rem; align-items: baseline; min-width: 0; }
.parts .part { flex: none; font-family: var(--mono); font-size: 0.6875rem; }
.parts code { overflow-wrap: anywhere; min-width: 0; }
.value { margin: 0.5rem 0 0; padding: 0.5rem; border-radius: 0.375rem; background: var(--faint); font-family: var(--mono); font-size: 0.75rem; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 16rem; overflow: auto; }
.button { margin-top: 0.5rem; font: inherit; color: inherit; background: var(--faint); border: 1px solid var(--line); border-radius: 0.375rem; padding: 0.25rem 0.625rem; cursor: pointer; }
.button:hover { background: color-mix(in oklab, currentColor 12%, transparent); }
`
