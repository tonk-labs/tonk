//! The language server itself.
//!
//! Hosts construct a single [`Server`], pump incoming JSON-RPC
//! messages through [`Server::handle_message`], and drain server-
//! initiated notifications via [`Server::take_outbound`] for
//! forwarding back to the client. The server is `!Sync` and meant to
//! live behind whatever locking primitive the host environment
//! provides — `Rc<RefCell<…>>` in single-threaded wasm; `Arc<Mutex<…>>`
//! in a tokio host.

use std::collections::HashMap;

use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, InitializeResult, PositionEncodingKind, PublishDiagnosticsParams,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    notification::{Notification as LspNotificationTrait, PublishDiagnostics},
    request::{Initialize, Request as LspRequestTrait},
};
use serde_json::Value;

use crate::jsonrpc::{Incoming, OutboundNotification, Response, ResponseError};

/// LSP language server for `tonk-notation` documents.
///
/// One instance serves any number of open documents (keyed by `Uri`)
/// over a single message channel. Hosts that want isolation between
/// editor instances should construct one server per channel — there's
/// no global state inside the server, so multiple instances coexist
/// freely.
#[derive(Default)]
pub struct Server {
    /// Open documents, keyed by their LSP `Uri`. Phase-0 sync model
    /// is "full" — every `didChange` carries the new text, so we
    /// just replace the value.
    documents: HashMap<Uri, String>,
    /// Server-initiated notifications waiting for the host to forward.
    /// Drained by [`Server::take_outbound`].
    outbound: Vec<OutboundNotification>,
    /// Whether the client has issued `initialize`. Per spec we
    /// shouldn't process most requests until then. We're lenient
    /// today — flag exists so we can tighten later without an API
    /// change.
    initialized: bool,
    /// Set after `shutdown`. Subsequent requests must error per spec.
    shutting_down: bool,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain queued server-to-client notifications. The host calls
    /// this after every `handle_message` and forwards the results
    /// over its push channel (SSE today). Returning `Vec` rather than
    /// an iterator keeps the borrow story simple — the host owns the
    /// vec and can serialize at its leisure.
    pub fn take_outbound(&mut self) -> Vec<OutboundNotification> {
        std::mem::take(&mut self.outbound)
    }

    /// Process one incoming JSON-RPC message. Returns the serialized
    /// response when the message was a request; returns `None` for
    /// notifications and for malformed JSON we couldn't even parse
    /// far enough to extract an `id`. Hosts forward `Some(bytes)`
    /// directly to the client.
    ///
    /// Server-pushed notifications generated as a side effect (e.g.
    /// publishDiagnostics fired by `didChange`) are queued in the
    /// outbound buffer; the host pulls them with `take_outbound`.
    pub fn handle_message(&mut self, raw: &[u8]) -> Option<Vec<u8>> {
        let parsed: Incoming = match serde_json::from_slice(raw) {
            Ok(p) => p,
            Err(err) => {
                // No id available — best we can do is drop the message.
                // Real LSP transports treat this as a connection error;
                // we follow suit.
                let _ = err;
                return None;
            }
        };

        match parsed {
            Incoming::Request(req) => {
                let response = self.handle_request(req.id.clone(), &req.method, req.params);
                serde_json::to_vec(&response).ok()
            }
            Incoming::Notification(note) => {
                self.handle_notification(&note.method, note.params);
                None
            }
            Incoming::Response { .. } => {
                // Server-initiated requests are not used in phase 0;
                // any incoming response is a protocol misuse we
                // silently drop.
                None
            }
        }
    }

    fn handle_request(&mut self, id: Value, method: &str, params: Value) -> Response {
        if self.shutting_down && method != "exit" {
            return Response::error(id, ResponseError::internal("server is shutting down"));
        }

        match method {
            Initialize::METHOD => match serde_json::from_value::<InitializeParams>(params) {
                Ok(_init) => {
                    self.initialized = true;
                    let result = InitializeResult {
                        capabilities: server_capabilities(),
                        server_info: Some(ServerInfo {
                            name: tonk_notation::SERVER_INFO.name.into(),
                            version: Some(tonk_notation::SERVER_INFO.version.into()),
                        }),
                    };
                    match serde_json::to_value(result) {
                        Ok(v) => Response::success(id, v),
                        Err(err) => Response::error(id, ResponseError::internal(err.to_string())),
                    }
                }
                Err(err) => Response::error(id, ResponseError::invalid_params(err.to_string())),
            },
            "shutdown" => {
                self.shutting_down = true;
                Response::success(id, Value::Null)
            }
            other => Response::error(id, ResponseError::method_not_found(other)),
        }
    }

