// The styles of the cells and their output, put into the editor's shadow
// root, which no page stylesheet reaches. Spare, so an output reads as an
// annotation on its cell; colours from the Web Awesome tokens, with
// fallbacks where they are absent.
(self) => `
/* A cell is part of the prose: no frame, a faint wash against the page's
   own foreground, so it works in either theme. Through <code-editor>'s own
   variables, so the editor is unchanged everywhere else. */
.md-code-block code-editor {
  --tonk-code-border: transparent;
  --tonk-code-radius: 4px;
  --tonk-code-bg: rgba(127, 127, 127, 0.06);
  --tonk-code-bg: color-mix(in srgb, currentColor 6%, transparent);
}
/* The cell you are in, a shade stronger. */
.md-code-block:focus-within code-editor {
  --tonk-code-bg: rgba(127, 127, 127, 0.11);
  --tonk-code-bg: color-mix(in srgb, currentColor 11%, transparent);
}
/* In a cell's output the "no view for this model" notice is noise: the
   default rendering IS what was asked for. Other callouts shrink to a bar. */
.notebook-cell-result [data-tonk-display-default-notice] {
  display: none;
}
.notebook-cell-result wa-callout {
  --wa-callout-padding: var(--wa-space-2xs, 0.25rem) var(--wa-space-xs, 0.5rem);
  font-size: var(--wa-font-size-xs, 0.75rem);
  line-height: 1.35;
  margin-block: var(--wa-space-2xs, 0.25rem);
}
.notebook-cell-result wa-callout::part(icon) {
  font-size: var(--wa-font-size-s, 0.875rem);
}
/* <tonk-notation>'s palette, on the element: the page declares it in the
   light DOM, which does not reach here. */
tonk-notation {
  --tonk-code-fg: var(--wa-color-text-normal);
  --tonk-code-key: var(--tonk-bauhaus-yellow, #c89a2b);
  --tonk-code-effect: var(--tonk-bauhaus-alarm, #d05a4a);
  --tonk-code-name-sigil: var(--wa-color-text-quiet);
  --tonk-code-name: var(--tonk-bauhaus-blue, #5a7fb8);
  --tonk-code-entity: var(--tonk-bauhaus-blue, #5a7fb8);
  --tonk-code-variable: var(--tonk-bauhaus-grey, #8a8a8a);
  --tonk-code-font: var(--wa-font-family-code, ui-monospace, monospace);
  --tonk-code-font-size: var(--wa-font-size-s, 0.875rem);
}
/* Long lines wrap, indentation kept: a result is read in place. */
tonk-notation .tonk-notation-pre {
  margin: 0;
  background: transparent;
  color: var(--tonk-code-fg);
  font-family: var(--tonk-code-font);
  font-size: var(--tonk-code-font-size);
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
tonk-notation .tonk-cm-key { color: var(--tonk-code-key); }
tonk-notation .tonk-cm-effect {
  color: var(--tonk-code-effect);
  font-weight: bold;
}
tonk-notation .tonk-cm-name-sigil { color: var(--tonk-code-name-sigil); }
tonk-notation .tonk-cm-name { color: var(--tonk-code-name); }
tonk-notation .tonk-cm-entity {
  color: var(--tonk-code-entity);
  text-decoration: underline;
  text-decoration-color: var(--tonk-code-entity);
  text-decoration-thickness: 1px;
  text-underline-offset: 2px;
}
tonk-notation .tonk-cm-variable {
  color: var(--tonk-code-variable);
  font-style: italic;
}
/* The bolt anchors to its cell. */
.md-code-block {
  margin: 0.5rem 0;
  position: relative;
}
/* It floats over the cell's bottom-right corner, reserving no room. */
.notebook-cell-held {
  position: static;
  height: 0;
}
.notebook-cell-held .evaluate-play {
  position: absolute;
  bottom: calc(-1 * var(--wa-space-m, 1rem));
  right: var(--wa-space-s, 0.75rem);
  z-index: 1;
}
.notebook-cell-result { display: block; margin: 0.25rem 0 0.75rem; }
.nb-out {
  font-size: 0.8125rem;
  line-height: 1.45;
  color: var(--wa-color-text-quiet, #9aa0a6);
}
.nb-out__summary {
  display: flex; align-items: baseline; gap: 0.5rem;
  padding: 0.125rem 0;
}
.nb-out__label {
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  color: var(--wa-color-text-normal, #e8eaed);
}
.nb-out__count { font-variant-numeric: tabular-nums; }
/* Cards as wide as what they hold, wrapping onto the next row. */
.nb-out__gallery {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 0.5rem;
  margin-top: 0.375rem;
}
.nb-card {
  flex: 0 1 auto;
  max-width: 100%;
  min-width: 0;
  box-sizing: border-box;
  border: 1px solid var(--wa-color-neutral-border-quiet, #3c4043);
  border-radius: var(--wa-border-radius-m, 0.375rem);
  padding: 0.375rem 0.5rem;
}
.nb-card__title {
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  color: var(--wa-color-text-normal, #e8eaed);
  overflow-wrap: anywhere;
  margin-bottom: 0.125rem;
}
/* A result drawn by its own view: a framed card headed by its entity. */
.nb-card--display {
  padding: 0;
  overflow: hidden;
}
.nb-card--display > .nb-card__title {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.5rem;
  margin: 0;
  padding: 0.3rem 0.625rem;
  background: rgba(127, 127, 127, 0.1);
  background: color-mix(in srgb, currentColor 8%, transparent);
  border-bottom: 1px solid var(--wa-color-neutral-border-quiet, #3c4043);
  font-family: var(--wa-font-family-body, inherit);
  font-size: 0.8125rem;
}
.nb-card--display > tonk-display {
  display: block;
  padding: 0.5rem 0.625rem;
}
.nb-card--display prose-editor {
  --tonk-prose-padding: 0;
}
.nb-card__name {
  font-weight: 600;
  color: var(--wa-color-text-normal, #e8eaed);
}
.nb-card__entity {
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  color: var(--tonk-bauhaus-blue, #5a7fb8);
  overflow-wrap: anywhere;
  user-select: all;
}
/* A result with no view of its own: the notation that asserts it. */
.nb-card--notation {
  padding: 0.375rem 0.625rem;
}
/* The folded rest of a long result: one line that opens it in place. */
.nb-out__rest > summary {
  cursor: pointer;
  list-style: none;
  margin-top: 0.375rem;
  width: fit-content;
}
.nb-out__rest > summary::-webkit-details-marker { display: none; }
.nb-out__rest > summary::before {
  content: "\\25B8\\00A0";
  display: inline-block;
}
.nb-out__rest[open] > summary::before { content: "\\25BE\\00A0"; }
.nb-out__more {
  margin-top: 0.25rem;
  font-style: italic;
}
.nb-out--error {
  display: flex; gap: 0.5rem; align-items: baseline;
  color: var(--wa-color-danger-on-quiet, #f28b82);
}
.nb-out__icon { font-weight: 700; }
`
