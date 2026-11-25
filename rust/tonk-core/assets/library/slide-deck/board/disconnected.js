(self) => {
  const s = self.__board;
  if (!s) return;
  s.observer?.disconnect();
  s.observer = null;
  if (s.onclick) self.removeEventListener('click', s.onclick);
  s.onclick = null;
  if (s.onfocus) self.removeEventListener('focusin', s.onfocus);
  s.onfocus = null;
}
