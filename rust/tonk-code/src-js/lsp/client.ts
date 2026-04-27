// Lazy LSP client singleton.
//
// One `LSPClient` per page session, regardless of how many
// `<tonk-code>` elements are mounted. Each editor opens its own
// document URI through `client.plugin(uri)`; the client multiplexes
// `textDocument/*` traffic over the shared transport.
//
// Singleton lifetime: the first `getClient()` call wires up the
// transport, sends `initialize`, and starts pumping events. Later
// calls return the same instance. We never tear it down — there's
// no scenario where the page wants to drop its language server
// without unloading.

import {
  LSPClient,
  languageServerExtensions,
  type Transport,
} from "@codemirror/lsp-client";
import { serviceWorkerTransport } from "./transport";

let cached: LSPClient | null = null;

export function getClient(): LSPClient {
  if (cached) return cached;
  const transport: Transport = serviceWorkerTransport();
  cached = new LSPClient({
    extensions: languageServerExtensions(),
    // Root URI is informational for our server (it only handles
    // synthetic `tonk-buffer://` URIs from in-memory editor
    // documents). Provide one anyway so the LSP `initialize`
    // params are well-formed.
    rootUri: "tonk-buffer:///",
  }).connect(transport);
  return cached;
}
