// One `change` per idle gap rather than per keystroke.
(self, value) => {
  const state = self.__prose;
  state.pending = value;
  clearTimeout(state.timer);
  state.timer = setTimeout(() => self.flush(), 400);
}
