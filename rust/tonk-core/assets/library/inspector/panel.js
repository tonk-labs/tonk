// The branch diagnostics above a space's cells, read-only: the
// repository's local metadata (`/api/repository/<repo>`) on mount and on
// `refresh`, and its remote's sync state (`.../sync/status`) only when
// asked, since that one goes over the network. A copy button beside each
// identifier puts it on the clipboard. Returns `{ host, refresh }`.
(self) => {
  const { repo, branch } = self.__inspector.context;
  const host = document.createElement('section');
  host.className = 'inspector-debug';
  host.setAttribute('aria-label', 'branch diagnostics');
  const panel = {
    host, repository: null, probe: { state: 'idle' }, refreshing: false, probing: false,
  };
  const replace = (html) => {
    const open = host.querySelector('.inspector-debug__disclosure')?.hasAttribute('open');
    host.innerHTML = html;
    if (open) host.querySelector('.inspector-debug__disclosure')?.setAttribute('open', '');
  };
  const busy = () => {
    const now = panel.refreshing || panel.probing;
    host.setAttribute('aria-busy', now ? 'true' : 'false');
    for (const button of host.querySelectorAll('[data-debug-action=refresh], [data-debug-action=probe]')) {
      button.toggleAttribute('disabled', now);
    }
  };
  const paint = () => {
    if (panel.repository === null) return;
    replace(self.panelHtml('repository', repo, branch, panel.repository, panel.probe));
    busy();
  };
  const read = async (path) => {
    const response = await fetch(path);
    const text = await response.text();
    if (response.ok) return text;
    throw new Error(text ? `GET ${path} returned HTTP ${response.status}: ${text}` : `GET ${path} returned HTTP ${response.status}`);
  };
  const feedback = (message) => {
    let line = host.querySelector('.inspector-debug__transient');
    if (!line) {
      line = document.createElement('div');
      line.className = 'inspector-debug__transient is-error';
      line.setAttribute('role', 'status');
      line.setAttribute('aria-live', 'polite');
      host.querySelector('.inspector-debug__body')?.append(line);
    }
    line.textContent = message;
  };
  panel.refresh = async () => {
    if (panel.refreshing || panel.probing) return;
    panel.refreshing = true;
    busy();
    try {
      panel.repository = await read(`/api/repository/${repo}`);
      panel.probe = { state: 'idle' };
      paint();
    } catch (error) {
      const message = error?.message ?? String(error);
      if (panel.repository !== null) feedback(`refresh failed: ${message}`);
      else replace(self.panelHtml('failure', repo, branch, message));
    }
    panel.refreshing = false;
    busy();
  };
  const probe = async () => {
    if (panel.probing || panel.refreshing || panel.repository === null) return;
    panel.probing = true;
    panel.probe = { state: 'loading' };
    paint();
    try {
      panel.probe = { state: 'response', body: await read(`/api/repository/${repo}/branch/${branch}/sync/status`) };
    } catch (error) {
      panel.probe = { state: 'failure', message: error?.message ?? String(error) };
    }
    panel.probing = false;
    paint();
  };
  const copy = async (button, value) => {
    try {
      await navigator.clipboard.writeText(value);
      button.textContent = 'copied';
      button.setAttribute('aria-label', 'copied');
      button.classList.add('is-copied');
    } catch (error) {
      button.textContent = 'retry';
      button.setAttribute('aria-label', 'copy failed; retry');
      button.setAttribute('title', error?.message ?? String(error));
      button.classList.add('is-error');
    }
  };
  host.addEventListener('click', (event) => {
    const button = event.target?.closest?.('button');
    if (!button || !host.contains(button)) return;
    const action = button.getAttribute('data-debug-action');
    if (action === 'refresh') panel.refresh();
    else if (action === 'probe') probe();
    else if (button.hasAttribute('data-copy-value')) copy(button, button.getAttribute('data-copy-value'));
  });
  host.innerHTML = self.panelHtml('loading', repo, branch);
  panel.refresh();
  return panel;
}
