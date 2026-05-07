// LSP client construction.
//
// `<tonk-diagnostics-provider>` owns the `LSPClient` and
// shares it across every `<tonk-code>` descendant that
// announces itself via `tonk-code-connect`. Each editor
// becomes one open document on the same client — diagnostics,
// hover, completion etc. all flow through the provider's
// single transport.

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
