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
    CompletionTextEdit, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, MarkupContent, MarkupKind, Position, PositionEncodingKind,
    PublishDiagnosticsParams, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Uri,
    notification::{Notification as LspNotificationTrait, PublishDiagnostics},
    request::{Completion, HoverRequest, Initialize, Request as LspRequestTrait},
};
use serde_json::Value;
use tonk_schema::claim::ConceptDescriptor;
use tonk_schema::concept::{
    AttributeDescriptor, ConceptDescriptor as DialogConceptDescriptor, Type,
};
use tonk_schema::resolution::{ConceptDefinition, ConceptReference, NamedReference};

use crate::env::{EnvProvider, Opened, Repo};
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

        // Open the live `(source, env)` pair for this document
        // once. Built-in completion always works; branch-side
        // sources only fire when the host opens one.
        let opened = open_for(uri, env).await;

        let mut items = if is_head_position(&line_prefix) {
            head_completions(opened.as_ref()).await
        } else if is_premise_assert_position(&line_prefix) {
            // A premise's `assert:` value accepts a concept name, a
            // built-in formula, a constraint, or a resolver — offer
            // all four.
            let mut out = head_completions(opened.as_ref()).await;
            out.extend(formula_completions_items());
            out.extend(constraint_completions_items());
            out.extend(resolver_completions_items());
            out
        } else if is_variable_position(&line_prefix) {
            variable_completions(text, position)
        } else if is_body_position(&line_prefix)
            && let Some(resolver) = enclosing_resolver(text, position)
        {
            resolver_operand_completions(&resolver)
        } else if let Some(head) = enclosing_head(text, position)
            && is_body_position(&line_prefix)
        {
            field_completions(&head, opened.as_ref()).await
        } else {
            return Vec::new();
        };

        // Without an explicit `text_edit`, `@codemirror/lsp-client`
        // computes the replace range from `wordAt`, whose default
        // word regex stops at `/`. A namespaced completion like
        // `tonk/employee` accepted over a typed `tonk/emp` then
        // only replaces `emp`, leaving `tonk/` to produce
        // `tonk/tonk/employee` (BUG-21). Pin each item's edit to the
        // whole partial token the cursor sits in so the client
        // replaces the namespace prefix too.
        let range = completion_replace_range(&line_prefix, position);
        for item in &mut items {
            apply_replace_range(item, range);
        }
        items
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

        // Open the live `(source, env)` pair once; head + field
        // hovers both need it to find branch-published concepts.
        let opened = open_for(uri, env).await;

        // Decide head vs field by indent: any leading whitespace
        // means we're inside a body. Mirrors the completion
        // dispatch — same cursor-position model.
        let line_prefix = &line[..start_col];
        let inside_body = line_prefix.starts_with(|c: char| c.is_whitespace());

        if !inside_body {
            // Head position. The cursor sits on a concept name.
            let descriptor = lookup_concept_descriptor(&name, opened.as_ref()).await?;
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
        let descriptor = lookup_concept_descriptor(&head, opened.as_ref()).await?;
        let attr = descriptor
            .concept()
            .with()
            .iter()
            .find(|(field, _)| *field == name.as_str())?
            .1;
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: render_field_hover(&name, attr.descriptor()),
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
    ///    we run `tonk_analyzer::analyzer::analyze` against the live
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
            let opened = open_for(&uri, env).await;
            diagnostics.extend(analyzer_diagnostics(syntax, opened.as_ref()).await);
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

/// Run analyzer diagnostics against the parsed syntax. Always
/// emits the structural variable-occurrence scan
/// (`scan_variables`); when the host opened an [`Opened`] for
/// this document, additionally runs the full analyzer pass
/// (`tonk_analyzer::analyze`) against its `(source, env)` pair
/// to surface branch-dependent diagnostics (`UnknownConcept`,
/// `UnknownNameReference`, `ResolverFailed`, `InvalidClaimAttribute`).
///
/// Without an opened env the analyzer pass is skipped — there's
/// no `Source` to resolve references against — and the
/// `/evaluate` route remains the authoritative source for
/// branch-dependent verdicts.
async fn analyzer_diagnostics<O: Opened + ?Sized>(
    syntax: &tonk_notation::Syntax,
    opened: Option<&O>,
) -> Vec<lsp_types::Diagnostic> {
    use tonk_analyzer::analyzer::{analyze, scan_variables};

    let mut out: Vec<lsp_types::Diagnostic> = scan_variables(syntax)
        .into_iter()
        .map(|d| diagnostic_from_analyze_diagnostic(syntax, d))
        .collect();

    if let Some(opened) = opened {
        // The analyzer surfaces errors as a single `AnalyzeError`
        // — translate it into one LSP diagnostic pinned to its
        // source range when present.
        if let Err(err) = analyze(syntax, opened.source()).perform(opened.env()).await {
            out.push(diagnostic_from_analyze_error(syntax, err));
        }
    }
    out
}

/// Translate an [`AnalyzeError`] into an LSP [`Diagnostic`].
/// Errors are always [`DiagnosticSeverity::ERROR`] and carry the
/// stable `E_…` code matching the analyzer's error kind.
fn diagnostic_from_analyze_error(
    syntax: &tonk_notation::Syntax,
    error: tonk_analyzer::analyzer::AnalyzeError,
) -> lsp_types::Diagnostic {
    use lsp_types::{Diagnostic, DiagnosticSeverity as LspSeverity, NumberOrString};

    let range = error.range.unwrap_or(syntax.range);
    let code = error.code();
    let message = error.to_string();
    Diagnostic {
        range,
        severity: Some(LspSeverity::ERROR),
        code: Some(NumberOrString::String(code.into())),
        code_description: None,
        source: Some("tonk-schema".into()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Translate an [`AnalyzeDiagnostic`] (warning/error severity)
/// into an LSP [`Diagnostic`]. Falls back to the document range
/// when the diagnostic carries no source span.
fn diagnostic_from_analyze_diagnostic(
    syntax: &tonk_notation::Syntax,
    diagnostic: tonk_analyzer::analyzer::AnalyzeDiagnostic,
) -> lsp_types::Diagnostic {
    use lsp_types::{Diagnostic, DiagnosticSeverity as LspSeverity, NumberOrString};
    use tonk_analyzer::analyzer::DiagnosticSeverity;

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

/// True when the cursor sits in the value of a rule premise's
/// `assert:` key — `  - assert: <cursor>` — where a concept name
/// or a built-in formula name belongs.
///
/// Matches the premise list item shape: leading indent, an
/// optional `-` sequence marker, the `assert` key, its `:`, and
/// then the (possibly partial) value the user is typing. The
/// formula names carry `/`, so the value charset is permissive —
/// anything up to the first whitespace after the `:`.
fn is_premise_assert_position(line_prefix: &str) -> bool {
    let trimmed = line_prefix.trim_start();
    // Drop a leading `-` sequence marker and the space after it.
    let trimmed = trimmed.strip_prefix('-').map_or(trimmed, str::trim_start);
    let Some(value) = trimmed.strip_prefix("assert:") else {
        return false;
    };
    // The cursor is in the value only while no second `:` has
    // been typed (which would mean we're past the value).
    !value.contains(':')
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

/// Charset for the partial token a completion replaces. A superset
/// of [`is_symbol_char`] that also admits `/`: concept and formula
/// names are namespaced (`tonk/employee`, `data/text`), so the token
/// the user is mid-typing spans the namespace separator.
fn is_completion_token_char(c: char) -> bool {
    is_symbol_char(c) || c == '/'
}

/// The range a completion item should replace: the whole partial
/// token immediately to the left of the cursor. Walks back from the
/// cursor over [`is_completion_token_char`], stopping at the `?`/`&`
/// sigil (which is not part of the inserted text), whitespace, or
/// the line start. Returns a zero-width range at the cursor when no
/// token precedes it (a completion triggered on empty space inserts
/// without replacing).
///
/// `line_prefix` is the text from the line start up to the cursor,
/// so its length is the cursor column.
fn completion_replace_range(line_prefix: &str, position: Position) -> Range {
    let end_col = line_prefix.len();
    let start_col = line_prefix
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_completion_token_char(*c))
        .last()
        .map_or(end_col, |(idx, _)| idx);
    Range {
        start: Position {
            line: position.line,
            character: start_col as u32,
        },
        end: Position {
            line: position.line,
            character: end_col as u32,
        },
    }
}

/// Attach `range` to a completion item as an explicit `text_edit`,
/// using the item's `insert_text` (falling back to its `label`) as
/// the replacement. Clears `insert_text` since `text_edit` supersedes
/// it. A no-op when the item already carries its own `text_edit`.
fn apply_replace_range(item: &mut CompletionItem, range: Range) {
    if item.text_edit.is_some() {
        return;
    }
    let new_text = item
        .insert_text
        .take()
        .unwrap_or_else(|| item.label.clone());
    item.text_edit = Some(CompletionTextEdit::Edit(TextEdit { range, new_text }));
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

/// The resolver name governing the `where:` block the cursor sits
/// in, if any.
///
/// A resolver premise nests two ways — as a rule premise
///
/// ```yaml
/// when:
///   - assert: tree/node
///     where:
///       <cursor>
/// ```
///
/// where [`enclosing_head`] would report the enclosing `rule`, not
/// the resolver. So walk up looking for the nearest `assert:` line
/// whose value names a resolver, stopping at a blank line (which
/// ends the block) exactly as `enclosing_head` does.
fn enclosing_resolver(text: &str, position: lsp_types::Position) -> Option<String> {
    let line_index = position.line as usize;
    let prior: Vec<&str> = text.split('\n').take(line_index).collect();
    for line in prior.into_iter().rev() {
        let trimmed = line.strip_suffix('\r').unwrap_or(line);
        if trimmed.trim().is_empty() {
            return None;
        }
        let body = trimmed.trim_start();
        let body = body.strip_prefix('-').map_or(body, str::trim_start);
        if let Some(value) = body.strip_prefix("assert:") {
            // A quoted name (`">"`) is a constraint, never a
            // resolver, and the quotes are not part of the name.
            let name = value.trim().trim_matches(['"', '\'']);
            return tonk_analyzer::analyzer::resolver_operands(name).map(|_| name.to_owned());
        }
    }
    None
}

/// Operand names a resolver's `where:` block accepts, carrying
/// dialog's own descriptions. Required inputs are labelled as such
/// and sort first: `of` must be bound for the premise to run at
/// all, so offering it indistinguishably from a produced binding
/// would invite a query that cannot be planned.
fn resolver_operand_completions(name: &str) -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};

    let Some(operands) = tonk_analyzer::analyzer::resolver_operands(name) else {
        return Vec::new();
    };

    operands
        .into_iter()
        .enumerate()
        .map(|(index, operand)| CompletionItem {
            label: operand.name.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: operand.required.then(|| "required input".to_owned()),
            documentation: (!operand.description.is_empty())
                .then_some(Documentation::String(operand.description)),
            // Preserve the required-first order the registry sorted
            // into; clients otherwise re-sort by label.
            sort_text: Some(format!("{index:03}")),
            insert_text: Some(format!("{}: ", operand.name)),
            ..CompletionItem::default()
        })
        .collect()
}

/// Field names declared by the head's concept descriptor, plus
/// the always-available `this:` meta-key. Looks up the concept
/// in the built-in registry first; falls back to resolving
/// against the live `(source, env)` so branch-published concepts
/// contribute their fields too.
async fn field_completions<O: Opened + ?Sized>(
    head: &str,
    opened: Option<&O>,
) -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};

    let Some(descriptor) = lookup_concept_descriptor(head, opened).await else {
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
/// every branch-published concept the live `(source, env)`
/// holds. The branch source is folded in only when the host
/// opened one; tests and document-only runs stay on built-ins.
async fn head_completions<O: Opened + ?Sized>(opened: Option<&O>) -> Vec<CompletionItem> {
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
            // Name the namespace. In `assert:` position concepts share
            // the list with formulas, constraints and resolvers, and
            // the names can collide (`tree/node` is a resolver, and a
            // branch could publish a concept spelled the same) — the
            // label alone does not say which one you are accepting.
            detail: Some("concept".to_owned()),
            documentation: definition
                .descriptor
                .concept()
                .description()
                .map(|d| Documentation::String(d.to_owned())),
            insert_text: Some((*name).to_owned()),
            // Concepts sort ahead of built-ins: they are what a
            // document mostly names, and the built-in sets are small
            // and fixed.
            sort_text: Some(format!("1{name}")),
            ..CompletionItem::default()
        });
    }

    if let Some(opened) = opened {
        // Branch concepts come back via `NamedReference::list`
        // joined with `ConceptDefinition::list` — only published
        // names make sense as head-position completions, since
        // the user types the name, not the entity URI. Built-in
        // names take precedence on collision (matches the
        // analyzer's resolution order).
        let names = NamedReference::list(opened.source())
            .perform(opened.env())
            .await
            .unwrap_or_default();
        let concepts = ConceptDefinition::list(opened.source())
            .perform(opened.env())
            .await
            .unwrap_or_default();
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

/// Built-in formula names as completion items for a premise's
/// `assert:` value. Sourced from the analyzer's formula registry
/// so the list never drifts from what the analyzer accepts.
fn formula_completions_items() -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};
    use tonk_analyzer::analyzer::formula_completions;

    formula_completions()
        .into_iter()
        .map(|formula| CompletionItem {
            label: formula.name.to_owned(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("formula".to_owned()),
            documentation: Some(Documentation::String(formula.detail)),
            insert_text: Some(formula.name.to_owned()),
            sort_text: Some(format!("2{}", formula.name)),
            ..CompletionItem::default()
        })
        .collect()
}

/// The built-in constraints (`==`) as completion items for a
/// premise's `assert:` value. Sourced from the analyzer's
/// constraint registry so the list never drifts from what the
/// analyzer accepts.
fn constraint_completions_items() -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};
    use tonk_analyzer::analyzer::constraint_completions;

    constraint_completions()
        .into_iter()
        .map(|constraint| CompletionItem {
            label: constraint.name.to_owned(),
            kind: Some(CompletionItemKind::OPERATOR),
            detail: Some("constraint".to_owned()),
            documentation: Some(Documentation::String(constraint.detail)),
            sort_text: Some(format!("2{}", constraint.name)),
            // Insert the notation form, not the bare name: an
            // unquoted `>` would parse as a folded scalar.
            insert_text: Some(constraint.insert.clone()),
            ..CompletionItem::default()
        })
        .collect()
}

