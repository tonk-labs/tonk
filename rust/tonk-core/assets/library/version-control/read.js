(self) => {
  const data = self.querySelector(':scope > .vc-data');
  const rows = (selector) => (data ? [...data.querySelectorAll(selector)] : []);
  const isEntity = (value) => typeof value === 'string' && value.includes(':') && !value.includes('{');

  const revisions = new Map();
  for (const row of rows('.vc-revision-row')) {
    const id = row.dataset.revision;
    if (!isEntity(id) || revisions.has(id)) continue;
    revisions.set(id, {
      id,
      edition: Number(row.dataset.edition),
      issuer: row.dataset.issuer ?? '',
      authority: row.dataset.authority ?? '',
      parents: [],
      children: [],
      comments: [],
      heads: [],
    });
  }
  for (const row of rows('.vc-parent-row')) {
    const revision = revisions.get(row.dataset.revision);
    const parent = row.dataset.parent;
    if (revision && isEntity(parent) && !revision.parents.includes(parent)) {
      revision.parents.push(parent);
    }
  }
  // A merge's parents are a set; order them deepest first, so the
  // revision continues the lane of the history it grew out of.
  const depth = (id) => revisions.get(id)?.edition ?? -1;
  for (const revision of revisions.values()) {
    revision.parents.sort((a, b) => depth(b) - depth(a) || (a < b ? -1 : 1));
    for (const parent of revision.parents) revisions.get(parent)?.children.push(revision.id);
  }

  for (const row of rows('.vc-head-row')) {
    const revision = revisions.get(row.dataset.revision);
    const name = row.dataset.name;
    if (revision && name && !revision.heads.includes(name)) revision.heads.push(name);
  }

  const seen = new Set();
  for (const row of rows('.vc-comment-row')) {
    const id = row.dataset.comment;
    const revision = revisions.get(row.dataset.revision);
    if (!revision || !isEntity(id) || seen.has(id)) continue;
    seen.add(id);
    revision.comments.push({
      id,
      body: row.dataset.body ?? '',
      time: Number(row.dataset.time),
      author: row.dataset.author ?? '',
    });
  }
  for (const revision of revisions.values()) {
    revision.comments.sort((a, b) => a.time - b.time);
  }

  const members = new Map();
  for (const row of rows('.vc-member-row')) {
    if (row.dataset.member && row.dataset.name) members.set(row.dataset.member, row.dataset.name);
  }
  const profile = rows('.vc-session-row')[0]?.dataset.profile ?? '';

  return { revisions, members, profile };
}
