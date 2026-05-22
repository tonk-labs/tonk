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
    CompletionItem, CompletionList, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    MarkupContent, MarkupKind, PositionEncodingKind, PublishDiagnosticsParams, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    notification::{Notification as LspNotificationTrait, PublishDiagnostics},
    request::{Completion, HoverRequest, Initialize, Request as LspRequestTrait},
};
use serde_json::Value;
use tonk_schema::concept::{
    AttributeDescriptor, ConceptDescriptor as DialogConceptDescriptor, Type,
};
use tonk_schema::mutation::ConceptDescriptor;
use tonk_schema::resolution::{ConceptReference, Environment, NamedReference};

use crate::env::EnvProvider;
use crate::jsonrpc::{Incoming, OutboundNotification, Response, ResponseError};

/// Per-URI document state. Today only the latest text is
/// retained; the version that the client tagged the
/// `didOpen` / `didChange` with flows straight to the
/// matching `publishDiagnostics` and isn't stored. Carrying
/// the version through is what lets `@codemirror/lsp-client`
/// discard diagnostic frames whose ranges no longer match the
/// buffer it's editing.
#[derive(Debug, Clone, Default)]
struct Document {
    text: String,
}

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
    documents: HashMap<Uri, Document>,
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
    ///
    /// `env` is the host's [`EnvProvider`], passed **per request**
    /// — the language server resolves diagnostics, completion, and
    /// hover against whatever live environment it opens. The
    /// no-host case passes [`crate::NoEnv`].
    pub async fn handle_message(&mut self, raw: &[u8], env: &impl EnvProvider) -> Option<Vec<u8>> {
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
                let response = self
                    .handle_request(req.id.clone(), &req.method, req.params, env)
                    .await;
                serde_json::to_vec(&response).ok()
            }
            Incoming::Notification(note) => {
                self.handle_notification(&note.method, note.params, env)
                    .await;
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

    async fn handle_request(
        &mut self,
        id: Value,
        method: &str,
        params: Value,
        env: &impl EnvProvider,
    ) -> Response {
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
            Completion::METHOD => match serde_json::from_value::<CompletionParams>(params) {
                Ok(p) => {
                    let items = self.complete(&p, env).await;
                    let response = CompletionResponse::List(CompletionList {
                        is_incomplete: false,
                        items,
                    });
                    match serde_json::to_value(response) {
                        Ok(v) => Response::success(id, v),
                        Err(err) => Response::error(id, ResponseError::internal(err.to_string())),
                    }
                }
                Err(err) => Response::error(id, ResponseError::invalid_params(err.to_string())),
            },
            HoverRequest::METHOD => match serde_json::from_value::<HoverParams>(params) {
                Ok(p) => {
                    let hover = self.hover(&p, env).await;
                    let value = match hover {
                        Some(h) => serde_json::to_value(h),
                        None => Ok(Value::Null),
                    };
                    match value {
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

    async fn handle_notification(&mut self, method: &str, params: Value, env: &impl EnvProvider) {
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
                    let version = p.text_document.version;
                    self.documents
                        .insert(uri.clone(), Document { text: text.clone() });
                    self.publish(uri, &text, Some(version), env).await;
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
                    let version = p.text_document.version;
                    self.documents.insert(
                        uri.clone(),
                        Document {
                            text: last.text.clone(),
                        },
                    );
                    self.publish(uri, &last.text, Some(version), env).await;
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

    /// Compute completion items for a `textDocument/completion`
    /// request. Position-driven dispatch: the trigger character
    /// is a hint, not the contract — what we return is decided
    /// by where the cursor actually sits in the parse tree.
    /// See `docs/auto-completion.md` for the full source taxonomy.
    async fn complete(
        &self,
        params: &CompletionParams,
        env: &impl EnvProvider,
    ) -> Vec<CompletionItem> {
        let uri = &params.text_document_position.text_document.uri;
        let Some(doc) = self.documents.get(uri) else {
            return Vec::new();
        };
        let text = doc.text.as_str();
        let position = params.text_document_position.position;
        let Some(line_prefix) = line_prefix_at(text, position) else {
            return Vec::new();
        };

        // Open the live environment for this document once.
        // Built-in completion always works; branch-side sources
        // only fire when the host opens an environment.
        let environment = open_environment(uri, env).await;

        if is_head_position(&line_prefix) {
            return head_completions(environment.as_ref()).await;
        }

        if is_variable_position(&line_prefix) {
            return variable_completions(text, position);
        }

        if let Some(head) = enclosing_head(text, position)
            && is_body_position(&line_prefix)
        {
            return field_completions(&head, environment.as_ref()).await;
        }

        Vec::new()
    }

    /// Compute hover contents for a `textDocument/hover`
    /// request. Position-driven dispatch:
    ///
    /// - Cursor on a head identifier → concept description
    ///   (built-in registry first, then introspection lookup).
    /// - Cursor on a body field name → backing attribute's
    ///   description, type, cardinality.
    /// - Anywhere else → `None` (the editor renders no
    ///   tooltip).
    async fn hover(&self, params: &HoverParams, env: &impl EnvProvider) -> Option<Hover> {
        let uri = &params.text_document_position_params.text_document.uri;
        let doc = self.documents.get(uri)?;
        let text = doc.text.as_str();
        let position = params.text_document_position_params.position;

        // Identify the identifier under the cursor by widening
        // from the cursor in both directions over symbol chars.
        let line = text.split('\n').nth(position.line as usize)?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        let col = (position.character as usize).min(line.len());
        let word = identifier_at(line, col)?;
        let (start_col, end_col, name) = word;

        let range = Some(lsp_types::Range {
            start: lsp_types::Position {
                line: position.line,
                character: start_col as u32,
            },
            end: lsp_types::Position {
                line: position.line,
                character: end_col as u32,
            },
        });

        // Open the live environment once; head + field hovers
        // both need it to find branch-published concepts.
        let environment = open_environment(uri, env).await;

        // Decide head vs field by indent: any leading whitespace
        // means we're inside a body. Mirrors the completion
        // dispatch — same cursor-position model.
        let line_prefix = &line[..start_col];
        let inside_body = line_prefix.starts_with(|c: char| c.is_whitespace());

        if !inside_body {
            // Head position. The cursor sits on a concept name.
            let descriptor = lookup_concept_descriptor(&name, environment.as_ref()).await?;
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: render_concept_hover(&name, descriptor.concept()),
                }),
                range,
            });
        }

        // Body position. Need the enclosing head's name to look
        // up the concept's field set.
        let head = enclosing_head(text, position)?;
        let descriptor = lookup_concept_descriptor(&head, environment.as_ref()).await?;
        let attr = descriptor
            .concept()
            .with()
            .iter()
            .find(|(field, _)| *field == name.as_str())?
            .1;
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: render_field_hover(&name, attr),
            }),
            range,
        })
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
    ///    we run `tonk_schema::analyzer::analyze` against the live
    ///    environment (when the host opens one) to catch structural
    ///    errors the parser accepts (`AssertionWithoutFields`,
    ///    `ClaimWithoutFields`, etc.) plus `UnknownConcept` for a
    ///    name the environment does not define. Without an
    ///    environment the analyzer runs document-only and
    ///    branch-dependent errors are filtered out.
    async fn publish(
        &mut self,
        uri: Uri,
        text: &str,
        version: Option<i32>,
        env: &impl EnvProvider,
    ) {
        let parsed = tonk_notation::parse(text);
        let mut diagnostics = parsed.diagnostics.clone();
        // Only run the analyzer when the parser is happy — its
        // errors are noise on top of structural parse failures.
        if diagnostics.is_empty()
            && let Some(syntax) = &parsed.syntax
            && !syntax.expressions.is_empty()
        {
            let environment = open_environment(&uri, env).await;
            diagnostics.extend(analyzer_diagnostics(syntax, environment.as_ref()).await);
        }
        // Clamp every range against the live text. The parser
        // already clamps its own emissions, but analyzer
        // diagnostics propagate spans from the syntax tree
        // unchanged — and the same `(line_count, 0)` saphyr
        // edge case lurks there too.
        for diagnostic in &mut diagnostics {
            diagnostic.range = clamp_range_to_text(diagnostic.range, text);
        }
        // Echo the version we just analyzed. `@codemirror/lsp-client`
        // (line 34 of its `diagnostics.ts`) drops frames whose
        // version doesn't match its tracked file version — which is
        // exactly what we want: stale frames must not poison the
        // client's diagnostic state. Omitting the version (which we
        // used to do) made the client apply every frame, including
        // ones whose verdict reflected a prior keystroke — leading
        // to auto-eval skipping the current buffer because
        // `errorCount` was the previous buffer's verdict.
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        };
        let value = match serde_json::to_value(params) {
            Ok(v) => v,
            Err(_) => return,
        };
        self.outbound
            .push(OutboundNotification::new(PublishDiagnostics::METHOD, value));
    }
}

