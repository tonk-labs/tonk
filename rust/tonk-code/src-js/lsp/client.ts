// LSP client construction.
//
// Each editor instance owns its own `LSPClient`. We *don't*
// share a single client across all `<tonk-code>` editors on a
// page because the rebuild-on-drop lifecycle is per-instance:
// when a transport closes, we tear down only the affected
// editor's session and rebuild against a fresh transport.
//
// Building per-editor is also much simpler than tracking
// dependents on a shared client; the LSP `Workspace`
// abstraction lets clients trivially manage one document each
// without any coordination.

import {
  LSPClient,
  languageServerExtensions,
  type Transport,
} from "@codemirror/lsp-client";

export interface BuildOptions {
  /** Project root URI passed to the server in `initialize`. Used
   *  for protocol completeness — our server doesn't read it. */
  rootUri?: string;
}

/** Build an `LSPClient` and connect it to the given transport.
 *  Returns the connected client. The caller is responsible for
 *  destroying the client (via `client.destroy()`) when it's no
 *  longer needed. */
export function connectLsp(transport: Transport, opts: BuildOptions = {}): LSPClient {
  return new LSPClient({
    extensions: languageServerExtensions(),
    rootUri: opts.rootUri ?? "tonk-buffer:///",
  }).connect(transport);
}
