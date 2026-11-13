(self, name, before, after) => {
  const state = self.__table;
  // The runtime replays attributes before `connected`, which reads them
  // when it mounts.
  if (!state) return;
  if (name === 'subject') {
    // The document changed modes or targets: remount.
    if (before !== after && state.grid) {
      state.grid.destroy();
      state.grid = null;
      const token = ++state.token;
      if (self.isConnected) self.mount(token);
    }
  } else if (name === 'content') self.adopt(after ?? '');
  // `value` is the bare CSV channel; reflected only when it differs, so it
  // does not fight the user's edits.
  else if (name === 'value' && (after ?? '') !== self.value) self.adopt(after ?? '');
  else if (name === 'readonly') state.grid?.setReadOnly(after !== null);
  else if (name === 'min-rows' || name === 'min-cols') {
    state.grid?.setMinExtent(self.extent('min-rows'), self.extent('min-cols'));
  }
  // `auto-focus` applies at mount only: stealing focus later would
  // surprise the user.
}
