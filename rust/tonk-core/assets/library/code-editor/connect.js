// Attach `client`, an LSP client from this editor's core, for the
// document `source` names: diagnostics, hover and completion follow.
// The same attachment twice is a no-op, since a second plugin would
// open the document again before the first closed it.
(self, client) => {
  const state = self.__code;
  const editor = state?.editor;
  const source = self.getAttribute('source');
  if (!editor || !source) return;
  if (!(client instanceof self.constructor.__core.LSPClient)) {
    console.warn('<code-editor> takes a language client from its own core; ignoring', client);
    return;
  }
  const language = self.getAttribute('language') || 'plaintext';
  const attachment = `${source}\n${language}`;
  if (client === state.client && attachment === state.attachment) return;
  // Close the old document before the new plugin opens one.
  if (state.client) editor.setLsp([]);
  state.client = client;
  state.attachment = attachment;
  editor.setLsp(client.plugin(source, language));
}