/// Run the analyzer and the structural variable-occurrence scan
/// against the parsed syntax, then merge their findings into a
/// single diagnostic list.
///
/// Two passes:
///
/// 1. **Variable scan** — purely structural, no resolver work,
///    runs unconditionally so warnings surface even when the
///    main analyzer short-circuits.
/// 2. **`tonk_schema::analyzer::analyze`** — catches structural
///    errors the parser accepts. When the host opened a live
///    `environment` the analyzer resolves against it, so
///    `UnknownConcept` reflects what the branch actually defines.
///    Without an environment the analyzer runs document-only
///    (`NoopResolver`) and branch-dependent errors
///    (`UnknownConcept`, `UnknownBookmark`, `ResolverFailed`,
///    `InvalidClaimAttribute`) are filtered out — they would
///    false-positive against the noop resolver.
async fn analyzer_diagnostics<E: Environment + dialog_common::ConditionalSync + ?Sized>(
    syntax: &tonk_notation::Syntax,
    environment: Option<&E>,
) -> Vec<lsp_types::Diagnostic> {
    use tonk_schema::analyzer::{EnvironmentResolver, NoopResolver, analyze, scan_variables};

    let mut out: Vec<lsp_types::Diagnostic> = scan_variables(syntax)
        .into_iter()
        .map(|d| diagnostic_from_analyze_diagnostic(syntax, d))
        .collect();

    // With a live environment the analyzer's branch-dependent
    // errors are accurate and must be kept; without one they are
    // dropped (the noop resolver false-positives every name).
    let live = environment.is_some();
    let result = match environment {
        Some(env) => analyze(syntax, &EnvironmentResolver::new(env)).await,
        None => analyze(syntax, &NoopResolver).await,
    };

    match result {
        Ok(analysis) => {
            // The analyzer's own diagnostics duplicate the scan
            // we already did above — `scan_variables` is also
            // called inside `analyze`. Dedup by (code, range).
            for diag in analysis.diagnostics {
                let mapped = diagnostic_from_analyze_diagnostic(syntax, diag);
                if !out
                    .iter()
                    .any(|existing| existing.code == mapped.code && existing.range == mapped.range)
                {
                    out.push(mapped);
                }
            }
        }
        Err(err) => {
            if let Some(diag) = diagnostic_from_analyze_error(syntax, err, live) {
                out.push(diag);
            }
        }
    }
    out
}

