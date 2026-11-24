(self) => {
  const s = self.__edit;
  if (!s) return;
  s.observer?.disconnect();
  s.observer = null;
  self.flushEdit();
}
