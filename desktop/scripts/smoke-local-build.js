// Run as a chrome-devtools evaluate_script function in an isolated browser.
// First run on / creates a disposable local fixture and returns its URL.
// Navigate there, verify the initial rendered checklist, then run again.
// No apply is retried, including when a response is lost.
async () => {
  if (location.hostname !== '127.0.0.1' || location.protocol !== 'http:')
    throw new Error('This destructive fixture test only runs on the isolated loopback runtime.');
  if (!navigator.serviceWorker.controller) throw new Error('Wait for the worker to control this page.');
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const request = async (url, body, method = 'POST', json = false) => {
    const response = await fetch(url, {
      method,
      headers: {'Content-Type': json ? 'application/json' : 'text/plain'},
      body: json ? JSON.stringify(body) : body,
      signal: AbortSignal.timeout(60000),
    });
    const text = await response.text();
    assert(response.ok, `HTTP ${response.status}: ${text.slice(0, 1000)}`);
    return JSON.parse(text);
  };
  const key = 'tonk-direct-build-smoke';
  if (location.pathname === '/') {
    const fixture = await fetch('/__test/direct-build.notation').then(r => {
      assert(r.ok, 'Missing local fixture');
      return r.text();
    });
    const info = await request('/api/repository/Local%20Build%20Smoke', {branch: {main: {}}}, 'PUT', true);
    const subject = info.subject;
    assert(typeof subject === 'string' && subject.startsWith('did:key:'), 'Missing new space identity');
    const endpoint = `/api/repository/${encodeURIComponent(subject)}/branch/main/evaluate`;
    // Initialization uses the ordinary evaluator to install schema and view.
    // The actual agent-style edit below must use the conditional endpoint.
    await request(endpoint, fixture);
    sessionStorage.setItem(key, JSON.stringify({subject, phase: 'prepared'}));
    return {phase: 'prepared', nextURL: `${location.origin}/space/${subject}`, expectedText: 'Waiting for direct apply'};
  }
  const state = JSON.parse(sessionStorage.getItem(key) || 'null');
  assert(state?.phase === 'prepared', 'Create a fresh local fixture first; never replay a previous apply.');
  assert(decodeURIComponent(location.pathname) === `/space/${state.subject}`, 'Wrong space selected');
  // Mark before sending: an exception must not allow accidental replay.
  sessionStorage.setItem(key, JSON.stringify({...state, phase: 'started'}));
  const endpoint = `/api/repository/${encodeURIComponent(state.subject)}/branch/main/evaluate`;
  const query = () => request(endpoint + '?transact=false', 'direct-build-task:\n');
  const before = await query();
  const rows = before.matches_after.flatMap(block => block.results);
  assert(rows.length === 1 && rows[0].fields.status === 'Waiting for direct apply', 'Unexpected fixture state');
  const document = 'direct-build-task!:\n  this: id:direct-build-task\n  status: Applied locally without CLI or sync\n';
  const preview = await request(endpoint + '?transact=false', document);
  assert(preview.commits.claims === 0 && same(preview.revision_after, before.revision_after), 'Preview changed state');
  const applied = await request(endpoint + '/conditional', {
    document, expected_revision: preview.revision_after,
  }, 'POST', true);
  assert(same(applied.revision_before, preview.revision_after), 'Apply used a different revision');
  assert(applied.commits.claims > 0 && !same(applied.revision_after, preview.revision_after), 'Apply did not commit');
  const stale = await fetch(endpoint + '/conditional', {
    method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({document: document.replace('Applied locally without CLI or sync', 'STALE WRITE MUST NOT APPEAR'), expected_revision: preview.revision_after}),
    signal: AbortSignal.timeout(60000),
  });
  assert(stale.status === 412, `Stale write returned ${stale.status}`);
  const after = await query();
  assert(same(after.revision_after, applied.revision_after), 'Stale request changed the revision');
  assert(after.matches_after.flatMap(block => block.results)[0].fields.status === 'Applied locally without CLI or sync', 'Readback mismatch');
  sessionStorage.setItem(key, JSON.stringify({...state, phase: 'applied'}));
  return {
    phase: 'applied', previewCommitted: false, staleStatus: stale.status,
    readbackVerified: true, renderingConfirmed: false,
    expectedRenderedText: 'Applied locally without CLI or sync',
    nextCheck: 'Take a fresh accessibility snapshot without navigating or reloading. Verify the expected text replaces Waiting for direct apply and no stale-write text appears.',
  };
}