/// Translate an [`AnalyzeDiagnostic`] (warning/error severity)
/// into an LSP [`Diagnostic`]. Falls back to the document range
/// when the diagnostic carries no source span.
fn diagnostic_from_analyze_diagnostic(
    syntax: &tonk_notation::Syntax,
    diagnostic: tonk_schema::analyzer::AnalyzeDiagnostic,
) -> lsp_types::Diagnostic {
    use lsp_types::{Diagnostic, DiagnosticSeverity as LspSeverity, NumberOrString};
    use tonk_schema::analyzer::DiagnosticSeverity;

    let severity = match diagnostic.severity {
        DiagnosticSeverity::Warning => LspSeverity::WARNING,
        DiagnosticSeverity::Error => LspSeverity::ERROR,
    };
    let code = diagnostic.code();
    let message = diagnostic.message();
    let range = diagnostic.range.unwrap_or(syntax.range);
    Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(code.into())),
        code_description: None,
        source: Some("tonk-schema".into()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Translate a single [`AnalyzeError`] into an LSP [`Diagnostic`].
/// When `live` is false the analyzer ran document-only — error
/// categories that need a real branch are dropped, since they
/// would false-positive against the noop resolver. When `live`
/// is true the analyzer resolved against the host environment,
/// so every error category is accurate and kept.
fn diagnostic_from_analyze_error(
    syntax: &tonk_notation::Syntax,
    err: tonk_schema::analyzer::AnalyzeError,
    live: bool,
) -> Option<lsp_types::Diagnostic> {
    use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
    use tonk_schema::analyzer::AnalyzeErrorKind;

    // Without a live environment, skip kinds that depend on a
    // real branch resolver — the worker's evaluate route is the
    // source of truth for those.
    if !live
        && matches!(
            err.kind,
            AnalyzeErrorKind::UnknownConcept { .. }
                | AnalyzeErrorKind::UnknownBookmark { .. }
                | AnalyzeErrorKind::ResolverFailed { .. }
                | AnalyzeErrorKind::InvalidClaimAttribute { .. }
        )
    {
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

/// Clamp `range`'s endpoints so neither one points past the
/// last line of `text`. Mirrors the same clamp the parser
/// applies to its own emissions; we re-do it here for
/// analyzer-derived diagnostics, which propagate spans
/// straight from the syntax tree.
fn clamp_range_to_text(range: lsp_types::Range, text: &str) -> lsp_types::Range {
    let lines: Vec<&str> = text.split('\n').collect();
    let last_line = lines.len().saturating_sub(1) as u32;
    let last_col = lines.last().map(|l| l.len() as u32).unwrap_or(0);
    let clamp = |p: lsp_types::Position| -> lsp_types::Position {
        if (p.line as usize) < lines.len() {
            return p;
        }
        lsp_types::Position {
            line: last_line,
            character: last_col,
        }
    };
    lsp_types::Range {
        start: clamp(range.start),
        end: clamp(range.end),
    }
}

/// Slice of a single line from the document text up to (but
/// not including) `position.character` UTF-16 code units. Returns
/// `None` when the position points past the end of the document
/// or past the end of its line.
///
/// LSP positions are zero-based and use UTF-16 code units, but
/// the carry-asserted documents we serve today are ASCII-clean
/// — every character is one UTF-16 unit — so we treat the
/// `character` field as a byte offset into the line. Switch to
/// UTF-16 stepping when documents start carrying non-BMP text.
fn line_prefix_at(text: &str, position: lsp_types::Position) -> Option<String> {
    // `str::lines` swallows a trailing empty line, so an empty
    // document or one ending in `\n` doesn't yield the line the
    // cursor is on. Splitting on `\n` keeps every line including
    // the trailing empty one.
    let line = text.split('\n').nth(position.line as usize)?;
    // Strip a trailing `\r` so `\r\n` files don't include the
    // carriage return in the prefix.
    let line = line.strip_suffix('\r').unwrap_or(line);
    let col = position.character as usize;
    if col > line.len() {
        return None;
    }
    Some(line[..col].to_owned())
}

/// True when `line_prefix` is consistent with the user typing a
/// fresh head: empty, or only the leading characters of an
/// identifier with no preceding indent or `:` separator.
///
/// Heads in carry-asserted notation live at column zero. An
/// indented prefix is body, a prefix containing `:` is a value,
/// and anything with a `?` or `&` is a value-position token.
fn is_head_position(line_prefix: &str) -> bool {
    // Disqualify if any character above is present.
    if line_prefix
        .chars()
        .any(|c| matches!(c, ':' | '?' | '&' | '!'))
    {
        return false;
    }
    // Disqualify if the line is indented (column > 0 and the
    // prefix starts with whitespace) — that's body position.
    if line_prefix.starts_with(|c: char| c.is_whitespace()) {
        return false;
    }
    true
}

/// True when `line_prefix` is consistent with the user typing a
/// fresh field name in body position: leading indent followed by
/// at most an identifier prefix, no `:` (we'd be past it),
/// no `?` (variable), no `&` (anchor only attaches to head).
fn is_body_position(line_prefix: &str) -> bool {
    if !line_prefix.starts_with(|c: char| c.is_whitespace()) {
        return false;
    }
    if line_prefix
        .chars()
        .any(|c| matches!(c, ':' | '?' | '&' | '!'))
    {
        return false;
    }
    true
}

/// True when the user has typed a `?` and the cursor is sitting
/// at the start of (or inside) the variable name that follows
/// it. Matches `?` and `?ali`, rejects `? ` and `name: ?alice ` —
/// the latter has whitespace between the `?` and the cursor, so
/// it's no longer the active variable token.
fn is_variable_position(line_prefix: &str) -> bool {
    let Some(idx) = line_prefix.rfind('?') else {
        return false;
    };
    let after = &line_prefix[idx + 1..];
    after.chars().all(is_symbol_char)
}

/// Symbol-charset predicate matching the carry-asserted notation:
/// lowercase ASCII letters, digits, `-`, `.`, `+`. No `/` because
/// the variable / anchor namespace forbids namespace separators.
fn is_symbol_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '+')
}

/// Collect every named logic variable (`?<name>`) and anchor
/// (`&<name>`) that appears in `text` strictly *before*
/// `position` and surface each as a completion item. Branch-free.
fn variable_completions(text: &str, position: lsp_types::Position) -> Vec<CompletionItem> {
    use lsp_types::CompletionItemKind;
    use std::collections::BTreeSet;

    let cutoff = byte_offset_of(text, position);
    let scope = &text[..cutoff];
    let mut names: BTreeSet<String> = BTreeSet::new();

    for sigil in ['?', '&'] {
        for (idx, _) in scope.match_indices(sigil) {
            let rest = &scope[idx + 1..];
            let name: String = rest.chars().take_while(|c| is_symbol_char(*c)).collect();
            if name.is_empty() {
                continue;
            }
            // `&` only attaches to head anchors — i.e. the
            // sigil sits between a head's `:` and its body's
            // `\n`. We don't enforce that strictly; the
            // anchor's name still binds a document-scoped
            // variable everywhere we'd care, and surfacing a
            // false positive (e.g. text containing `&foo`
            // inside a string literal) is benign in a
            // suggestion menu.
            names.insert(name);
        }
    }

    // Drop the in-flight token the user is currently typing so
    // it doesn't show up as its own completion.
    if let Some(line) = text.split('\n').nth(position.line as usize) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let col = (position.character as usize).min(line.len());
        let prefix = &line[..col];
        if let Some(idx) = prefix.rfind('?') {
            let active: String = prefix[idx + 1..]
                .chars()
                .take_while(|c| is_symbol_char(*c))
                .collect();
            if !active.is_empty() {
                names.remove(&active);
            }
        }
    }

    names
        .into_iter()
        .map(|name| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            insert_text: Some(name),
            ..CompletionItem::default()
        })
        .collect()
}

