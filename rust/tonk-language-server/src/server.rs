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
    /// Open documents, keyed by their LSP `Uri`. The sync model
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
    pub async fn handle_message(&mut self, raw: &[u8]) -> Option<Vec<u8>> {
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
                self.handle_notification(&note.method, note.params).await;
                None
            }
            Incoming::Response { .. } => {
                // Server-initiated requests aren't used yet, so any
                // incoming response is a protocol misuse we
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

    async fn handle_notification(&mut self, method: &str, params: Value) {
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
                    self.publish(uri, &text).await;
                }
            }
            "textDocument/didChange" => {
                if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(params) {
                    // Sync model is "full" — the spec says exactly
                    // one change with no `range`; treat any change
                    // shape as full text by taking the last entry.
                    // Tighten when
                    // we move to incremental sync.
                    let Some(last) = p.content_changes.into_iter().last() else {
                        return;
                    };
                    let uri = p.text_document.uri;
                    self.documents.insert(uri.clone(), last.text.clone());
                    self.publish(uri, &last.text).await;
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
    ///
    /// Two passes:
    ///
    /// 1. **Parser** — `tonk_notation::document_diagnostics` returns
    ///    structural diagnostics (unbalanced indentation, missing
    ///    colon, etc.). The parser is permissive: it produces a
    ///    `Syntax` tree even when there are recoverable errors.
    /// 2. **Analyzer** — when the parser returns *no* diagnostics,
    ///    we run `tonk_schema::analyzer::analyze` with a
    ///    `NoopResolver` to catch structural errors the parser
    ///    accepts (`AssertionWithoutFields`, `ClaimWithoutFields`,
    ///    etc.). Errors that need the branch (`UnknownConcept`,
    ///    `UnknownBookmark`, `ResolverFailed`) are filtered out —
    ///    they need a real branch resolver and the worker's
    ///    evaluate route is the source of truth for those.
    async fn publish(&mut self, uri: Uri, text: &str) {
        let parsed = tonk_notation::parse(text);
        let mut diagnostics = parsed.diagnostics.clone();
        // Only run the analyzer when the parser is happy — its
        // errors are noise on top of structural parse failures.
        if diagnostics.is_empty()
            && let Some(syntax) = &parsed.syntax
            && !syntax.expressions.is_empty()
        {
            diagnostics.extend(analyzer_diagnostics(syntax).await);
        }
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

/// Run `tonk_schema::analyzer::analyze` against the parsed
/// syntax with a `NoopResolver` and surface the result as LSP
/// diagnostics.
///
/// Errors that fundamentally need the branch (`UnknownConcept`,
/// `UnknownBookmark`, `ResolverFailed`, `InvalidClaimAttribute`)
/// are filtered out here — they trigger spuriously under the
/// noop resolver and are the worker's responsibility once
/// `evaluate` runs against a real branch.
///
/// The analyzer short-circuits on the first error today, so
/// callers see at most one diagnostic per pass. Multi-error
/// reporting is a future analyzer change; once it lands, this
/// function picks it up automatically by virtue of mapping
/// every error in the result.
async fn analyzer_diagnostics(syntax: &tonk_notation::Syntax) -> Vec<lsp_types::Diagnostic> {
    use tonk_schema::analyzer::{NoopResolver, analyze};

    let resolver = NoopResolver;
    match analyze(syntax, &resolver).await {
        Ok(_) => Vec::new(),
        Err(err) => match diagnostic_from_analyze_error(syntax, err) {
            Some(diag) => vec![diag],
            None => Vec::new(),
        },
    }
}

/// Translate a single [`AnalyzeError`] into an LSP [`Diagnostic`].
/// Returns `None` for error categories that need a real branch
/// to evaluate accurately (and would therefore false-positive
/// against the LSP's `NoopResolver`).
fn diagnostic_from_analyze_error(
    syntax: &tonk_notation::Syntax,
    err: tonk_schema::analyzer::AnalyzeError,
) -> Option<lsp_types::Diagnostic> {
    use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
    use tonk_schema::analyzer::AnalyzeErrorKind;

    // Skip kinds that depend on a real branch resolver — the
    // worker is the source of truth for those.
    if matches!(
        err.kind,
        AnalyzeErrorKind::UnknownConcept { .. }
            | AnalyzeErrorKind::UnknownBookmark { .. }
            | AnalyzeErrorKind::ResolverFailed { .. }
            | AnalyzeErrorKind::InvalidClaimAttribute { .. }
    ) {
        return None;
    }

    let code = err.code();
    let message = err.kind.to_string();
    // Fall back to the document range when an error has no
    // span. Better than dropping the diagnostic — the user
    // still sees the message.
    let range = err.range.unwrap_or(syntax.range);
    Some(Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(code.into())),
        code_description: None,
        source: Some("tonk-schema".into()),
        message,
        related_information: None,
        tags: None,
        data: None,
    })
}

/// Capabilities advertised in the `initialize` response. Kept in one
/// place so future features (completion, hover, etc.) extend a single
/// constant rather than scattering capability flags across handlers.
fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Transport is "full text on every change." Cheap for
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
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    async fn run(server: &mut Server, msg: &Value) -> Option<Value> {
        let bytes = serde_json::to_vec(msg).unwrap();
        server
            .handle_message(&bytes)
            .await
            .map(|reply| serde_json::from_slice(&reply).unwrap())
    }

    #[dialog_common::test]
    async fn initialize_returns_capabilities() {
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
        let reply = run(&mut server, &req).await.expect("response");
        assert_eq!(reply["id"], json!(1));
        assert!(reply["result"]["capabilities"].is_object());
    }

    #[dialog_common::test]
    async fn did_open_clean_yaml_publishes_empty_diagnostics() {
        let mut server = Server::new();
        let _ = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        )
        .await;
        let note = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "tonk-buffer:///test",
                    "languageId": "carry-asserted",
                    "version": 1,
                    "text": "attribute!:\n  the: io.gozala.person/name\n  as: text\n  cardinality: one\n  description: \"The person's full name\"\n"
                }
            }
        });
        run(&mut server, &note).await;
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].method, "textDocument/publishDiagnostics");
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        assert!(diags.is_empty(), "expected clean doc, got {diags:?}");
    }

    #[dialog_common::test]
    async fn did_open_broken_yaml_publishes_one_diagnostic() {
        let mut server = Server::new();
        let _ = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        )
        .await;
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
        run(&mut server, &note).await;
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
    }

    /// Analyzer-derived diagnostics carry a stable `code` and a
    /// source `range` so the editor can underline the offending
    /// span and route quickfixes by code.
    #[dialog_common::test]
    async fn it_publishes_diagnostic_with_code_and_range_from_analyzer() {
        let mut server = Server::new();
        let _ = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
        )
        .await;
        // `person!:` (empty body) — analyzer rejects with
        // E_ASSERTION_WITHOUT_FIELDS, range pinned to the head.
        let note = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "tonk-buffer:///test",
                    "languageId": "carry-asserted",
                    "version": 1,
                    "text": "person!:\n"
                }
            }
        });
        run(&mut server, &note).await;
        let outbound = server.take_outbound();
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        let diag = &diags[0];
        assert_eq!(diag["code"], json!("E_ASSERTION_WITHOUT_FIELDS"));
        assert_eq!(diag["source"], json!("tonk-schema"));
        // Range covers the head on line 0.
        assert_eq!(diag["range"]["start"]["line"], json!(0));
    }
}
