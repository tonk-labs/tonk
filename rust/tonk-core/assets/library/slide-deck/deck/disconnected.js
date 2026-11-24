(self) => {
  const s = self.__deck;
  if (!s) return;
  s.observer?.disconnect();
  s.observer = null;
  s.resize?.disconnect();
  s.resize = null;
  if (s.onkey) document.removeEventListener('keydown', s.onkey);
  s.onkey = null;
  clearTimeout(s.timer);
  s.source = null;
}
