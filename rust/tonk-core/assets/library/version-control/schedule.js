(self) => {
  if (self.__frame) return;
  self.__frame = requestAnimationFrame(() => {
    self.__frame = 0;
    if (self.isConnected) self.render();
  });
}
