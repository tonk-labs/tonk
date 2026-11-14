// Mount the grid. With a `subject` the store is the document: claim rows
// in the light DOM feed it and each edit leaves as a typed event. Without
// one the initial workbook is, in order: a property write that came
// before the mount, the light-DOM text, then the `content` or `value`
// attribute, read through the envelope so a later echo of that same
// version is recognised and dropped.
async (self, token) => {
  const state = self.__table;
  const { core } = state;
  const subject = self.getAttribute('subject');
  let mode;
  if (subject !== null) {
    mode = { kind: 'claims', subject };
  } else {
    let raw = state.pending;
    if (raw === null) {
      const light = core.lightContent(self);
      raw = light !== '' ? light : (self.getAttribute('content') ?? self.getAttribute('value'));
    }
    state.pending = null;
    const parsed = core.parseContent(raw ?? '');
    if (parsed.hlc !== null && parsed.hlc > state.known) state.known = state.clock.receive(parsed.hlc);
    mode = { kind: 'standalone', source: core.toSource(parsed) };
  }
  let grid;
  try {
    grid = await core.createGrid(state.mount, {
      mode,
      emit: (type, detail) => self.emit(type, detail),
      readOnly: self.hasAttribute('readonly'),
      minRows: self.extent('min-rows'),
      minCols: self.extent('min-cols'),
      // Only standalone round-trips an envelope; claims mode emits events.
      onChange: mode.kind === 'standalone' ? () => self.schedule() : undefined,
    }, () => self.engine());
  } catch (error) {
    console.warn('[table-editor] failed to mount grid:', error);
    return;
  }
  // Went stale while the engine instantiated: never exposed, so drop it.
  if (token !== state.token || !self.isConnected) {
    grid.destroy();
    return;
  }
  state.grid = grid;
  if (mode.kind === 'claims') {
    // The rows the view has rendered already; later ones arrive through
    // the observer.
    self.apply();
  } else if (state.pending !== null) {
    // A write that arrived while the engine was instantiating, already
    // version-gated by `adopt`.
    const pending = core.parseContent(state.pending);
    state.pending = null;
    grid.load(core.toSource(pending));
  }
  self.emit('ready', { grid });
  if (!self.hasAttribute('readonly') && self.hasAttribute('auto-focus')) {
    // Deferred so sibling mounts settling in the same tick cannot take
    // focus back.
    setTimeout(() => { if (state.grid === grid) grid.focus(); }, 0);
  }
}
