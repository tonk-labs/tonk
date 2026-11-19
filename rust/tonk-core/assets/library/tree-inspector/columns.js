// A node row's columns, right-packed so they line up at every depth: a
// bar and the block's size, its spans or entries, and what is buffered.
// `⊕n` is what the node buffers for its own children; `↓n` is what its
// parent still buffers against it, the one count a node not read yet
// can show. A cloud marks a node this replica does not hold.
(self, info, span, held) => {
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const cells = document.createDocumentFragment();
  const size = element('span', 'col size');
  if (info) {
    const bar = element('span', 'bar');
    bar.style.width = `${Math.max(2, Math.round(70 * info.size / Math.max(1, self.__tree.widest ?? 1)))}px`;
    size.append(bar, element('span', 'num', self.size(info.size)));
  } else {
    size.append(element('span', 'num quiet', '–'));
  }
  const count = element('span', 'col count', info ? String(info.count) : '');
  const novelty = element('span', 'col novelty');
  if (info?.novelty > 0) {
    const own = element('span', 'nov nov-own', `⊕${info.novelty}`);
    own.title = `${info.novelty} ops buffered on this node`;
    novelty.append(own);
  }
  if (span?.novelty > 0) {
    const pending = element('span', 'nov nov-pending', `↓${span.novelty}`);
    pending.title = `${span.novelty} ops buffered against this subtree in its parent`;
    novelty.append(pending);
  }
  cells.append(size, count, novelty);
  const cloud = element('span', 'cloud');
  cloud.setAttribute('aria-hidden', 'true');
  if (held === false) {
    cloud.title = 'not held by this replica';
    cloud.innerHTML = '<svg viewBox="0 0 16 16" width="14" height="14"><path fill="currentColor" d="M4.5 13a3.5 3.5 0 0 1-.42-6.98A4.5 4.5 0 0 1 12.8 6.6 3.2 3.2 0 0 1 12.5 13z"/></svg>';
  }
  cells.append(cloud);
  return cells;
}