/// Every built-in resolver as a completion item. Reads the analyzer's
/// resolver registry so the list never drifts from what the analyzer
/// accepts — the `tree/*` family that replaced the worker-intercepted
/// tree formulas.
fn resolver_completions_items() -> Vec<CompletionItem> {
    use lsp_types::{CompletionItemKind, Documentation};
    use tonk_analyzer::analyzer::resolver_completions;

    resolver_completions()
        .into_iter()
        .map(|resolver| CompletionItem {
            label: resolver.name.to_owned(),
            // A resolver reads structure out of the store rather than
            // computing a value, so it presents as a function-like
            // premise head, the same as a formula.
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("resolver".to_owned()),
            documentation: Some(Documentation::String(resolver.detail)),
            insert_text: Some(resolver.name.to_owned()),
            sort_text: Some(format!("2{}", resolver.name)),
            ..CompletionItem::default()
        })
        .collect()
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
/// built-in registry first, then the live `(source, env)` pair.
/// Returns the descriptor (without the surrounding
/// `ConceptDefinition` envelope) since hover and completion only
/// need the field set.
async fn lookup_concept_descriptor<O: Opened + ?Sized>(
    name: &str,
    opened: Option<&O>,
) -> Option<ConceptDescriptor> {
    use tonk_schema::builtin::lookup_concept;
    if let Some(definition) = lookup_concept(name) {
        return Some(definition.descriptor);
    }
    let opened = opened?;
    let reference = ConceptReference::from(NamedReference(name.to_owned()));
    match reference
        .resolve(opened.source())
        .perform(opened.env())
        .await
    {
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
            // Optional fields render as `field?` with an `(optional)`
            // tag, mirroring the `maybe:` block they were declared in.
            let opt_mark = if attr.is_optional() { "?" } else { "" };
            let opt_tag = if attr.is_optional() {
                " _(optional)_"
            } else {
                ""
            };
            out.push_str(&format!("\n- `{field}{opt_mark}`{ty} — {card}{opt_tag}"));
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
    let mut out = format!("**{field}** _(field)_\n\n`{}` : {ty} — {card}", attr.the(),);
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

/// Open the host's live `(source, env)` pair for a document URI.
///
/// The URI is parsed to `(repo, branch)` and handed to the
/// host's [`EnvProvider`]. Returns `None` when the URI doesn't
/// name a branch the host knows — completion, hover, and
/// diagnostics then degrade to the document-local sources.
async fn open_for<P: EnvProvider>(uri: &Uri, env: &P) -> Option<P::Opened> {
    let (repo, branch) = parse_repo_branch(uri)?;
    env.open(&repo, &branch).await
}

/// The repo-segment prefix naming the profile-as-repository,
/// mirroring the `profile:<name>` token in the `branch@repo`
/// location grammar the mounting element's `with` already speaks.
const PROFILE_PREFIX: &str = "profile:";

/// Pull `(repo, branch)` out of a
/// `tonk-buffer:///<encoded-repo>/<encoded-branch>/<cell-suffix>` URI — the shape
/// the editor's `<tonk-code source>` sets.
///
/// A `profile:<name>` repo segment names the profile-as-repository
/// rather than a repository called that; anything else is a named
/// repo. Returns `None` for any other shape.
fn parse_repo_branch(uri: &Uri) -> Option<(Repo, String)> {
    let rest = uri.as_str().strip_prefix("tonk-buffer:///")?;
    // First segment is the repo, second the branch.
    let mut parts = rest.splitn(3, '/');
    let repo_segment = parts.next()?;
    let branch = tonk_worker_api::decode_lsp_scope_segment(parts.next()?)?;
    let repo = match repo_segment.strip_prefix(PROFILE_PREFIX) {
        // `profile:` with no name is malformed, not the profile.
        Some("") => return None,
        Some(name) => Repo::Profile(tonk_worker_api::decode_lsp_scope_segment(name)?),
        None => Repo::Named(tonk_worker_api::decode_lsp_scope_segment(repo_segment)?),
    };
    Some((repo, branch))
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
    use async_trait::async_trait;
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_repository::Branch;
    use serde_json::json;
    use tonk_schema::concept::QueryEnv;
    use tonk_schema::query_source::Source;

    use super::*;
    use crate::env::NoEnv;

    /// Refcounted handle to a value that may not be `Send + Sync`
    /// on wasm. `Arc` everywhere except wasm32, where the test
    /// operator's storage isn't `Sync` and the runtime is
    /// single-threaded anyway.
    #[cfg(not(target_arch = "wasm32"))]
    type Rc<T> = std::sync::Arc<T>;
    #[cfg(target_arch = "wasm32")]
    type Rc<T> = std::rc::Rc<T>;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    async fn run(server: &mut Server, msg: &Value) -> Option<Value> {
        let bytes = serde_json::to_vec(msg).unwrap();
        server
            .handle_message(&bytes, &NoEnv)
            .await
            .map(|reply| serde_json::from_slice(&reply).unwrap())
    }

    /// Run a message against the server with a live host env
    /// backed by an empty test branch.
    async fn run_with<P: EnvProvider>(server: &mut Server, msg: &Value, env: &P) -> Option<Value> {
        let bytes = serde_json::to_vec(msg).unwrap();
        server
            .handle_message(&bytes, env)
            .await
            .map(|reply| serde_json::from_slice(&reply).unwrap())
    }

    /// A test [`EnvProvider`] backed by a real but empty test
    /// branch — enough for the analyzer pass to surface
    /// structural errors that don't depend on branch-published
    /// content. Generic over the operator type so we don't have
    /// to name `VolatileSpace` in scope.
    struct TestEnv<Op> {
        operator: Rc<Op>,
        branch: Branch,
    }

    /// The [`Opened`] handed back per request — a cheap clone of
    /// the underlying branch + a refcount of the operator.
    struct TestOpened<Op> {
        operator: Rc<Op>,
        branch: Branch,
    }

    impl<Op> Opened for TestOpened<Op>
    where
        Op: QueryEnv,
    {
        type Env = Op;
        fn source(&self) -> Source<'_> {
            Source::from(&self.branch)
        }
        fn env(&self) -> &Self::Env {
            &self.operator
        }
    }

    /// A `profile:<name>` repo segment names the profile-as-repository,
    /// not a repository spelled that way. The profile lives in its own
    /// namespace on the host, so flattening it to a named repo sent the
    /// host looking for one that does not exist — it opened no branch and
    /// completion on `/inspector` degraded to built-ins only.
    #[dialog_common::test]
    fn it_reads_a_profile_repo_segment_as_the_profile() {
        let uri: Uri = "tonk-buffer:///profile:team%2Ftonk/feat%2Fartifact/scratch-0"
            .parse()
            .expect("uri parses");
        assert_eq!(
            parse_repo_branch(&uri),
            Some((
                Repo::Profile("team/tonk".to_owned()),
                "feat/artifact".to_owned(),
            ))
        );
    }

    /// Everything without the prefix is an ordinary named repo. Reserved
    /// bytes in the DID and branch stay inside their canonical segments.
    #[dialog_common::test]
    fn it_reads_encoded_repo_and_slash_branch_segments() {
        let uri: Uri = "tonk-buffer:///did%3Akey%3AzAlice/feat%2Fartifact/scratch-0"
            .parse()
            .expect("uri parses");
        assert_eq!(
            parse_repo_branch(&uri),
            Some((
                Repo::Named("did:key:zAlice".to_owned()),
                "feat/artifact".to_owned(),
            ))
        );
    }

    #[dialog_common::test]
    fn it_rejects_non_canonical_repo_and_branch_aliases() {
        for raw in [
            "tonk-buffer:///did:key:zAlice/main/scratch-0",
            "tonk-buffer:///did%3akey%3azAlice/main/scratch-0",
            "tonk-buffer:///did%3Akey%3AzAlice/feat%2fartifact/scratch-0",
        ] {
            let uri: Uri = raw
                .parse()
                .expect("URI syntax parses before scope decoding");
            assert_eq!(parse_repo_branch(&uri), None, "accepted alias {raw}");
        }
    }

    /// `profile:` with no name is malformed, not a request for the
    /// profile — the seam carries the name, so refuse to invent one.
    #[dialog_common::test]
    fn it_rejects_a_profile_segment_without_a_name() {
        let uri: Uri = "tonk-buffer:///profile:/main/scratch-0"
            .parse()
            .expect("uri parses");
        assert_eq!(parse_repo_branch(&uri), None);
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl<Op> EnvProvider for TestEnv<Op>
    where
        Op: QueryEnv,
    {
        type Opened = TestOpened<Op>;
        async fn open(&self, _repo: &Repo, _branch: &str) -> Option<Self::Opened> {
            Some(TestOpened {
                operator: self.operator.clone(),
                branch: self.branch.clone(),
            })
        }
    }

    /// Build a `TestEnv` wrapping a fresh empty test branch.
    /// `impl Trait` in the return type hides the operator's space
    /// parameter.
    async fn new_test_env() -> TestEnv<impl QueryEnv> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo
            .branch("main")
            .open()
            .perform(&operator)
            .await
            .expect("test branch opens");
        TestEnv {
            operator: Rc::new(operator),
            branch,
        }
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
    /// span and route quickfixes by code. Exercises the live
    /// analyzer pass — the host opens an empty test branch, and
    /// the structural error fires before any branch lookup.
    #[dialog_common::test]
    async fn it_publishes_diagnostic_with_code_and_range_from_analyzer() {
        let mut server = Server::new();
        let env = new_test_env().await;
        let _ = run_with(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "capabilities": {} }
            }),
            &env,
        )
        .await;
        // `person!:` (empty body) — analyzer rejects with
        // E_ASSERTION_WITHOUT_FIELDS, range pinned to the head.
        let note = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "tonk-buffer:///repo/main/test",
                    "languageId": "carry-asserted",
                    "version": 1,
                    "text": "person!:\n"
                }
            }
        });
        run_with(&mut server, &note, &env).await;
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

    /// `is_premise_assert_position` recognises the cursor sitting
    /// in a premise's `assert:` value, whether or not a `-`
    /// sequence marker precedes it, and rejects body / head /
    /// past-the-value prefixes.
    #[test]
    fn it_detects_premise_assert_position() {
        assert!(is_premise_assert_position("    - assert: "));
        assert!(is_premise_assert_position("    - assert: math/"));
        assert!(is_premise_assert_position("      assert: math/sum"));
        // Not an assert value.
        assert!(!is_premise_assert_position("  count: "));
        assert!(!is_premise_assert_position("person"));
        // Past the value — a second `:` means we've moved on.
        assert!(!is_premise_assert_position("    - assert: ping\n  where:"));
    }

    /// In a premise's `assert:` value the completion list carries
    /// the built-in formulas (`math/sum`, …) and constraints (`==`)
    /// alongside concepts.
    #[dialog_common::test]
    async fn it_offers_formulas_in_premise_assert_position() {
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
        let text = "rule!:\n  assert!: counter\n  when:\n    - assert: ";
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
                        "text": text
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
                "position": { "line": 3, "character": 14 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or_default())
            .collect();
        for name in [
            "math/sum",
            "math/difference",
            "boolean/and",
            "text/concatenate",
            "==",
            // Range predicates and the `tree/*` resolvers share this
            // position: anything that can head a premise is offered
            // here, or it is undiscoverable in the editor.
            "<",
            "<=",
            ">",
            ">=",
            "starts-with",
            "tree/node",
            "tree/span",
            "tree/entry",
            "tree/key",
        ] {
            assert!(
                labels.contains(&name),
                "expected `{name}` in completion list, got {labels:?}",
            );
        }

        // `>` and `>=` must insert QUOTED: bare, they open a YAML
        // folded scalar, and the document then parses with an empty
        // predicate name.
        // The server turns `insert_text` into an explicit `text_edit`
        // so the client replaces the whole token (see `with_range`),
        // so the inserted text is the edit's `newText`.
        let insert_for = |name: &str| -> String {
            items
                .iter()
                .find(|i| i["label"].as_str() == Some(name))
                .and_then(|i| {
                    i["textEdit"]["newText"]
                        .as_str()
                        .or_else(|| i["insertText"].as_str())
                })
                .unwrap_or_default()
                .to_owned()
        };
        assert_eq!(insert_for(">"), "\">\"");
        assert_eq!(insert_for(">="), "\">=\"");
        assert_eq!(insert_for("<"), "<");
    }

    /// Names collide across namespaces — `tree/node` is a resolver,
    /// and nothing stops a branch publishing a concept spelled the
    /// same. In `assert:` position all four namespaces share one
    /// list, so every item must say which namespace it came from or
    /// the user cannot tell what they are accepting.
    #[dialog_common::test]
    async fn it_labels_the_namespace_of_each_premise_completion() {
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
        let text = "rule!:\n  assert!: counter\n  when:\n    - assert: ";
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
                        "text": text
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let reply = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": "tonk-buffer:///test" },
                    "position": { "line": 3, "character": 14 }
                }
            }),
        )
        .await
        .expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");

        let detail_for = |name: &str| -> String {
            items
                .iter()
                .find(|i| i["label"].as_str() == Some(name))
                .and_then(|i| i["detail"].as_str())
                .unwrap_or_default()
                .to_owned()
        };

        assert_eq!(detail_for("tree/node"), "resolver");
        assert_eq!(detail_for("math/sum"), "formula");
        assert_eq!(detail_for("=="), "constraint");
        assert_eq!(detail_for("concept"), "concept");
    }

    /// Inside a resolver premise's `where:`, the offered keys are the
    /// resolver's own operands — read off dialog's schema, carrying
    /// its descriptions, with the required input first. Without this
    /// the block completes to nothing, since a resolver has no
    /// concept descriptor to read fields from.
    #[dialog_common::test]
    async fn it_offers_resolver_operands_inside_a_premise_where_block() {
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
        let text =
            "rule!:\n  assert!: alert\n  when:\n    - assert: tree/node\n      where:\n        ";
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
                        "text": text
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        let reply = run(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": "tonk-buffer:///test" },
                    "position": { "line": 5, "character": 8 }
                }
            }),
        )
        .await
        .expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or_default())
            .collect();

        for operand in ["of", "kind", "size", "count"] {
            assert!(
                labels.contains(&operand),
                "expected `{operand}` among tree/node operands, got {labels:?}"
            );
        }

        let of = items
            .iter()
            .find(|i| i["label"].as_str() == Some("of"))
            .expect("`of` is offered");
        assert_eq!(
            of["detail"].as_str(),
            Some("required input"),
            "`of` must be marked as the term that has to be bound"
        );
        assert!(
            of["documentation"].as_str().is_some_and(|d| !d.is_empty()),
            "operand docs come off dialog's schema"
        );
    }

    /// A quoted constraint is not a resolver. `- assert: ">"` names
    /// the range predicate, so its `where:` must fall through to the
    /// ordinary path rather than being read as a resolver named `>`.
    #[dialog_common::test]
    fn it_does_not_read_a_quoted_constraint_as_a_resolver() {
        let text = "rule!:\n  when:\n    - assert: \">\"\n      where:\n        ";
        let position = lsp_types::Position {
            line: 4,
            character: 8,
        };
        assert_eq!(enclosing_resolver(text, position), None);

        let resolver = "rule!:\n  when:\n    - assert: tree/span\n      where:\n        ";
        assert_eq!(
            enclosing_resolver(resolver, position).as_deref(),
            Some("tree/span")
        );
    }

    /// Accepting a namespaced completion over a typed partial name
    /// must replace the *whole* token, namespace prefix included.
    /// Each item carries an explicit `textEdit` whose range starts
    /// before the `/` (at the token start), not after it — otherwise
    /// the client keeps the typed `math/` and produces
    /// `math/math/sum` (BUG-21).
    #[dialog_common::test]
    async fn it_replaces_the_namespace_prefix_on_namespaced_completions() {
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
        // Cursor sits just after the partial `math/su`.
        let text = "rule!:\n  assert!: counter\n  when:\n    - assert: math/su";
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
                        "text": text
                    }
                }
            }),
        )
        .await;
        let _ = server.take_outbound();
        // `    - assert: math/su` is 21 characters; the cursor is at
        // the end of the line.
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "tonk-buffer:///test" },
                "position": { "line": 3, "character": 21 }
            }
        });
        let reply = run(&mut server, &req).await.expect("response");
        let items = reply["result"]["items"].as_array().expect("items array");
        let sum = items
            .iter()
            .find(|i| i["label"] == "math/sum")
            .expect("math/sum offered");
        let edit = &sum["textEdit"];
        // The range starts at the `m` of `math/su` (column 14, right
        // after `    - assert: `), spanning the whole partial token,
        // and replaces it with the full namespaced name.
        assert_eq!(edit["range"]["start"]["character"], 14);
        assert_eq!(edit["range"]["end"]["character"], 21);
        assert_eq!(edit["newText"], "math/sum");
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