/// Convert an LSP `Position` into a byte offset in `text`. UTF-16
/// caveat from `line_prefix_at` applies — ASCII-clean documents
/// only.
fn byte_offset_of(text: &str, position: lsp_types::Position) -> usize {
    let mut offset = 0usize;
    for (idx, line) in text.split('\n').enumerate() {
        if idx == position.line as usize {
            let line_no_cr = line.strip_suffix('\r').unwrap_or(line);
            let col = (position.character as usize).min(line_no_cr.len());
            return offset + col;
        }
        offset += line.len() + 1; // `+ 1` for the `\n`
    }
    text.len()
}

/// Walk *backwards* from `position` looking for the nearest
/// column-zero head line — `<head>:` or `<head>!:` — and return
/// the head's identifier. Stops at a blank line (interpreted as
/// the prior expression's end).
///
/// Cheap structural scan; we don't reparse the document. The
/// parser's `Syntax` could give a more accurate answer once we
/// teach the analyzer to surface partial trees, but for the
/// common case ("user is typing inside the obvious enclosing
/// body") prefix matching on `\n<ident>:` is enough.
fn enclosing_head(text: &str, position: lsp_types::Position) -> Option<String> {
    let line_index = position.line as usize;
    let prior: Vec<&str> = text.split('\n').take(line_index).collect();
    for line in prior.into_iter().rev() {
        let trimmed = line.strip_suffix('\r').unwrap_or(line);
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with(|c: char| c.is_whitespace()) {
            // Still inside a body; keep walking up.
            continue;
        }
        // Column-zero line. Strip the optional `!` and require
        // an `:` somewhere; everything before that is the head.
        let head = trimmed.split(':').next().unwrap_or("");
        let head = head.strip_suffix('!').unwrap_or(head);
        if head.is_empty() {
            return None;
        }
        return Some(head.to_owned());
    }
    None
}

