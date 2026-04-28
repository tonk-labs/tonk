//! Parser entry point.
//!
//! This module owns the YAML well-formedness check and the token-
//! tree representation that downstream validators (the three-level
//! shape check, concept-schema validation, …) traverse. We use
//! [`saphyr`] rather than `serde_yaml` because we need source
//! positions on every node so that diagnostics can be anchored to
//! a precise range — `serde_yaml`'s `Value` is span-less. Saphyr's
//! `MarkedYaml` carries a `Span { start, end }` on every node and
//! its parse errors expose a `Marker { line, col }` directly,
//! which we lift into LSP `Diagnostic`s with no information loss.
//!
//! Saphyr nodes borrow from the input buffer (lifetime `'a`), so
//! callers that want to keep the parsed tree around must keep
//! `text` alive too. [`document_diagnostics`] (in `diagnostics.rs`)
//! does the parsing and validation in a single call so the borrow
//! is internal and consumers only see owned [`Diagnostic`] values.

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use saphyr::{LoadableYamlNode, MarkedYaml, ScanError};

/// Outcome of [`parse`]: the parsed YAML documents (when parsing
/// succeeded) plus any parse-error diagnostic.
///
/// `documents` borrows from the input slice. The return type
/// keeps the borrow explicit so the lifetime is visible at every
/// call site — without it, callers would silently leak references
/// into static data.
pub struct Parsed<'a> {
    /// One entry per `---`-separated YAML document. Empty when
    /// parsing failed; the diagnostic explains why.
    pub documents: Vec<MarkedYaml<'a>>,
    /// Diagnostics produced while scanning. The well-formedness
    /// pass emits at most one (saphyr stops at the first scan
    /// error); other passes may layer additional diagnostics in.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse `text` as a YAML stream and surface any parse error as
/// an LSP diagnostic. The empty document parses to an empty
/// `documents` vec with no diagnostics — that's intentional, so a
/// freshly-opened editor doesn't squiggle.
pub fn parse(text: &str) -> Parsed<'_> {
    match MarkedYaml::load_from_str(text) {
        Ok(documents) => Parsed {
            documents,
            diagnostics: Vec::new(),
        },
        Err(err) => Parsed {
            documents: Vec::new(),
            diagnostics: vec![diagnostic_for_scan_error(&err)],
        },
    }
}

/// Convert saphyr's `ScanError` into an LSP diagnostic.
///
/// Saphyr's `Marker { line, col }` is **1-indexed line, 0-indexed
/// column**. LSP positions are 0-indexed on both axes, so the
/// line gets one subtracted and the column passes through. The
/// scanner's marker is a single point; we extend the diagnostic
/// to a 1-character span so the editor renders a visible squiggle
/// (a zero-width range collapses to a caret).
fn diagnostic_for_scan_error(err: &ScanError) -> Diagnostic {
    let marker = err.marker();
    let line = (marker.line() as u32).saturating_sub(1);
    let column = marker.col() as u32;
    let start = Position {
        line,
        character: column,
    };
    let end = Position {
        line,
        character: column + 1,
    };
    Diagnostic {
        range: Range { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("yaml".into()),
        message: err.info().to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Convert a saphyr node's start span into an LSP `Position`.
///
/// Exposed at the module level so the validators in
/// `shape.rs` can reuse it without re-deriving the
/// 1-indexed-line / 0-indexed-column convention.
pub(crate) fn position_at(marker: &saphyr::Marker) -> Position {
    Position {
        line: (marker.line() as u32).saturating_sub(1),
        character: marker.col() as u32,
    }
}

/// Convert a saphyr node's span into an LSP `Range`. When start == end
/// (a zero-width span at, e.g., the start of an empty document),
/// extend the end by one character so the resulting squiggle
/// has visible width in the editor.
///
/// We take a `&MarkedYaml` rather than a `&Span` because the
/// `Span` type isn't re-exported at the `saphyr` crate root —
/// pulling it in would mean adding `saphyr-parser` as a direct
/// dep just for one type name.
pub(crate) fn range_of(node: &MarkedYaml<'_>) -> Range {
    let start = position_at(&node.span.start);
    let mut end = position_at(&node.span.end);
    if start.line == end.line && start.character == end.character {
        end.character = end.character.saturating_add(1);
    }
    Range { start, end }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn empty_document_is_clean() {
        let parsed = parse("");
        assert!(parsed.diagnostics.is_empty());
    }

    #[dialog_common::test]
    fn well_formed_document_returns_a_tree() {
        let parsed = parse("a: 1\nb: two\n");
        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.documents.len(), 1);
    }

    #[dialog_common::test]
    fn parse_error_surfaces_with_real_position() {
        let parsed = parse("a:\n  b: 1\n c: 2\n");
        assert!(parsed.documents.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        let diag = &parsed.diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diag.source.as_deref(), Some("yaml"));
        // The error is on line 3 (0-indexed: 2). The exact column
        // varies between YAML parser implementations — we just
        // assert the line is right.
        assert_eq!(diag.range.start.line, 2);
    }
}
