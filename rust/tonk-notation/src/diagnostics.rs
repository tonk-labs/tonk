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

/// Run the parser against the document text and return its
/// diagnostics.
///
/// Today this is a thin wrapper around [`crate::parse::parse`] —
/// the parser itself emits structural diagnostics as part of
/// building the [`crate::syntax::Syntax`] tree. Future
/// validation passes (concept-schema validation, semantic
/// checks) compose here on top of `parsed.syntax` without
/// changing the entry point.
pub fn document_diagnostics(text: &str) -> Vec<Diagnostic> {
    parse::parse(text).diagnostics
}