/// Field names declared by the head's concept descriptor, plus
/// the always-available `this:` meta-key. Looks up the concept
/// in the built-in registry first; falls back to resolving
/// against the live environment so branch-published concepts
/// contribute their fields too.
async fn field_completions<E: Environment + ?Sized>(
    head: &str,
    environment: Option<&E>,
) -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};

    let Some(descriptor) = lookup_concept_descriptor(head, environment).await else {
        return Vec::new();
    };

    let mut out: Vec<CompletionItem> = Vec::new();
    // `this:` is always valid — meta-key, not a concept field.
    out.push(CompletionItem {
        label: "this".to_owned(),
        kind: Some(CompletionItemKind::KEYWORD),
        documentation: Some(Documentation::String(
            "Selects the entity the expression operates on.".to_owned(),
        )),
        insert_text: Some("this: ".to_owned()),
        ..CompletionItem::default()
    });
    for (field, attr) in descriptor.concept().with().iter() {
        let description = attr.description();
        out.push(CompletionItem {
            label: field.to_owned(),
            kind: Some(CompletionItemKind::FIELD),
            documentation: if description.is_empty() {
                None
            } else {
                Some(Documentation::String(description.to_owned()))
            },
            insert_text: Some(format!("{field}: ")),
            ..CompletionItem::default()
        });
    }
    out
}

/// Concept names available in head position — built-ins plus
/// every branch-published concept the live environment holds.
/// The branch source is folded in only when the host opened an
/// environment; tests and document-only runs stay on built-ins.
async fn head_completions<E: Environment + ?Sized>(environment: Option<&E>) -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};
    use std::collections::HashSet;
    use tonk_schema::builtin::concept_registry;

    let mut emitted: HashSet<String> = HashSet::new();
    let mut out: Vec<CompletionItem> = Vec::new();

    for (name, definition) in concept_registry().iter() {
        emitted.insert((*name).to_owned());
        out.push(CompletionItem {
            label: (*name).to_owned(),
            kind: Some(CompletionItemKind::CLASS),
            documentation: definition
                .descriptor
                .concept()
                .description()
                .map(|d| Documentation::String(d.to_owned())),
            insert_text: Some((*name).to_owned()),
            ..CompletionItem::default()
        });
    }

    if let Some(env) = environment {
        // Branch concepts come back via `list_names` joined with
        // `list_concepts` — only published names make sense as
        // head-position completions, since the user types the
        // name, not the entity URI. Built-in names take
        // precedence on collision (matches the analyzer's
        // resolution order).
        let names = env.list_names().await.unwrap_or_default();
        let concepts = env.list_concepts().await.unwrap_or_default();
        let concept_entities: HashSet<_> = concepts.iter().map(|c| c.entity.to_string()).collect();
        for named in names {
            // `Name` carries `this` (`id:<name>`) and `entity`
            // (the referent). Only `id:`-shaped names are
            // user-typeable in head position.
            let Some(label) = named
                .this
                .to_string()
                .strip_prefix("id:")
                .map(str::to_owned)
            else {
                continue;
            };
            let target = &named.entity.0;
            if !concept_entities.contains(&target.to_string()) {
                continue;
            }
            if !emitted.insert(label.clone()) {
                continue;
            }
            // The concept's own description (when set) makes
            // a better hover than the bare name alone.
            let description = concepts
                .iter()
                .find(|c| &c.entity == target)
                .and_then(|c| c.descriptor.concept().description())
                .map(|d| Documentation::String(d.to_owned()));
            out.push(CompletionItem {
                label: label.clone(),
                kind: Some(CompletionItemKind::CLASS),
                documentation: description,
                insert_text: Some(label),
                ..CompletionItem::default()
            });
        }
    }

    out
}

