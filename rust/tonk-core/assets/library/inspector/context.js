// The branch this inspector's own `with` names, parsed the way the host
// parses it (`branch@repo`, a bare `repo` meaning its `main`, and
// `profile:<name>` the profile), or null when it names none: absent,
// malformed, or a template placeholder not yet stamped (`{branch}@{repo}`).
//
// `repo` keeps a profile's `profile:<name>` token, since the language
// server reads it back out of a cell's buffer URI to open the profile
// rather than a repository by that name.
(self) => {
  const value = (self.getAttribute('with') ?? '').trim();
  if (!value || value.includes('{')) return null;
  const at = value.indexOf('@');
  const branch = at === -1 ? 'main' : value.slice(0, at);
  const repo = at === -1 ? value : value.slice(at + 1);
  if (!branch || !repo || repo === 'profile' || repo === 'profile:') return null;
  return { with: value, repo, branch, profile: repo.startsWith('profile:') };
}
