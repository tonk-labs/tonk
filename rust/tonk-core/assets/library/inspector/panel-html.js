// The branch diagnostics as HTML, for `kind`:
//
//   'loading': reading the local metadata.
//   'failure': it could not be read, `(message)`.
//   'repository': it was, `(body, probe)`: the `/api/repository` JSON and
//     the remote probe, `{state: 'idle' | 'loading'}`, `{state:
//     'response', body}` or `{state: 'failure', message}`. A body that
//     does not decode renders as the failure.
//
// Collapsed by default: the summary names the branch, its head and
// whether it has a remote, and the rows open below it.
(self, kind, repo, branch, ...rest) => {
  const esc = (text) => String(text)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  const base58 = (bytes) => {
    const zeros = bytes.findIndex((byte) => byte !== 0);
    if (zeros === -1) return '1'.repeat(bytes.length);
    const digits = [];
    for (const byte of bytes.slice(zeros)) {
      let carry = byte;
      for (let i = 0; i < digits.length; i++) {
        carry += digits[i] * 256;
        digits[i] = carry % 58;
        carry = Math.floor(carry / 58);
      }
      while (carry > 0) {
        digits.push(carry % 58);
        carry = Math.floor(carry / 58);
      }
    }
    return '1'.repeat(zeros) + digits.reverse().map((digit) => ALPHABET[digit]).join('');
  };
  // A revision's tree as `#<base58>`, or null for none: the wire carries
  // the hash as bytes.
  const tree = (value) => {
    if (typeof value === 'string' && value) return value;
    if (Array.isArray(value) && value.length > 0 && value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)) {
      return `#${base58(value)}`;
    }
    return null;
  };
  const start = (value, status, refresh, probe) =>
    '<details class="inspector-debug__disclosure"><summary class="inspector-debug__summary">'
    + `<span>branch diagnostics</span><strong>${esc(branch)}</strong>`
    + `<code title="${esc(value)}">${esc(value)}</code><small>${esc(status)}</small></summary>`
    + '<div class="inspector-debug__body"><div class="inspector-debug__actions">'
    + (refresh ? '<button type="button" data-debug-action="refresh">refresh</button>' : '')
    + (probe ? '<button type="button" data-debug-action="probe">probe remote</button>' : '')
    + '</div>';
  const end = '</div></details>';
  const row = (label, value, copyable) =>
    `<div class="inspector-debug__row"><dt>${esc(label)}</dt><dd><code>${esc(value)}</code>`
    + (copyable
      ? `<button type="button" class="inspector-debug__copy" data-copy-value="${esc(value)}" aria-live="polite" aria-label="copy ${esc(label)}">copy</button>`
      : '')
    + '</dd></div>';
  const decode = (body, what) => {
    try {
      return JSON.parse(body);
    } catch (error) {
      throw new Error(`${what} decode: ${error.message}`);
    }
  };

  const failure = (message) => start(repo, 'metadata unavailable', true, false)
    + `<div class="inspector-debug__notice is-error" role="status">${esc(message)}</div>` + end;
  const repository = (body, probe) => {
    const info = decode(body, 'repository metadata');
    for (const name of ['name', 'label', 'subject', 'operator', 'profile']) {
      if (typeof info?.[name] !== 'string') throw new Error(`repository metadata decode: missing field \`${name}\``);
    }
    const state = info.branch?.[branch];
    const revision = tree(state?.revision?.tree) ?? 'no commits';
    const upstream = state?.upstream ?? null;
    const remote = upstream ? info.remote?.[upstream.remote] : null;

    let html = start(revision, upstream ? 'remote configured' : 'local only', true, !!upstream);
    html += '<dl class="inspector-debug__rows">';
    html += row('space', info.label, false);
    html += row('route', info.name, true);
    html += row('branch', branch, true);
    html += row('revision', revision, revision !== 'no commits');
    if (upstream) {
      html += row('upstream', `${upstream.remote}/${upstream.branch}`, true);
      if (remote) {
        const endpoint = remote.address?.Ucan?.endpoint;
        html += row('remote', typeof endpoint === 'string' ? endpoint : JSON.stringify(remote.address), true);
        html += row('remote subject', remote.subject ?? info.subject, true);
      } else {
        html += row('remote', 'configuration missing', false);
      }
    } else {
      html += row('upstream', 'none — local only', false);
      html += row('remote', 'not configured', false);
    }
    html += row('repository', info.subject, true);
    html += row('profile', info.profile, true);
    html += row('operator', info.operator, true);
    if (upstream) {
      const probed = probe ?? { state: 'idle' };
      if (probed.state === 'loading') html += row('sync', 'probing remote…', false);
      else if (probed.state === 'failure') {
        html += `<div class="inspector-debug__probe-error"><dt>sync</dt><dd>${esc(probed.message)}</dd></div>`;
      } else if (probed.state === 'response') {
        const status = decode(probed.body, 'sync status');
        const local = tree(status.local?.tree) ?? 'no commits';
        const head = tree(status.remote?.tree) ?? 'no commits';
        html += row('sync', status.state, false);
        html += row('local head', local, local !== 'no commits');
        html += row('remote head', head, head !== 'no commits');
      } else html += row('sync', 'not probed', false);
    }
    return html + '</dl>' + end;
  };

  if (kind === 'loading') {
    return start(repo, 'reading metadata…', false, false)
      + '<div class="inspector-debug__notice" role="status">reading local repository metadata…</div>' + end;
  }
  if (kind === 'failure') return failure(rest[0]);
  try {
    return repository(...rest);
  } catch (error) {
    return failure(error.message);
  }
}
