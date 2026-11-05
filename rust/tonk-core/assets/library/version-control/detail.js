(self, model, revision, helpers) => {
  const { base, name, short } = helpers;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const pane = element('aside', 'vc-detail');
  pane.setAttribute('aria-label', 'Selected revision');
  if (!revision) {
    pane.append(element('p', 'vc-quiet', 'Select a revision to see who made it and what was said about it.'));
    return pane;
  }

  pane.append(element('h2', '', `Revision #${revision.edition}`));
  const facts = element('dl', 'vc-facts');
  const fact = (label, value, mono) => {
    facts.append(element('dt', '', label));
    const cell = element('dd', mono ? 'vc-mono' : '', value);
    facts.append(cell);
    return cell;
  };
  fact('Revision', revision.id, true);
  fact('Edition', String(revision.edition));
  const author = fact('Author', name(revision.authority));
  author.title = revision.authority;
  fact('Device', `…${revision.issuer.slice(-6)}`, true).title = revision.issuer;
  if (revision.heads.length) fact('Head of', revision.heads.join(', '));
  pane.append(facts);

  const links = (title, ids, none) => {
    pane.append(element('h3', '', title));
    if (!ids.length) {
      pane.append(element('p', 'vc-quiet', none));
      return;
    }
    const list = element('ul', 'vc-links');
    for (const id of ids) {
      const other = model.revisions.get(id);
      const item = element('li');
      const link = element('a', '', other ? `#${other.edition} ${short(id)}` : short(id));
      link.href = `${base}/${id}`;
      link.title = other ? id : `${id} (not replicated here)`;
      item.append(link);
      list.append(item);
    }
    pane.append(list);
  };
  links('Built on', revision.parents, 'Nothing: this is the first revision.');
  links('Built upon by', revision.children, 'Nothing yet: this is a head.');

  pane.append(element('h3', '', `Comments (${revision.comments.length})`));
  if (revision.comments.length) {
    const thread = element('ol', 'vc-thread');
    const relative = (time) => {
      if (!Number.isFinite(time)) return '';
      const seconds = Math.round((Date.now() - time) / 1000);
      if (seconds < 60) return 'just now';
      if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
      if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
      return new Date(time).toLocaleDateString();
    };
    for (const comment of revision.comments) {
      const item = element('li', 'vc-comment');
      const header = element('header');
      const who = element('strong', '', comment.author === model.profile ? 'You' : name(comment.author));
      who.title = comment.author;
      const when = element('time', '', relative(comment.time));
      if (Number.isFinite(comment.time)) {
        when.dateTime = new Date(comment.time).toISOString();
        when.title = new Date(comment.time).toLocaleString();
      }
      header.append(who, when);
      if (comment.author && comment.author === model.profile) {
        const withdraw = element('button', 'vc-withdraw', 'Withdraw');
        withdraw.type = 'button';
        withdraw.addEventListener('click', () => {
          withdraw.disabled = true;
          self.dispatchEvent(new CustomEvent('vc-uncomment', {
            bubbles: true,
            detail: { comment: comment.id },
          }));
        });
        header.append(withdraw);
      }
      item.append(header, element('p', '', comment.body));
      thread.append(item);
    }
    pane.append(thread);
  } else {
    pane.append(element('p', 'vc-quiet', 'No comments on this revision yet.'));
  }

  const form = element('form');
  const box = element('textarea');
  box.name = 'body';
  box.placeholder = 'Leave a comment on this revision';
  box.setAttribute('aria-label', 'Comment');
  box.value = self.__drafts.get(revision.id) ?? '';
  const send = element('button', '', 'Comment');
  send.type = 'submit';
  send.disabled = !box.value.trim();
  box.addEventListener('input', () => {
    self.__drafts.set(revision.id, box.value);
    send.disabled = !box.value.trim();
  });
  box.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      form.requestSubmit();
    }
  });
  form.addEventListener('submit', (event) => {
    event.preventDefault();
    const body = box.value.trim();
    if (!body) return;
    self.dispatchEvent(new CustomEvent('vc-comment', {
      bubbles: true,
      detail: { revision: revision.id, body, time: Date.now() },
    }));
    self.__drafts.delete(revision.id);
    box.value = '';
    send.disabled = true;
  });
  form.append(box, send);
  pane.append(form);
  return pane;
}
