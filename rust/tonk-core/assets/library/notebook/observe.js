// Watch the rows and the editor, and bring the one in line with the other
// whenever either changes.
//
// The rows are hidden `.notebook-block-row` and `.notebook-order__item`
// nodes, rendered by nested <tonk-display>s on their own schedule and
// carrying their data in `data-*` attributes, so a store update is often
// an attribute change. The editor renders into a shadow root, which an
// observer on the host does not reach; it exists only once the editor
// mounts, so it is watched from the first mutation that finds it.
//
// Never the host's subtree: the editor lives under it, and the
// projection's own write into the editor would re-fire it forever.
//
// The refresh runs on a fresh task, never inside the callback: the
// callback runs inside the DOM write that triggered it, which can be
// ProseMirror's own transaction, and replacing the document under it
// breaks the selection.
(self) => {
  const state = self.__notebook;
  const RETRY_MS = 120;
  const RETRIES = 25;
  const full = { childList: true, subtree: true, attributes: true, characterData: true };
  const watch = () => {
    const root = state.prose.shadowRoot;
    if (root) state.observer.observe(root, { childList: true, subtree: true });
    const pane = self.querySelector('.notebook-data');
    if (pane) state.observer.observe(pane, full);
  };
  state.observer = new MutationObserver(() => {
    if (state.settling) return;
    self.later(() => self.refresh(), 0);
    watch();
  });
  state.observer.observe(state.prose, full);
  // The pane is a static child of the view, so normally here already; the
  // host's direct children are watched for when it is not.
  state.observer.observe(self, { childList: true });
  watch();
  self.refresh();
  // Rows that landed before the observer watched are only caught by
  // looking again, a few times, until one look projects.
  let remaining = RETRIES;
  const retry = () => {
    if (self.__notebook !== state || state.projectedOnce || remaining-- <= 0) return;
    self.refresh();
    self.later(retry, RETRY_MS);
  };
  self.later(retry, RETRY_MS);
}
