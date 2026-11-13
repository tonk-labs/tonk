// Put the document named by `source` on a language server, the way
// <tonk-code> does: say so with a bubbling `code-editor-connect`, and
// when no ancestor claims it (`preventDefault`, then `connect(client)`
// on the editor), attach to the core's own session for the
// `language-server` endpoint. No `source`, no language server.
//
// Not `tonk-code-connect`: the host's <tonk-diagnostics-provider>
// answers that one with a client built on the host bundle's CodeMirror,
// which this editor's CodeMirror cannot take.
(self) => {
  const state = self.__code;
  const source = self.getAttribute('source');
  if (!state?.editor || !source || state.announced) return;
  const url = self.getAttribute('language-server') || '/api/language-server';
  const language = self.getAttribute('language') || 'plaintext';
  state.announced = source;
  const event = new CustomEvent('code-editor-connect', {
    detail: { source, language, url }, bubbles: true, composed: true, cancelable: true,
  });
  self.dispatchEvent(event);
  if (event.defaultPrevented) return;
  state.server = url;
  self.constructor.__core.languageServer(url).attach(source, self);
}
