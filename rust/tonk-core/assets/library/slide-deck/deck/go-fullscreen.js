(self) => {
  if (document.fullscreenElement) document.exitFullscreen?.();
  else self.requestFullscreen?.().catch((error) => console.warn('slide-deck: fullscreen refused', error));
}
