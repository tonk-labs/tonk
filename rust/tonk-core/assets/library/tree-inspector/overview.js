// The page with no exploration selected: the tree the branch names now,
// and every exploration of this session, newest first.
(self, model, current, base, explore, short) => {
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const page = element('div', 'overview');
  const branchName = self.getAttribute('branch') || 'main';

  page.append(element('h2', '', `${branchName} now`));
  if (!current.length) page.append(element('p', 'note', 'This branch has no tree yet. The first write to the space makes one.'));
  for (const root of current) {
    const line = element('div', 'line');
    line.append(element('span', 'dot read'), element('span', 'entity', root));
    line.append(explore(root, model.explorations.has(root) ? 'open' : 'explore'));
    page.append(line);
  }

  page.append(element('h2', '', 'Explorations'));
  const explorations = [...model.explorations.values()].sort((a, b) => b.started - a.started);
  if (!explorations.length) page.append(element('p', 'note', 'None yet. They are kept for this session only.'));
  for (const exploration of explorations) {
    const line = element('div', 'line');
    const link = element('a', 'entity', exploration.root);
    link.href = `${base}/${exploration.root}`;
    line.append(element('span', 'dot read'), link);
    const meta = element('span', 'meta');
    meta.append(element('span', '', `${exploration.open.size} open`));
    if (exploration.started) meta.append(element('span', '', new Date(exploration.started).toLocaleTimeString()));
    if (current.includes(exploration.root)) meta.append(element('span', 'chip chip--spill', 'current'));
    line.append(meta);
    page.append(line);
  }
  return page;
}