/// Find the identifier surrounding `col` on a single line. Returns
/// `(start_col, end_col, name)` widened in both directions over
/// [`is_symbol_char`]. `None` when the cursor isn't sitting on an
/// identifier character (or one immediately to its left, which is
/// the usual hover position when the cursor is just past a word).
fn identifier_at(line: &str, col: usize) -> Option<(usize, usize, String)> {
    if line.is_empty() {
        return None;
    }
    let bytes = line.as_bytes();
    // Anchor inside the word: prefer the byte at `col`, fall back
    // to `col - 1` when the cursor sits right after the word.
    let anchor = if col < bytes.len() && is_symbol_char(bytes[col] as char) {
        col
    } else if col > 0 && is_symbol_char(bytes[col - 1] as char) {
        col - 1
    } else {
        return None;
    };

    let mut start = anchor;
    while start > 0 && is_symbol_char(bytes[start - 1] as char) {
        start -= 1;
    }
    let mut end = anchor;
    while end < bytes.len() && is_symbol_char(bytes[end] as char) {
        end += 1;
    }
    let name = line[start..end].to_owned();
    if name.is_empty() {
        return None;
    }
    Some((start, end, name))
}

/// Resolve a concept name to its durability-tagged descriptor —
/// built-in registry first, then the live environment. Returns
/// the descriptor (without the surrounding `ConceptDefinition`
/// envelope) since hover and completion only need the field set.
async fn lookup_concept_descriptor<E: Environment + ?Sized>(
    name: &str,
    environment: Option<&E>,
) -> Option<ConceptDescriptor> {
    use tonk_schema::builtin::lookup_concept;
    if let Some(definition) = lookup_concept(name) {
        return Some(definition.descriptor);
    }
    let env = environment?;
    let reference = ConceptReference::from(NamedReference(name.to_owned()));
    match env.resolve_concept(reference).await {
        Ok(Some(definition)) => Some(definition.descriptor),
        _ => None,
    }
}

/// Render a Markdown hover body for a concept: the bold name, then
/// the description (if any) and the field list. The field list is
/// useful even when there's no description — it documents the
/// shape the head accepts.
fn render_concept_hover(name: &str, descriptor: &DialogConceptDescriptor) -> String {
    let mut out = format!("**{name}** _(concept)_");
    if let Some(desc) = descriptor.description() {
        out.push_str("\n\n");
        out.push_str(desc);
    }
    let fields = descriptor.with();
    if fields.iter().len() > 0 {
        out.push_str("\n\n**Fields**");
        for (field, attr) in fields.iter() {
            let card = format!("{:?}", attr.cardinality()).to_lowercase();
            let ty = match attr.content_type() {
                Some(t) => format!(" : {}", format_type(&t)),
                None => String::new(),
            };
            out.push_str(&format!("\n- `{field}`{ty} — {card}"));
            let d = attr.description();
            if !d.is_empty() {
                out.push_str(" — ");
                out.push_str(d);
            }
        }
    }
    out
}

/// Render a Markdown hover body for a body-position field: the
/// backing attribute's qualified name, type, cardinality, and
/// description.
fn render_field_hover(field: &str, attr: &AttributeDescriptor) -> String {
    let card = format!("{:?}", attr.cardinality()).to_lowercase();
    let ty = match attr.content_type() {
        Some(t) => format_type(&t),
        None => "any".to_owned(),
    };
    let mut out = format!(
        "**{field}** _(field)_\n\n`{}/{}` : {ty} — {card}",
        attr.domain(),
        attr.name(),
    );
    let d = attr.description();
    if !d.is_empty() {
        out.push_str("\n\n");
        out.push_str(d);
    }
    out
}

