//! Phase-0 YAML well-formedness parser.
//!
//! The output of [`parse`] is a [`Parsed`] record carrying the
//! decoded `serde_yaml::Value` (when the document is well-formed)
//! and any diagnostics surfaced by the parser. Diagnostics are LSP-
//! shaped so they round-trip through the language server transport
//! without a separate conversion step.
//!
//! Future phases will layer asserted-notation rules on top of this:
//! the YAML well-formedness check stays here; the three-level shape
//! check, concept-schema validation, etc. each get their own module
//! and contribute additional diagnostics.

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use serde_yaml::Value;

/// Result of [`parse`]: the decoded YAML value (when the document
/// parses cleanly) plus any diagnostics produced along the way.
///
/// `value` and `diagnostics` are not mutually exclusive in general —
/// later phases may surface non-fatal diagnostics alongside a parsed
/// value (e.g. "unknown concept name" warnings). For phase 0 a single
/// fatal parse error is the only diagnostic shape we emit, so callers
/// will see either `Some(value)` with no diagnostics, or `None` with
/// exactly one error diagnostic.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    /// The decoded YAML value, when the document is well-formed.
    pub value: Option<Value>,
    /// Diagnostics produced during parsing.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse `text` as YAML and surface any parse error as a diagnostic.
///
/// `text` is the full document buffer. The empty document parses to
/// `Value::Null` with no diagnostics — that's a deliberate choice so
/// the editor doesn't squiggle a freshly-opened buffer.
pub fn parse(text: &str) -> Parsed {
    match serde_yaml::from_str::<Value>(text) {
        Ok(value) => Parsed {
            value: Some(value),
            diagnostics: Vec::new(),
        },
        Err(err) => Parsed {
            value: None,
            diagnostics: vec![diagnostic_for_yaml_error(&err)],
        },
    }
}

/// Convert a `serde_yaml::Error` into an LSP diagnostic. The crate's
/// `Location` reports a 1-indexed (line, column); LSP positions are
/// 0-indexed, so we subtract one. When location is unavailable (rare
/// — only some kinds of errors lack it) the diagnostic is anchored at
/// `0:0` rather than dropped, so the user still sees the message.
fn diagnostic_for_yaml_error(err: &serde_yaml::Error) -> Diagnostic {
    let position = match err.location() {
        // serde_yaml's columns are 1-indexed and counted in characters
        // rather than UTF-16 code units. For phase 0 we accept that
        // discrepancy; multi-byte characters in YAML keys/values are
        // rare in asserted notation. Revisit if the editor cursor
        // disagrees with diagnostic ranges in real use.
        Some(loc) => Position {
            line: loc.line().saturating_sub(1) as u32,
            character: loc.column().saturating_sub(1) as u32,
        },
        None => Position {
            line: 0,
            character: 0,
        },
    };

    Diagnostic {
        // Single-character span — the parser's location is a point,
        // not a region. Bumping `end.character` by 1 makes the
        // squiggle visible in the editor; without it the underline
        // collapses to a zero-width caret.
        range: Range {
            start: position,
            end: Position {
                line: position.line,
                character: position.character + 1,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("yaml".into()),
        message: format!("{err}"),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_is_clean() {
        let parsed = parse("");
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn well_formed_document_returns_value() {
        let parsed = parse("a: 1\nb: two\n");
        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.value.is_some());
    }

    #[test]
    fn parse_error_surfaces_as_diagnostic() {
        // Indentation inconsistency — serde_yaml rejects this.
        let parsed = parse("a:\n  b: 1\n c: 2\n");
        assert!(parsed.value.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        let diag = &parsed.diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diag.source.as_deref(), Some("yaml"));
    }
}
