// The index's styles, put into the page once: the directory's rows and the
// switcher's panel are in the light DOM.
(self) => `
notebook-index {
  display: block;
  max-width: 48rem;
  margin: 0 auto;
  padding: var(--wa-space-l, 1.5rem);
}
/* The rows feed the suggestions and are the no-script fallback, so they
   are hidden once the draft notebook is there. */
notebook-index:has(notebook-editor[draft]) .notebook-index,
notebook-index:has(notebook-editor[draft]) .notebook-index__item {
  display: none;
}
.notebook-index__item {
  display: block;
  padding: var(--wa-space-s, 0.75rem) var(--wa-space-m, 1rem);
  border-radius: var(--wa-border-radius-m, 0.375rem);
  color: var(--wa-color-text-normal);
  text-decoration: none;
}
.notebook-index__item:hover {
  background: var(--wa-color-neutral-fill-quiet);
}
.notebook-index__item + .notebook-index__item {
  border-top: 1px solid var(--wa-color-surface-border);
}
.notebook-index__title { font-weight: 500; }
.notebook-index .notebook-block-rows { display: none; }
.notebook-switcher__panel {
  margin: 0 0 var(--wa-space-l, 1.5rem);
  border: 1px solid var(--wa-color-surface-border);
  border-radius: var(--wa-border-radius-m, 0.375rem);
  overflow: hidden;
}
.notebook-switcher__panel[hidden] { display: none; }
.notebook-switcher__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--wa-space-m, 1rem);
  padding: var(--wa-space-s, 0.75rem) var(--wa-space-m, 1rem);
  cursor: pointer;
}
.notebook-switcher__name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* What Enter does, quiet: the name is what you read. */
.notebook-switcher__verb {
  flex: 0 0 auto;
  color: var(--wa-color-text-quiet);
  font-size: var(--wa-font-size-xs, 0.75rem);
}
.notebook-switcher__row--active {
  background: var(--wa-color-neutral-fill-quiet);
}
/* The matched characters, so the reason a row is here is visible. */
.notebook-switcher__row mark {
  background: none;
  color: var(--wa-color-brand-fill-loud, currentColor);
  font-weight: 600;
}
`
