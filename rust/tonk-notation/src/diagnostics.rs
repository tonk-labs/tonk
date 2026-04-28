//! Top-level diagnostic entry point and language-server metadata.
//!
//! Today this is a thin wrapper around [`crate::parse::parse`].
//! The function exists so additional validation passes
//! (well-formedness, three-level shape, concept-schema, semantic
//! checks) can compose behind one stable entry point — callers
//! won't have to thread new passes into their dispatch code.

use lsp_types::Diagnostic;

use crate::parse;

/// LSP `languageId` we register documents under. Editors set this on
/// `textDocument/didOpen` so the server can route to the right
/// validator when it eventually hosts more than one language.
pub const NOTATION_LANGUAGE_ID: &str = "carry-asserted";

/// Server-name / version pair returned in `initialize`'s `serverInfo`.
/// Hard-coded here so all consumers (the in-SW dispatcher and a future
/// CLI lint mode) advertise the same identity to clients.
pub const SERVER_INFO: ServerInfo = ServerInfo {
    name: "tonk-notation",
    version: env!("CARGO_PKG_VERSION"),
};

/// LSP `serverInfo` structure, lifted out so it isn't allocated per
/// `initialize` request.
#[derive(Debug, Clone, Copy)]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// Run all validators against the document text and return a merged
/// diagnostic list.
///
/// Composes:
/// 1. YAML well-formedness ([`crate::parse`]). When parsing fails
///    the documents vec is empty and the parse error is the only
///    thing surfaced; the structural check is skipped because it
///    has nothing to walk.
/// 2. Three-level shape check ([`crate::shape`]). Verifies
///    entity → context → fields and the reserved-domain rule
///    against the parsed tree.
///
/// Concept-schema and semantic checks compose here without
/// changing this entry point.
pub fn document_diagnostics(text: &str) -> Vec<Diagnostic> {
    let parsed = parse::parse(text);
    let mut diagnostics = parsed.diagnostics;
    if diagnostics.is_empty() {
        // Only run the shape pass when parsing produced a tree.
        // Walking after a parse error would just add noise on top
        // of the real issue.
        diagnostics.extend(crate::shape::validate(&parsed.documents));
    }
    diagnostics
}
