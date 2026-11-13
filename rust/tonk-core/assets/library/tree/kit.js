// Small helpers every renderer here shares, made once per tag.
(self) => (self.constructor.__kit ??= {
  /** An element, with a class and text when given. */
  el(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = text;
    return element;
  },
  /** A node's dot: a filled disc when the replica holds the node, a
   *  hollow ring when it has not fetched it yet. */
  dot(cached, extra) {
    return this.el('span', `dot ${cached ? 'dot-local' : 'dot-remote'} ${extra ?? ''}`.trim());
  },
  /** Bytes as people read them. */
  human(n) {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  },
  /** The first `n` characters of a hash, without its `#`. */
  short(text, n) {
    return [...(text.startsWith('#') ? text.slice(1) : text)].slice(0, n).join('');
  },
  /** `text`, cut to `n` characters with an ellipsis when longer. */
  trunc(text, n) {
    const chars = [...text];
    return chars.length > n ? `${chars.slice(0, n).join('')}…` : text;
  },
  /** Keep `node`, unless one by its hash is kept already, and grow the
   *  widest size the bars scale to. */
  put(state, node) {
    state.maxSize = Math.max(state.maxSize, node.size);
    if (!state.nodes.has(node.hash)) state.nodes.set(node.hash, node);
  },
})
