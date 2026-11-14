// Take the announced document off its language server: the inverse of
// `announce`, before a new `source`, language or endpoint is announced,
// and when the editor goes away. A `code-editor-disconnect` tells
// whoever supplied the client.
(self) => {
  const state = self.__code;
  const source = state?.announced;
  if (!source) return;
  state.announced = null;
  if (state.server) self.constructor.__core.languageServer(state.server).detach(source);
  state.server = null;
  self.emit('code-editor-disconnect', { source });
  self.disconnect();
}
