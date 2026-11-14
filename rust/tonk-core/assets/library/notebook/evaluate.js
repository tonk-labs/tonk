// Evaluate a notation document on this notebook's branch: a dry run with
// `transact` false, a commit with it true. Resolves to the worker's
// evaluate response; rejects with the worker's reason.
async (self, notation, transact) => {
  const context = self.__notebook?.context ?? self.context();
  if (!context) throw new Error('no repository in context');
  const dry = transact ? '' : '?transact=false';
  const response = await fetch(
    `/api/repository/${context.repo}/branch/${context.branch}/evaluate${dry}`,
    { method: 'POST', headers: { 'content-type': 'text/yaml', accept: 'application/json' }, body: notation },
  );
  const text = await response.text();
  if (!response.ok) {
    let message = text;
    try { message = JSON.parse(text).error?.message ?? text; } catch {}
    throw new Error(message || `HTTP ${response.status}`);
  }
  return text ? JSON.parse(text) : {};
}
