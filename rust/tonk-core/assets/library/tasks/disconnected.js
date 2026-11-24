(self) => {
  const reader = self.__reader;
  self.__reader = null;
  if (reader) reader.cancel().catch(() => {});
}
