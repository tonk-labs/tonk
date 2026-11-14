// One `change` per idle gap rather than per edit. The flush pulls the
// workbook, so a burst costs one serialization.
(self) => {
  const state = self.__table;
  state.dirty = true;
  clearTimeout(state.timer);
  state.timer = setTimeout(() => self.flush(), 400);
}
