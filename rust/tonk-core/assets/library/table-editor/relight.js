// The light-DOM text changed (the store wrote new content). Only a change
// to the text itself is adopted: a style-sheet edit must not reload it.
(self) => {
  const state = self.__table;
  const text = state.core.lightContent(self);
  if (text === state.light) return;
  state.light = text;
  self.adopt(text);
}
