//! Structural pre-pass over the syntax tree — independent of
//! resolver, branch state, or the rest of the analyzer.
//!
//! Three checks. An anchor declared under a built-in anchor's name, a
//! type's (`&text`) or a pick's (`&all`), shadows the built-in within
//! the document, which is warned about. An attribute body written the
//! way an earlier release spelled it (`cardinality:`, or a bare or
//! quoted word where a keyword belongs) is read as it is, and warned
//! about so the document moves to the current spelling. And
//! *single-occurrence variables*: a `?var`
//! that appears exactly once in the document is almost always a
//! mistake, because variables exist to create joins and a one-shot
//! variable binds nothing useful. Context-aware diagnostics:
//!
//! - Query body, non-`this:` field → **warning** (suggest `_`)
//! - Query body, `this:` slot → **warning** (suggest `_`)
//! - Assertion body, `this:` slot → **warning** (suggest omitting
//!   `this:` to derive a fresh entity, or query first)
//! - Assertion body, non-`this:` field → **error** (no value to
//!   write — committing a logic variable as a fact is meaningless)
//!
//! Runs entirely on the parsed [`Syntax`] — no resolver calls — so
//! it surfaces even when the rest of the analyzer would short-
//! circuit (e.g. `UnknownConcept` before per-field walks).

use std::collections::HashMap;

use lsp_types::Range;
use tonk_notation::{Expression, Field, FieldValue, HeadName, Scalar, Syntax};

use super::error::{AnalyzeDiagnostic, AnalyzeDiagnosticKind};

/// Where in the document a `?var` was used. Drives the
/// context-aware diagnostic shape — query vs assertion, `this:`
/// slot vs everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    QueryThis,
    QueryField,
    AssertionThis,
    AssertionField,
}

#[derive(Debug, Clone)]
struct Occurrence {
    position: Position,
    /// Field name (for non-`this:` positions). `None` for the
    /// `this:` slot.
    field: Option<String>,
    /// Source range of the `?var` token.
    range: Range,
}

