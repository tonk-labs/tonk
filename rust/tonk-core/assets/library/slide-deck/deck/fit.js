(self) => {
  const s = self.__deck;
  if (!s) return;
  const k = Math.min((self.clientWidth || 1280) / 1280, (self.clientHeight || 720) / 720);
  s.stage.style.transform = `translate(-50%, -50%) scale(${k})`;
}
