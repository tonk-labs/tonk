(() => {
  if (globalThis.__welcomeDeferred) return;
  const state = globalThis.__welcomeDeferred = {
    ready: false,
    pending: null,
    prepare(root) {
      if (this.ready) return Promise.resolve();
      if (this.pending) return this.pending;
      const scope = root.closest('[with]')?.getAttribute('with') || '';
      const at = scope.indexOf('@');
      if (at < 1 || scope.includes('{')) return Promise.reject(Error('Space is not ready'));
      const url = '/api/repository/' + scope.slice(at + 1) +
        '/branch/' + scope.slice(0, at) + '/onboarding';
      this.pending = fetch(url, {method:'POST'}).then(response => {
        if (!response.ok) throw Error('Could not prepare the example pages');
        this.ready = true;
        root.querySelector('.welcome-prepare-error')?.remove();
      }).catch(error => {
        this.pending = null;
        if (root.isConnected && !root.querySelector('.welcome-prepare-error')) {
          const notice = document.createElement('button');
          notice.className = 'welcome-prepare-error';
          notice.textContent = 'Example pages could not load. Click to retry.';
          notice.addEventListener('click', () => {
            notice.remove();
            this.prepare(root).then(() => root.querySelector('vault-active')?.apply()).catch(() => {});
          });
          root.querySelector('.vault-main')?.prepend(notice);
        }
        throw error;
      });
      return this.pending;
    }
  };
  // Wait for actual Welcome content, then give it a frame before optional IO.
  const rendered = () => {
    const welcome = document.querySelector('.wp-outer');
    const root = welcome?.closest('.vault-root');
    if (!root || !welcome.checkVisibility()) return;
    observer.disconnect();
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (root.isConnected) state.prepare(root).catch(() => {});
    }));
  };
  const observer = new MutationObserver(rendered);
  observer.observe(document.body, {childList:true, subtree:true});
  rendered();

  customElements.define('welcome-image', class extends HTMLElement {
    static observedAttributes = ['with', 'entity'];
    connectedCallback() {
      this.observe();
      this.retry = () => this.observe();
      window.addEventListener('online', this.retry);
    }
    attributeChangedCallback() { if (this.isConnected) this.observe(); }
    disconnectedCallback() {
      this.observer?.disconnect();
      this.controller?.abort();
      window.removeEventListener('online', this.retry);
      if (this.objectUrl) URL.revokeObjectURL(this.objectUrl);
      this.objectUrl = null;
      this.loading = false;
      this.querySelector('img')?.removeAttribute('src');
    }
    observe() {
      this.observer?.disconnect();
      const img = this.querySelector('img');
      if (!img || this.objectUrl) return;
      this.observer = new IntersectionObserver(entries => {
        if (entries.some(e => e.isIntersecting)) this.load(img);
      });
      this.observer.observe(img);
    }
    async load(img) {
      if (this.loading || this.objectUrl) return;
      const scope = this.getAttribute('with') || '';
      const at = scope.indexOf('@');
      if (at < 1 || scope.includes('{')) return;
      this.loading = true;
      const controller = this.controller = new AbortController();
      try {
        const url = '/api/repository/' + scope.slice(at + 1) + '/branch/' +
          scope.slice(0, at) + '/blob/' + this.getAttribute('entity');
        const response = await fetch(url, {signal:controller.signal});
        if (!response.ok) throw Error('Image unavailable');
        const blob = await response.blob();
        if (!this.isConnected || controller.signal.aborted) return;
        this.objectUrl = URL.createObjectURL(blob);
        img.src = this.objectUrl;
        await img.decode();
        this.observer.disconnect();
      } catch (error) {
        if (error.name !== 'AbortError') console.warn('Welcome image:', error);
      } finally { this.loading = false; }
    }
  });
})();
