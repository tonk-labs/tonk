(self) => `
  :host { display: block; }
  .bar { display: flex; gap: .25rem; justify-content: flex-end; margin-bottom: .25rem; }
  .bar:empty { display: none; }
  .bar button { font: 12px system-ui, sans-serif; padding: .15rem .5rem; border: 1px solid #bbb; border-radius: 4px; background: transparent; color: inherit; cursor: pointer; }
  .bar button[aria-pressed="true"] { background: #2a6fdb; border-color: #2a6fdb; color: #fff; }
  .mount > prose-editor, .mount > code-editor { display: block; min-height: 8rem; }
  textarea { box-sizing: border-box; width: 100%; min-height: 14rem; resize: vertical; padding: .75rem; border: 1px solid #bbb; border-radius: 6px; font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; background: transparent; color: inherit; tab-size: 2; }
`
