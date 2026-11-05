(self) => {
  const model = self.read();
  const { rows, width } = self.layout(model);
  const base = (self.getAttribute('base') || '/console/history').replace(/\/$/, '');
  const branch = self.getAttribute('branch') || 'main';
  const wanted = self.getAttribute('selected') ?? '';
  const head = rows.find((row) => row.revision.heads.includes(branch))?.revision;
  // With nothing in the path, keep showing what was shown: a comment is a
  // commit, so following the head would move the reader off the thread
  // they just wrote in.
  const selected = model.revisions.get(wanted)
    ?? model.revisions.get(self.__shown)
    ?? head
    ?? rows[0]?.revision
    ?? null;
  self.__shown = selected?.id ?? null;

  // A re-render must not lose what the reader was in the middle of.
  const active = self.ownerDocument.activeElement;
  const typing = active?.matches?.('.vc-detail textarea') && self.contains(active);
  const caret = typing ? [active.selectionStart, active.selectionEnd] : null;
  const refocus = self.__refocus
    ?? (active?.matches?.('.vc-entry a') && self.contains(active) ? active.dataset.revision : null);
  self.__refocus = null;

  const name = (did) => model.members.get(did) ?? (did ? `…${did.slice(-6)}` : 'unknown');
  const short = (id) => id.slice(-7);
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };

  const main = element('section', 'vc-main');
  const heading = element('header', 'vc-heading');
  heading.append(element('h1', '', 'History'));
  const count = rows.length;
  heading.append(element('p', '', `${count} revision${count === 1 ? '' : 's'} · ${branch}`));
  main.append(heading);

  if (!rows.length) {
    main.append(element('p', 'vc-empty', 'No revisions yet. The first write to this space makes one.'));
  } else {
    const log = element('ol', 'vc-log');
    log.setAttribute('aria-label', 'Revisions, newest first');
    for (const row of rows) {
      const revision = row.revision;
      const current = revision === selected;
      const item = element('li', 'vc-entry');
      item.setAttribute('aria-current', String(current));
      const link = element('a');
      link.href = `${base}/${revision.id}`;
      link.dataset.revision = revision.id;
      link.title = revision.id;
      link.append(self.rails(row, width, { head: revision.heads.length > 0, selected: current }));
      link.append(element('span', 'vc-id', `#${revision.edition} ${short(revision.id)}`));
      const who = name(revision.authority);
      link.append(element('span', 'vc-who', revision.authority === model.profile ? `${who} (you)` : who));
      for (const head of revision.heads) link.append(element('span', 'vc-badge vc-badge--head', head));
      if (revision.parents.length > 1) link.append(element('span', 'vc-badge vc-badge--merge', 'merge'));
      if (revision.comments.length) {
        const badge = element('span', 'vc-badge vc-badge--comments', `${revision.comments.length} 💬`);
        badge.title = `${revision.comments.length} comment${revision.comments.length === 1 ? '' : 's'}`;
        link.append(badge);
      }
      item.append(link);
      log.append(item);
    }
    main.append(log);
  }

  self.__shell.replaceChildren(main, self.detail(model, selected, { base, name, short }));

  if (typing && selected) {
    const box = self.__shell.querySelector('.vc-detail textarea');
    if (box) {
      box.focus();
      box.setSelectionRange(caret[0], caret[1]);
    }
  }
  if (refocus) {
    const link = [...self.__shell.querySelectorAll('.vc-entry a')].find((a) => a.dataset.revision === refocus);
    link?.focus();
    link?.scrollIntoView({ block: 'nearest' });
  }
}
