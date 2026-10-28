(self) => {
  self.__observer?.disconnect();
  self.__observer = null;
  if (self.__frame) cancelAnimationFrame(self.__frame);
  self.__frame = 0;
}