/// Scan the document for single-occurrence variables and return
/// a diagnostic for each. Pure function of the syntax tree —
/// no resolver work, no branch lookup. Safe to call from the
/// LSP independently of [`crate::analyzer::analyze`].
pub fn scan_variables(syntax: &Syntax) -> Vec<AnalyzeDiagnostic> {
    let mut occurrences: HashMap<String, Vec<Occurrence>> = HashMap::new();
    for expression in &syntax.expressions {
        // `rule!:` claims have their own scoping (each premise's
        // `where:` binds variables) and don't share the top-level
        // query/assertion variable namespace, so single-occurrence
        // scanning treats them like queries for the head-bindings
        // but skips the `when:`/`unless:` premise bodies (those
        // variables are joined by the analyzer's rule lift, not by
        // top-level position tracking).
        match expression {
            Expression::Query(q) => collect_from_fields(
                &q.fields,
                Position::QueryThis,
                Position::QueryField,
                &mut occurrences,
            ),
            Expression::Claim(c) => collect_from_fields(
                &c.inner.fields,
                Position::AssertionThis,
                Position::AssertionField,
                &mut occurrences,
            ),
        }
    }

    let mut out = Vec::new();
    for (name, uses) in occurrences {
        if uses.len() != 1 {
            continue;
        }
        let occ = &uses[0];
        let kind = match occ.position {
            Position::QueryField => AnalyzeDiagnosticKind::SingleOccurrenceVariableQueryField {
                name: name.clone(),
                field: occ.field.clone().unwrap_or_default(),
            },
            Position::QueryThis => {
                AnalyzeDiagnosticKind::SingleOccurrenceVariableQueryThis { name: name.clone() }
            }
            Position::AssertionThis => {
                AnalyzeDiagnosticKind::SingleOccurrenceVariableAssertionThis { name: name.clone() }
            }
            Position::AssertionField => {
                AnalyzeDiagnosticKind::SingleOccurrenceVariableAssertionField {
                    name: name.clone(),
                    field: occ.field.clone().unwrap_or_default(),
                }
            }
        };
        let diagnostic = match occ.position {
            // Assertion-field is the only error — committing a
            // logic variable as a fact has no useful semantics.
            Position::AssertionField => AnalyzeDiagnostic::error(kind, occ.range),
            _ => AnalyzeDiagnostic::warning(kind, occ.range),
        };
        out.push(diagnostic);
    }
    // An anchor named like a built-in anchor, a type or a pick,
    // shadows it within the document.
    for expression in &syntax.expressions {
        if let Expression::Claim(claim) = expression
            && let Some(anchor) = &claim.anchor
            && tonk_notation::builtin(&anchor.name).is_some()
        {
            out.push(AnalyzeDiagnostic::warning(
                AnalyzeDiagnosticKind::ShadowsBuiltin {
                    name: anchor.name.clone(),
                },
                anchor.range,
            ));
        }
    }
    for expression in &syntax.expressions {
        if let Expression::Claim(claim) = expression {
            match &claim.inner.predicate.name {
                HeadName::Concept(name) if name == "attribute" => {
                    older_attribute_spellings(&claim.inner.fields, &mut out);
                }
                HeadName::Concept(name) if name == "concept" => {
                    for block in &claim.inner.fields {
                        if (block.name == "with" || block.name == "maybe")
                            && let FieldValue::Nested(fields) = &block.value
                        {
                            for field in fields {
                                if let FieldValue::Nested(body) = &field.value {
                                    older_attribute_spellings(body, &mut out);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Sort by range so output is deterministic for tests.
    out.sort_by_key(|d| d.range.map(|r| (r.start.line, r.start.character)));
    out
}

/// The word a keyword-valued field writes, in its spelling as written
/// and as the keyword it reads as, when it is written the older way: the
/// bare word or the quoted word. `None` for the keyword spelling and for
/// anything else, which the analyzer judges.
fn older_keyword(field: &Field) -> Option<(String, String)> {
    match &field.value {
        FieldValue::Symbol(word) => Some((word.clone(), word.clone())),
        FieldValue::Literal(Scalar::String(word)) => Some((format!("{word:?}"), word.clone())),
        _ => None,
    }
}

/// Warn about the spellings an earlier release used in an attribute
/// body: `cardinality:` in any spelling, and a bare or quoted word
/// under `pick:` or `role:`. The analyzer reads them; this only says
/// what to write instead.
fn older_attribute_spellings(fields: &[Field], out: &mut Vec<AnalyzeDiagnostic>) {
    for field in fields {
        match field.name.as_str() {
            "cardinality" => {
                let (value, word) = match &field.value {
                    FieldValue::Uri(uri) => {
                        let word = uri.strip_suffix(':').unwrap_or(uri);
                        (format!(":{word}"), word.to_owned())
                    }
                    _ => match older_keyword(field) {
                        Some(spelled) => spelled,
                        None => continue,
                    },
                };
                let advice = match word.as_str() {
                    "many" | "Many" => "write `pick: :all`".to_owned(),
                    "one" | "One" => {
                        "write `pick: :last` (the default, so the field can also go)".to_owned()
                    }
                    _ => "a field's pick is `pick: :last` (the default) or `pick: :all`".to_owned(),
                };
                out.push(AnalyzeDiagnostic::warning(
                    AnalyzeDiagnosticKind::DeprecatedCardinality { value, advice },
                    field.value_range,
                ));
            }
            "pick" | "role" => {
                if let Some((word, keyword)) = older_keyword(field) {
                    out.push(AnalyzeDiagnostic::warning(
                        AnalyzeDiagnosticKind::OlderKeywordSpelling {
                            field: field.name.clone(),
                            word,
                            keyword,
                        },
                        field.value_range,
                    ));
                }
            }
            _ => {}
        }
    }
}

fn collect_from_fields(
    fields: &[Field],
    this_position: Position,
    field_position: Position,
    occurrences: &mut HashMap<String, Vec<Occurrence>>,
) {
    for field in fields {
        let position = if field.name == "this" {
            this_position
        } else {
            field_position
        };
        collect_from_value(
            &field.value,
            field.value_range,
            position,
            field.name.clone(),
            occurrences,
        );
    }
}

fn collect_from_value(
    value: &FieldValue,
    range: Range,
    position: Position,
    field_name: String,
    occurrences: &mut HashMap<String, Vec<Occurrence>>,
) {
    match value {
        FieldValue::Variable(name) => {
            occurrences
                .entry(name.clone())
                .or_default()
                .push(Occurrence {
                    position,
                    field: if matches!(position, Position::QueryThis | Position::AssertionThis) {
                        None
                    } else {
                        Some(field_name)
                    },
                    range,
                });
        }
        FieldValue::Nested(inner) => {
            // Recurse into nested mappings using the same
            // position context so a `?var` inside `concept!`'s
            // `with:` is still tracked. Nested-field range
            // resolution uses each inner field's own range
            // through the recursive call.
            collect_from_fields(inner, position, position, occurrences);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::DiagnosticSeverity;
    use tonk_notation::parse;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn must_parse(src: &str) -> tonk_notation::Syntax {
        let parsed = parse(src);
        assert!(
            parsed.diagnostics.is_empty(),
            "parser failed: {:?}",
            parsed.diagnostics
        );
        parsed.syntax.expect("parser produced no syntax")
    }

    #[dialog_common::test]
    fn it_warns_on_single_use_query_field_variable() {
        // `?name` appears once, in a non-`this:` field of a
        // query — suggest `_`.
        let syntax = must_parse(
            r#"
person:
  this: ?p
  name: ?name
"#,
        );
        let diagnostics = scan_variables(&syntax);
        // `?p` is the `this:` slot of a query, also single
        // occurrence — also a warning. `?name` is the field
        // case we're checking here.
        let codes: Vec<&'static str> = diagnostics.iter().map(|d| d.code()).collect();
        assert!(codes.contains(&"W_SINGLE_OCCURRENCE_VARIABLE_QUERY_FIELD"));
        for diag in &diagnostics {
            assert_eq!(diag.severity, DiagnosticSeverity::Warning);
        }
    }

    #[dialog_common::test]
    fn it_warns_on_single_use_query_this_variable() {
        let syntax = must_parse(
            r#"
person:
  this: ?p
  name: "Alice"
"#,
        );
        let diagnostics = scan_variables(&syntax);
        let kinds: Vec<&'static str> = diagnostics.iter().map(|d| d.code()).collect();
        assert_eq!(kinds, vec!["W_SINGLE_OCCURRENCE_VARIABLE_QUERY_THIS"]);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
    }

    #[dialog_common::test]
    fn it_warns_on_single_use_assertion_this_variable() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  age: 29
"#,
        );
        let diagnostics = scan_variables(&syntax);
        let codes: Vec<&'static str> = diagnostics.iter().map(|d| d.code()).collect();
        assert!(codes.contains(&"W_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_THIS"));
        let this_diag = diagnostics
            .iter()
            .find(|d| d.code() == "W_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_THIS")
            .unwrap();
        assert_eq!(this_diag.severity, DiagnosticSeverity::Warning);
    }

    #[dialog_common::test]
    fn it_errors_on_single_use_assertion_field_variable() {
        // `?value` in an assertion field with no other use is an
        // error — there's no value to write.
        let syntax = must_parse(
            r#"
person!:
  this: did:key:zMkAlice
  name: ?value
"#,
        );
        let diagnostics = scan_variables(&syntax);
        let field_diag = diagnostics
            .iter()
            .find(|d| d.code() == "E_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_FIELD")
            .expect("expected E_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_FIELD");
        assert_eq!(field_diag.severity, DiagnosticSeverity::Error);
    }

    #[dialog_common::test]
    fn it_does_not_warn_when_variable_used_twice() {
        // `?p` is bound by the query and consumed by the
        // assertion — two uses, no warning.
        let syntax = must_parse(
            r#"
person:
  this: ?p
  name: ?n

person!:
  this: ?p
  name: ?n
"#,
        );
        let diagnostics = scan_variables(&syntax);
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics for variables used twice, got {diagnostics:?}"
        );
    }

    #[dialog_common::test]
    fn it_attaches_range_to_variable_occurrence() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  age: 29
"#,
        );
        let diagnostics = scan_variables(&syntax);
        let this_diag = diagnostics
            .iter()
            .find(|d| d.code() == "W_SINGLE_OCCURRENCE_VARIABLE_ASSERTION_THIS")
            .unwrap();
        let range = this_diag
            .range
            .expect("scan must produce ranged diagnostics");
        // The `?alice` token is on line 2 (0-indexed) given the
        // leading newline, after `this: `.
        assert_eq!(range.start.line, 2);
    }

    /// `cardinality:` is read, in every spelling, and warned about with
    /// the pick to write; the keyword spelling of a pick is not.
    #[dialog_common::test]
    fn it_warns_about_a_deprecated_cardinality_in_every_spelling() {
        for (spelling, advice) in [
            ("one", "write `pick: :last`"),
            (":one", "write `pick: :last`"),
            ("\"one\"", "write `pick: :last`"),
            ("many", "write `pick: :all`"),
            (":many", "write `pick: :all`"),
        ] {
            let syntax = must_parse(&format!(
                "attribute!: &tags\n  description: \"labels\"\n  the: x.y/tags\n  as: text\n  cardinality: {spelling}\n"
            ));
            let diagnostics = scan_variables(&syntax);
            let found = diagnostics
                .iter()
                .find(|d| d.code() == "W_DEPRECATED_CARDINALITY")
                .unwrap_or_else(|| panic!("cardinality: {spelling} is warned about"));
            assert_eq!(found.severity, DiagnosticSeverity::Warning);
            assert!(
                found.message().contains(advice),
                "cardinality: {spelling} says what to write: {}",
                found.message()
            );
        }
        let syntax = must_parse(
            "attribute!: &tags\n  description: \"labels\"\n  the: x.y/tags\n  as: text\n  pick: :all\n",
        );
        assert!(
            scan_variables(&syntax).is_empty(),
            "the keyword spelling of a pick is the current one"
        );
    }

    /// A bare or quoted word under `pick:` or `role:` is warned about
    /// with the keyword, in an `attribute!` body and in a concept's
    /// inline field alike; a word in an ordinary fact's field is not a
    /// keyword and is left alone.
    #[dialog_common::test]
    fn it_warns_about_the_older_spelling_of_a_keyword() {
        let syntax = must_parse(
            r#"
attribute!: &tags
  description: "labels"
  the: x.y/tags
  as: text
  pick: all

concept!: &person
  description: "A person"
  with:
    name:
      description: "The name"
      the: x.y/name
      as: text
      role: "object"

person!:
  name: "Alice"
  role: object
"#,
        );
        let diagnostics: Vec<_> = scan_variables(&syntax)
            .into_iter()
            .filter(|d| d.code() == "W_OLDER_KEYWORD_SPELLING")
            .collect();
        let messages: Vec<String> = diagnostics.iter().map(|d| d.message()).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].contains("`pick: :all`"), "{messages:?}");
        assert!(messages[1].contains("`role: :object`"), "{messages:?}");
        for diagnostic in &diagnostics {
            assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
        }
    }
}