/// JSON-shaped rendering of a [`Type`]. The type enum has no
/// `Display`; serializing through serde gives the same shape used
/// in concept descriptors elsewhere, which is the form the user
/// already sees in error messages.
fn format_type(ty: &Type) -> String {
    serde_json::to_string(ty).unwrap_or_else(|_| format!("{ty:?}"))
}

/// Open the host's live environment for a document URI.
///
/// The URI is parsed to `(repo, branch)` and handed to the
/// host's [`EnvProvider`]. Returns `None` when the URI doesn't
/// name a branch the host knows — completion, hover, and
/// diagnostics then degrade to the document-local sources.
async fn open_environment<P: EnvProvider>(uri: &Uri, env: &P) -> Option<P::Env> {
    let (repo, branch) = parse_repo_branch(uri)?;
    env.environment(&repo, &branch).await
}

/// Pull `(repo, branch)` out of a
/// `tonk-buffer:///<repo>/<branch>/<cell-suffix>` URI — the shape
/// the editor's `<tonk-code source>` sets. Returns `None` for any
/// other shape, including profile buffers (which use `<profile>`
/// as the repo segment).
fn parse_repo_branch(uri: &Uri) -> Option<(String, String)> {
    let rest = uri.as_str().strip_prefix("tonk-buffer:///")?;
    // First segment is repo (or `<profile>`); second is branch.
    let mut parts = rest.splitn(3, '/');
    let repo = parts.next()?;
    let branch = parts.next()?;
    if repo.is_empty() || branch.is_empty() || repo == "<profile>" {
        return None;
    }
    Some((repo.to_owned(), branch.to_owned()))
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
        // Trigger on newline (concept-name / field-name surfaces
        // depending on cursor indent) and `?` (variables only).
        // `:` is intentionally not a trigger — value position can
        // hold a variable, a reference, or a constant; auto-firing
        // would surface the wrong thing more often than the right.
        // See `docs/auto-completion.md`.
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec!["\n".into(), "?".into()]),
            resolve_provider: Some(false),
            ..CompletionOptions::default()
        }),
        // Hover surfaces concept descriptions in head position
        // and the backing attribute's description / type /
        // cardinality for fields inside a body. Variables get
        // a one-liner pointing back at the introducing
        // expression. See `Server::hover`.
        hover_provider: Some(HoverProviderCapability::Simple(true)),
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
            .handle_message(&bytes, &crate::env::NoEnv)
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

    /// `person!:\n  this: ?alice\n  age: 29` — the user's
    /// exact reproducer. `person` is unknown to the LSP's noop
    /// resolver, but the structural variable scan still
    /// surfaces a warning on the lone `?alice` because it runs
    /// independently of concept resolution.
    #[dialog_common::test]
    async fn it_warns_on_lone_assertion_this_variable_even_when_concept_unknown() {
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
                    "text": "person!:\n  this: ?alice\n  age: 29\n"
                }
            }
        });
        run(&mut server, &note).await;
        let outbound = server.take_outbound();
        let diags = outbound[0].params["diagnostics"].as_array().unwrap();
        let codes: Vec<&str> = diags
            .iter()
            .map(|d| d["code"].as_str().unwrap_or_default())
            .collect();
        assert!(
            codes.contains(&"W_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_THIS"),
            "expected the lone ?alice warning, got codes {codes:?}"
        );
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

    /// Open an empty document, ask for completions at column 0
    /// — the head position should surface every built-in
    /// concept (`attribute`, `concept`, `name`, `branch`,
    /// `replica`, `remote`, `tracking-branch`).
    #[dialog_common::test]
    async fn it_offers_builtin_concepts_at_head_position() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": ""
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test" },
                "position": { "line": 0, "character": 0 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or_default())
            .collect();
        for builtin in ["attribute", "concept", "name", "branch"] {
            assert!(
                labels.contains(&builtin),
                "expected `{builtin}` in completion list, got {labels:?}",
            );
        }
    }

    /// Inside a body (line indented, cursor past the indent)
    /// the head-set is suppressed. Field completions for the
    /// enclosing concept fire instead — `attribute:` is a
    /// built-in whose descriptor declares `id`, `type`,
    /// `cardinality`, `description`. Plus the always-available
    /// `this:` meta-key.
    #[dialog_common::test]
    async fn it_offers_concept_fields_inside_body() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": "attribute!:\n  "
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test" },
                "position": { "line": 1, "character": 2 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or_default())
            .collect();
        // Built-in `attribute` concept declares these fields.
        for field in ["this", "id", "type", "cardinality", "description"] {
            assert!(
                labels.contains(&field),
                "expected `{field}` in field completions, got {labels:?}",
            );
        }
        // No concept names should leak through.
        assert!(
            !labels.contains(&"attribute"),
            "head set must not surface inside body; got {labels:?}",
        );
    }

    /// Typing `?` after introducing a few variables earlier in
    /// the document offers them as completions. The active
    /// in-flight token (the variable the user is typing right
    /// now) must not surface as its own completion.
    #[dialog_common::test]
    async fn it_offers_prior_variables_after_question_mark() {
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
        // First expression introduces ?alice and the &maintainer
        // anchor. Second expression's body is being typed —
        // cursor sits right after `?` on its `this:` line.
        let text = "person:\n  this: ?alice\n  name: \"Alice\"\n\nattribute!: &maintainer\n  the: x.y/role\n  as: text\n  cardinality: one\n  description: ok\n\nperson!:\n  this: ?";
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": text,
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        // Cursor sits right after `this: ?` on the last line.
        let last_line = text.split('\n').count() - 1;
        let last_col = text
            .split('\n')
            .next_back()
            .map(str::len)
            .unwrap_or_default();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test" },
                "position": { "line": last_line as u32, "character": last_col as u32 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or_default())
            .collect();
        assert!(
            labels.contains(&"alice"),
            "expected `alice` from prior `?alice`; got {labels:?}",
        );
        assert!(
            labels.contains(&"maintainer"),
            "expected `maintainer` from prior `&maintainer`; got {labels:?}",
        );
    }

    /// `publishDiagnostics` must echo the version of the document
    /// the verdict was computed against. Without it,
    /// `@codemirror/lsp-client` cannot drop frames that arrive out
    /// of order — and a stale frame's `errorCount` would then poison
    /// the editor's freshness gate, blocking auto-evaluate on the
    /// *current* buffer because the previous buffer's verdict is
    /// still believed to be live.
    #[dialog_common::test]
    async fn it_echoes_version_in_publish_diagnostics() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test/no-version",
                        "languageId": "carry-asserted",
                        "version": 7,
                        "text": "person:\n"
                    }
                }
            }),
        )
        .await;
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].method, "textDocument/publishDiagnostics");
        // The frame must carry the version we received on `didOpen`.
        let version = &outbound[0].params["version"];
        assert_eq!(
            version,
            &json!(7),
            "publishDiagnostics must echo the document version",
        );
    }

    /// Likewise on `didChange` — every frame carries the
    /// `didChange`'s version so the client can drop frames that
    /// arrive after the buffer has moved on.
    #[dialog_common::test]
    async fn it_echoes_version_in_publish_diagnostics_on_change() {
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
        let uri = "tonk-buffer:///test/no-version-change";
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri, "languageId": "carry-asserted",
                        "version": 1, "text": "person:\n"
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": 42 },
                    "contentChanges": [{ "text": "person:\n  name: \"Alice\"\n" }]
                }
            }),
        )
        .await;
        let outbound = server.take_outbound();
        assert_eq!(outbound.len(), 1);
        let version = &outbound[0].params["version"];
        assert_eq!(
            version,
            &json!(42),
            "publishDiagnostics on didChange must echo the didChange version",
        );
    }

    /// Hovering on a built-in concept name in head position
    /// returns a Markdown body that names the concept and at
    /// least one of its declared fields. We don't pin the exact
    /// text — only that hover resolves and surfaces the
    /// descriptor.
    #[dialog_common::test]
    async fn it_hovers_builtin_concept_in_head() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test/hover-head",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": "attribute:\n"
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test/hover-head" },
                "position": { "line": 0, "character": 3 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let value = reply["result"]["contents"]["value"]
            .as_str()
            .expect("markdown body");
        assert!(
            value.contains("**attribute**"),
            "expected concept name in hover body; got {value}",
        );
        assert!(
            value.contains("`id`"),
            "expected `id` field in hover body; got {value}",
        );
    }

    /// Hovering on a body-position field name returns the
    /// backing attribute's qualified name and cardinality.
    #[dialog_common::test]
    async fn it_hovers_concept_field_in_body() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test/hover-field",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": "attribute:\n  id\n"
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test/hover-field" },
                "position": { "line": 1, "character": 3 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let value = reply["result"]["contents"]["value"]
            .as_str()
            .expect("markdown body");
        assert!(
            value.contains("**id**"),
            "expected field name in hover body; got {value}",
        );
        assert!(
            value.to_lowercase().contains("one") || value.to_lowercase().contains("many"),
            "expected cardinality in hover body; got {value}",
        );
    }

    /// Hovering off any identifier returns no hover (`result:
    /// null`), not an error.
    #[dialog_common::test]
    async fn it_returns_null_hover_off_identifier() {
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
        run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "tonk-buffer:///test/hover-empty",
                        "languageId": "carry-asserted",
                        "version": 1,
                        "text": "\n"
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test/hover-empty" },
                "position": { "line": 0, "character": 0 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        assert!(
            reply["result"].is_null(),
            "expected null hover off identifier; got {}",
            reply["result"],
        );
    }
}
