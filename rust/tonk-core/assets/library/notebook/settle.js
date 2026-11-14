// Take the editor's text as the base, as the editor spells it.
//
// The editor normalizes what it is given (an empty fence gains a line,
// trailing spaces go), so its text can differ from the projection that
// produced it. Taking the projection as the base made every such block
// look edited, and held back every remote change as if mid-edit.
(self) => {
  const state = self.__notebook;
  const value = self.live();
  if (value === null) return;
  const parts = self.blocks().split(value);
  if (parts.length === state.blocks.length) {
    state.blocks.forEach((block, index) => { block.source = parts[index]; });
  }
  state.projected = value;
}