    fn handle_notification(&mut self, method: &str, params: Value) {
        match method {
            "initialized" => {
                // No payload; `initialize` already flipped the flag.
            }
            "exit" => {
                // Host is responsible for actually tearing down the
                // transport; we just record state.
                self.shutting_down = true;
            }
            "textDocument/didOpen" => {
                if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(params) {
                    let uri = p.text_document.uri;
                    let text = p.text_document.text;
                    self.documents.insert(uri.clone(), text.clone());
                    self.publish(uri, &text);
                }
            }
            "textDocument/didChange" => {
                if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(params) {
                    // Phase-0 sync is "full" — the spec says exactly one
                    // change with no `range`; treat any change shape as
                    // full text by taking the last entry. Tighten when
                    // we move to incremental sync.
                    let Some(last) = p.content_changes.into_iter().last() else {
                        return;
                    };
                    let uri = p.text_document.uri;
                    self.documents.insert(uri.clone(), last.text.clone());
                    self.publish(uri, &last.text);
                }
            }
            "textDocument/didClose" => {
                if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
                    self.documents.remove(&p.text_document.uri);
                }
            }
            _ => {
                // Unknown notification — spec says ignore.
            }
        }
    }

    /// Run validators against `text` and queue a `publishDiagnostics`
    /// notification on the outbound channel. Always emits the
    /// notification, even when the diagnostic list is empty — that's
    /// how clients learn an error has been *resolved*.
    fn publish(&mut self, uri: Uri, text: &str) {
        let diagnostics = tonk_notation::document_diagnostics(text);
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        };
        let value = match serde_json::to_value(params) {
            Ok(v) => v,
            Err(_) => return,
        };
        self.outbound
            .push(OutboundNotification::new(PublishDiagnostics::METHOD, value));
    }
}

/// Capabilities advertised in the `initialize` response. Kept in one
/// place so future features (completion, hover, etc.) extend a single
/// constant rather than scattering capability flags across handlers.
fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Phase-0 transport is "full text on every change." Cheap for
        // small carry documents; we'll move to incremental when
        // documents grow.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        // Push diagnostics — we emit `publishDiagnostics` on every
        // change. The pull-diagnostic capability is an alternative
        // mode we don't advertise, so clients won't request it.
        diagnostic_provider: None,
        // We accept UTF-16 positions, which is the LSP default and
        // what `@codemirror/lsp-client` sends. Documenting it here
        // makes the assumption visible.
        position_encoding: Some(PositionEncodingKind::UTF16),
        // Required by `ServerCapabilities` even for unsupported
        // features — `OneOf::Left(false)` is the canonical
        // "explicitly not supported" form.
        workspace: None,
        ..ServerCapabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(server: &mut Server, msg: &Value) -> Option<Value> {
        let bytes = serde_json::to_vec(msg).unwrap();
        server
            .handle_message(&bytes)
            .map(|reply| serde_json::from_slice(&reply).unwrap())
    }

    #[test]
    fn initialize_returns_capabilities() {
        let mut server = Server::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {}
            }
        });
        let reply = run(&mut server, &req).expect("response");
        assert_eq!(reply["id"], json!(1));
        assert!(reply["result"]["capabilities"].is_object());
    }

    #[test]
    fn did_open_clean_yaml_publishes_empty_diagnostics() {
        let mut server = Server::new();
        let _ = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        );
        let note = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "tonk-buffer:///test",
                    "languageId": "carry-asserted",
                    "version": 1,
                    "text": "did:key:zAlice:\n  profile:\n    name: Alice\n"
                }
            }
        });
        run(&mut server, &note);
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].method, "textDocument/publishDiagnostics");
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn did_open_broken_yaml_publishes_one_diagnostic() {
        let mut server = Server::new();
        let _ = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        );
        let note = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "tonk-buffer:///test",
                    "languageId": "carry-asserted",
                    "version": 1,
                    "text": "a:\n  b: 1\n c: 2\n"
                }
            }
        });
        run(&mut server, &note);
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
    }
}
