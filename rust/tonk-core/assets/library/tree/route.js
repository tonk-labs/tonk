// The route reads go to, as the host takes it on a `tonk-query` detail,
// or null when this element names none. An explicit `repo` (and
// `branch`) names a space directly; otherwise its own `with` does, a
// space or the profile, and the host routes by it. A `with` still holding
// a template placeholder (`{branch}@{repo}`) is not a route yet.
(self) => {
  const repo = self.getAttribute('repo');
  if (repo) {
    const branch = self.getAttribute('branch');
    return { key: `${branch ?? ''}@${repo}`, detail: branch ? { space: repo, branch } : { space: repo } };
  }
  const own = (self.getAttribute('with') ?? '').trim();
  if (!own || own.includes('{')) return null;
  return { key: own, detail: {} };
}
