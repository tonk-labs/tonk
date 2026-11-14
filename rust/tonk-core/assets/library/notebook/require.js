// Say so while the space lacks an editor this notebook writes in:
// <prose-editor> for the document, <code-editor> for its cells.
//
// A library component cannot install another, so the notebook cannot
// fetch its editors itself. It asks the space whether each name refers to
// anything (`id:<tag>`, which an install publishes), and the notice lists
// the ones that do not. Installing one defines its tag, which is what
// takes it off the notice: the editor waiting for <prose-editor> mounts
// then, and code blocks redraw as <code-editor> cells.
(self) => {
  const state = self.__notebook;
  const notice = document.createElement('div');
  notice.className = 'notebook-requires';
  notice.hidden = true;
  state.notice = notice;
  self.append(notice);
  const missing = new Set();
  const draw = () => {
    if (self.__notebook !== state) return;
    notice.hidden = missing.size === 0;
    const names = [...missing].map((tag) => `<code>${tag}</code>`).join(' and ');
    notice.innerHTML =
      '<wa-callout variant="warning"><wa-icon slot="icon" name="triangle-exclamation"></wa-icon>' +
      `The notebook needs ${names}, which this space does not have. Install ` +
      `${missing.size === 1 ? 'it' : 'them'} and the notebook becomes editable.</wa-callout>`;
  };
  const { repo, branch } = state.context;
  for (const tag of ['prose-editor', 'code-editor']) {
    if (customElements.get(tag)) continue;
    customElements.whenDefined(tag).then(() => {
      missing.delete(tag);
      draw();
    });
    const query = {
      predicate: { with: { entity: { the: 'db.name/referent', as: 'Entity', cardinality: 'one' } } },
      terms: { this: `id:${tag}`, entity: { '?': { name: 'entity' } } },
    };
    fetch(`/api/repository/${repo}/branch/${branch}/query`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: JSON.stringify(query),
    })
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error(`HTTP ${response.status}`))))
      .then((rows) => {
        if (Array.isArray(rows) && rows.length > 0) return;
        if (customElements.get(tag)) return;
        missing.add(tag);
        draw();
      })
      .catch((error) => console.warn(`<notebook-editor> could not ask for ${tag}:`, error));
  }
}
