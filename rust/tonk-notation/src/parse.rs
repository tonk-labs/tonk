//! YAML parser for asserted notation.
//!
//! [`parse`] takes a YAML document and walks it via [`saphyr`],
//! producing a [`Syntax`] tree plus a list of diagnostics. The
//! parser is permissive: a malformed expression yields a
//! diagnostic and the rest of the document still parses, so the
//! language server can underline several problems at once.
//!
//! The parser does **not** resolve names against a branch, derive
//! entity URIs, or know about the dialog meta-schema. Those
//! concerns live in [`tonk-schema`'s analyzer][analyze].
//!
//! [analyze]: https://github.com/dialog-db/tonk-workers/tree/main/rust/tonk-schema/src/interpret.rs

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use saphyr::{LoadableYamlNode, MarkedYaml, Scalar as SaphyrScalar, ScanError, YamlData};

use crate::syntax::{
    Assertion, Binding, Expression, Field, FieldValue, Head, HeadName, Query, Reference,
    Retraction, Scalar, Syntax,
};

/// Outcome of a parse: a [`Syntax`] tree (when the parser made it
/// through end-to-end) plus any diagnostics raised along the way.
///
/// `syntax.is_some()` does not imply `diagnostics.is_empty()` — a
/// partially-broken document yields the well-formed subset of
/// expressions so the language server can show every issue.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    /// The parsed [`Syntax`] when parsing reached the bottom.
    pub syntax: Option<Syntax>,
    /// Diagnostics raised during the parse.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse `text` as a YAML document and convert it to a [`Syntax`]
/// tree.
///
/// Empty input is a no-op success.
pub fn parse(text: &str) -> Parsed {
    let documents = match MarkedYaml::load_from_str(text) {
        Ok(documents) => documents,
        Err(err) => {
            return Parsed {
                syntax: None,
                diagnostics: vec![diagnostic_for_scan_error(&err)],
            };
        }
    };

    if documents.is_empty() {
        return Parsed::default();
    }

    let mut diagnostics = Vec::new();
    let mut expressions = Vec::new();
    let mut overall_range: Option<Range> = None;
    for doc in &documents {
        let doc_range = range_of(doc);
        overall_range = Some(match overall_range {
            None => doc_range,
            Some(existing) => extend_range(existing, doc_range),
        });
        walk_document(doc, &mut expressions, &mut diagnostics);
    }

    let range = overall_range.unwrap_or_default();
    Parsed {
        syntax: Some(Syntax { expressions, range }),
        diagnostics,
    }
}

// -----------------------------------------------------------------
// YAML walker (saphyr → Syntax)
// -----------------------------------------------------------------

fn walk_document(
    doc: &MarkedYaml<'_>,
    expressions: &mut Vec<Expression>,
    out: &mut Vec<Diagnostic>,
) {
    let YamlData::Mapping(top) = &doc.data else {
        out.push(error(
            range_of(doc),
            "Asserted notation expects a mapping at the document root \
             (head → body).",
        ));
        return;
    };
    for (head_key, body_value) in top {
        if let Some(expression) = walk_expression(head_key, body_value, out) {
            expressions.push(expression);
        }
    }
}

fn walk_expression(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<Expression> {
    let key_text = match string_of(key) {
        Some(s) => s,
        None => {
            out.push(error(
                range_of(key),
                "Head must be a string (a concept name like `person`, \
                 `person!`, or a claim domain like `xyz.tonk!`).",
            ));
            return None;
        }
    };

    let key_range = range_of(key);
    let head = parse_head(key_text, key_range, out)?;
    let block_range = extend_range(key_range, range_of(value));

    // Body: `_` (retraction), null/empty (no-fields query or
    // assertion), or a mapping of fields.
    if is_blank_scalar(value) {
        if !head.effect {
            out.push(error(
                range_of(value),
                "Bare `_` body is only valid on `head!:` (assertion or \
                 retraction). A query body cannot be `_`.",
            ));
            return None;
        }
        return Some(Expression::Retraction(Retraction {
            head,
            range: block_range,
        }));
    }

    let field_nodes = match &value.data {
        YamlData::Mapping(fields) => {
            let mut nodes = Vec::new();
            for (field_key, field_value) in fields {
                if let Some(field) = walk_field(field_key, field_value, out) {
                    nodes.push(field);
                }
            }
            nodes
        }
        // `head:` with no body — saphyr surfaces this as a null
        // scalar. Treat as an empty field list (a query for any
        // entity matching the head, or an effect-free assertion).
        YamlData::Value(SaphyrScalar::Null) => Vec::new(),
        _ => {
            out.push(error(
                range_of(value),
                "Body must be either a mapping of `field: value`, empty, \
                 or `_` (retraction).",
            ));
            return None;
        }
    };

    if head.effect {
        Some(Expression::Assertion(Assertion {
            head,
            fields: field_nodes,
            range: block_range,
        }))
    } else {
        Some(Expression::Query(Query {
            head,
            fields: field_nodes,
            range: block_range,
        }))
    }
}

/// Parse `"name"`, `"name!"`, `"name binding"`, `"name! binding"`
/// into a [`Head`]. Splits on the first ASCII whitespace; the
/// first token (with optional trailing `!`) is the name; the rest
/// is the binding.
fn parse_head(text: &str, key_range: Range, out: &mut Vec<Diagnostic>) -> Option<Head> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        out.push(error(key_range, "Head must not be empty."));
        return None;
    }

    let (name_token, binding_token) = match trimmed.find(char::is_whitespace) {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].trim_start()),
        None => (trimmed, ""),
    };

    let (name_str, effect) = if let Some(stripped) = name_token.strip_suffix('!') {
        (stripped, true)
    } else {
        (name_token, false)
    };

    if name_str.is_empty() {
        out.push(error(
            key_range,
            "Head name must not be empty (got `!` with no name).",
        ));
        return None;
    }

    // Spans for name vs binding inside the key. We only know the
    // key's overall range, not byte offsets within it, so we
    // attribute both to the same range. Editors will still
    // highlight the right line; precise sub-spans can land later.
    let name_range = key_range;
    let binding_range = key_range;

    let name = if name_str.contains('.') {
        HeadName::Claim(name_str.to_owned())
    } else {
        HeadName::Concept(name_str.to_owned())
    };

    let binding = parse_binding(binding_token);

    Some(Head {
        name,
        name_range,
        name_source: name_str.to_owned(),
        effect,
        binding,
        binding_range,
    })
}

fn parse_binding(text: &str) -> Binding {
    if text.is_empty() {
        Binding::Anonymous
    } else if let Some(rest) = text.strip_prefix('?') {
        Binding::Variable(rest.to_owned())
    } else if text.contains(':') {
        Binding::Uri(text.to_owned())
    } else {
        Binding::Bookmark(text.to_owned())
    }
}

fn walk_field(
    key: &MarkedYaml<'_>,
    value: &MarkedYaml<'_>,
    out: &mut Vec<Diagnostic>,
) -> Option<Field> {
    let Some(name) = string_of(key) else {
        out.push(error(range_of(key), "Field name must be a string."));
        return None;
    };
    let value_range = range_of(value);
    let field_value = walk_field_value(value, out)?;
    Some(Field {
        name: name.to_owned(),
        name_range: range_of(key),
        value: field_value,
        value_range,
    })
}

fn walk_field_value(value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) -> Option<FieldValue> {
    match &value.data {
        YamlData::Value(SaphyrScalar::String(s)) => Some(classify_string_value(s.as_ref())),
        YamlData::Representation(text, _, _) => Some(classify_string_value(text.as_ref())),
        YamlData::Value(scalar) => Some(FieldValue::Literal(scalar_from_saphyr(scalar))),
        YamlData::Mapping(map) => {
            let mut nested = Vec::new();
            for (k, v) in map {
                if let Some(field) = walk_field(k, v, out) {
                    nested.push(field);
                }
            }
            Some(FieldValue::Nested(nested))
        }
        YamlData::Sequence(_) => {
            out.push(error(
                range_of(value),
                "Sequence values are not supported in this notation. \
                 Use repeated assertions for cardinality-many writes.",
            ));
            None
        }
        YamlData::Tagged(_, inner) => walk_field_value(inner, out),
        YamlData::Alias(_) | YamlData::BadValue => {
            out.push(error(range_of(value), "Unsupported YAML node here."));
            None
        }
    }
}

/// Classify a string value into the appropriate [`FieldValue`].
///
/// - `_` → [`FieldValue::Blank`]
/// - `?name` → [`FieldValue::Variable`]
/// - `.name` (leading dot) → [`FieldValue::Reference`] (bookmark)
/// - any text containing `:` → [`FieldValue::Reference`] (URI)
/// - everything else (bare identifiers, sentences, etc.) →
///   [`FieldValue::Literal`] string
///
/// References require an explicit sigil (`.` for bookmarks, `:`
/// inside the value for URIs) so the parser never has to guess
/// whether a bare token is a reference or a literal. The leading
/// dot is unambiguous because no bare identifier begins with `.`
/// (the dot is reserved for reverse-domain claim heads, which
/// never appear in value position).
fn classify_string_value(s: &str) -> FieldValue {
    if s == "_" {
        FieldValue::Blank
    } else if let Some(rest) = s.strip_prefix('?') {
        FieldValue::Variable(rest.to_owned())
    } else if let Some(rest) = s.strip_prefix('.') {
        FieldValue::Reference(Reference::Bookmark(rest.to_owned()))
    } else if s.contains(':') {
        FieldValue::Reference(Reference::Uri(s.to_owned()))
    } else {
        FieldValue::Literal(Scalar::String(s.to_owned()))
    }
}

fn is_blank_scalar(value: &MarkedYaml<'_>) -> bool {
    matches!(string_of(value), Some("_"))
}

fn scalar_from_saphyr(scalar: &SaphyrScalar<'_>) -> Scalar {
    match scalar {
        SaphyrScalar::Null => Scalar::Null,
        SaphyrScalar::Boolean(b) => Scalar::Boolean(*b),
        SaphyrScalar::Integer(i) => Scalar::Integer(i128::from(*i)),
        SaphyrScalar::FloatingPoint(f) => Scalar::Float(**f),
        SaphyrScalar::String(s) => Scalar::String(s.as_ref().to_owned()),
    }
}

// -----------------------------------------------------------------
// Saphyr helpers
// -----------------------------------------------------------------

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

/// Convert a saphyr marker to an LSP position. Saphyr uses
/// 1-indexed lines and 0-indexed columns; LSP uses 0-indexed for
/// both.
pub(crate) fn position_at(marker: &saphyr::Marker) -> Position {
    Position {
        line: (marker.line() as u32).saturating_sub(1),
        character: marker.col() as u32,
    }
}

/// Convert a saphyr node's span to an LSP range. Zero-width spans
/// get widened to one column so editor squiggles render visibly.
pub(crate) fn range_of(node: &MarkedYaml<'_>) -> Range {
    let start = position_at(&node.span.start);
    let mut end = position_at(&node.span.end);
    if start.line == end.line && start.character == end.character {
        end.character = end.character.saturating_add(1);
    }
    Range { start, end }
}

fn extend_range(start: Range, end: Range) -> Range {
    Range {
        start: start.start,
        end: end.end,
    }
}

fn string_of<'a>(node: &'a MarkedYaml<'_>) -> Option<&'a str> {
    match &node.data {
        YamlData::Value(SaphyrScalar::String(s)) => Some(s.as_ref()),
        YamlData::Representation(text, _, _) => Some(text.as_ref()),
        _ => None,
    }
}

// -----------------------------------------------------------------
// Diagnostic helpers
// -----------------------------------------------------------------

fn error(range: Range, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("asserted-notation".into()),
        message: message.into(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn parse_clean(text: &str) -> Syntax {
        let parsed = parse(text);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:#?}",
            parsed.diagnostics
        );
        parsed.syntax.expect("syntax should be Some on clean parse")
    }

    #[dialog_common::test]
    fn empty_input_is_clean() {
        let parsed = parse("");
        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.syntax.is_none());
    }

    #[dialog_common::test]
    fn yaml_parse_error_surfaces() {
        let parsed = parse("a:\n  b: 1\n c: 2\n");
        assert!(parsed.syntax.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].source.as_deref(), Some("yaml"));
    }

    #[dialog_common::test]
    fn scalar_root_is_an_error() {
        let parsed = parse("just a string\n");
        assert_eq!(parsed.diagnostics.len(), 1);
        assert!(
            parsed.diagnostics[0]
                .message
                .contains("mapping at the document root")
        );
    }

    #[dialog_common::test]
    fn query_concept_no_binding() {
        let syntax = parse_clean("person:\n");
        assert_eq!(syntax.expressions.len(), 1);
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query, got {:?}", syntax.expressions[0]);
        };
        assert!(matches!(&q.head.name, HeadName::Concept(n) if n == "person"));
        assert!(matches!(q.head.binding, Binding::Anonymous));
        assert!(!q.head.effect);
        assert!(q.fields.is_empty());
    }

    #[dialog_common::test]
    fn query_concept_with_variable_and_literal() {
        let syntax = parse_clean(
            "person ?alice:\n\
             \x20 name: \"Alice\"\n\
             \x20 age: ?age\n",
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(&q.head.binding, Binding::Variable(v) if v == "alice"));
        assert_eq!(q.fields.len(), 2);
        assert!(matches!(
            &q.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "Alice"
        ));
        assert!(matches!(&q.fields[1].value, FieldValue::Variable(v) if v == "age"));
    }

    #[dialog_common::test]
    fn query_claim_is_classified_by_dot() {
        let syntax = parse_clean(
            "xyz.tonk ?tonkee:\n\
             \x20 name: \"Alice\"\n\
             \x20 role: ?role\n",
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(&q.head.name, HeadName::Claim(n) if n == "xyz.tonk"));
    }

    #[dialog_common::test]
    fn assertion_concept_anonymous() {
        let syntax = parse_clean(
            "person!:\n\
             \x20 name: \"Nick\"\n\
             \x20 address: \"Portland, OR\"\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.head.name, HeadName::Concept(n) if n == "person"));
        assert!(matches!(a.head.binding, Binding::Anonymous));
        assert!(a.head.effect);
    }

    #[dialog_common::test]
    fn assertion_with_uri_binding() {
        let syntax = parse_clean(
            "person! did:key:zNick:\n\
             \x20 name: Nicholas\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.head.binding, Binding::Uri(u) if u == "did:key:zNick"));
    }

    #[dialog_common::test]
    fn assertion_with_bookmark_binding() {
        let syntax = parse_clean(
            "person! nick:\n\
             \x20 name: \"Nick\"\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(matches!(&a.head.binding, Binding::Bookmark(b) if b == "nick"));
    }

    #[dialog_common::test]
    fn retraction_with_blank_body() {
        let syntax = parse_clean("person! ?nick: _\n");
        let Expression::Retraction(r) = &syntax.expressions[0] else {
            panic!("expected Retraction, got {:?}", syntax.expressions[0]);
        };
        assert!(matches!(&r.head.binding, Binding::Variable(v) if v == "nick"));
    }

    #[dialog_common::test]
    fn field_level_blank_is_assertion() {
        // `name: _` inside an assertion body is field-level
        // retraction, represented as Assertion with FieldValue::Blank.
        let syntax = parse_clean(
            "person! did:key:zNick:\n\
             \x20 name: _\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert_eq!(a.fields.len(), 1);
        assert!(matches!(a.fields[0].value, FieldValue::Blank));
    }

    #[dialog_common::test]
    fn query_body_with_blank_is_an_error() {
        let parsed = parse("person ?alice: _\n");
        assert!(!parsed.diagnostics.is_empty());
    }

    #[dialog_common::test]
    fn nested_map_is_preserved() {
        let syntax = parse_clean(
            "concept! person:\n\
             \x20 description: A person\n\
             \x20 with:\n\
             \x20   name: .person-name\n\
             \x20   age:  .person-age\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        let with_field = a.fields.iter().find(|f| f.name == "with").unwrap();
        let FieldValue::Nested(inner) = &with_field.value else {
            panic!("expected nested map");
        };
        assert_eq!(inner.len(), 2);
        assert!(matches!(
            &inner[0].value,
            FieldValue::Reference(Reference::Bookmark(b)) if b == "person-name"
        ));
    }

    #[dialog_common::test]
    fn join_across_expressions() {
        let syntax = parse_clean(
            "person ?alice:\n\
             \x20 name: \"Alice\"\n\
             xyz.tonk:\n\
             \x20 person: ?alice\n\
             \x20 role: ?role\n",
        );
        assert_eq!(syntax.expressions.len(), 2);
        let Expression::Query(q1) = &syntax.expressions[0] else {
            panic!("expected Query 1");
        };
        let Expression::Query(q2) = &syntax.expressions[1] else {
            panic!("expected Query 2");
        };
        assert!(matches!(&q1.head.binding, Binding::Variable(v) if v == "alice"));
        assert!(matches!(&q2.head.name, HeadName::Claim(n) if n == "xyz.tonk"));
        assert!(matches!(
            &q2.fields[0].value,
            FieldValue::Variable(v) if v == "alice"
        ));
    }

    #[dialog_common::test]
    fn assertion_with_quoted_literal_string() {
        let syntax = parse_clean(
            "person! nick:\n\
             \x20 address: \"Portland, OR\"\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        assert!(matches!(
            &a.fields[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "Portland, OR"
        ));
    }

    #[dialog_common::test]
    fn bare_identifier_is_a_literal_not_a_reference() {
        // A bare identifier on the right of `field:` is a literal
        // string. References require the leading-`.` sigil
        // (`name: .person-name`).
        let syntax = parse_clean(
            "concept! person:\n\
             \x20 with:\n\
             \x20   name: person-name\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        let FieldValue::Nested(inner) = &a.fields[0].value else {
            panic!("expected nested");
        };
        assert!(matches!(
            &inner[0].value,
            FieldValue::Literal(Scalar::String(s)) if s == "person-name"
        ));
    }

    #[dialog_common::test]
    fn dotted_value_is_a_bookmark_reference() {
        let syntax = parse_clean(
            "concept! person:\n\
             \x20 with:\n\
             \x20   name: .person-name\n",
        );
        let Expression::Assertion(a) = &syntax.expressions[0] else {
            panic!("expected Assertion");
        };
        let FieldValue::Nested(inner) = &a.fields[0].value else {
            panic!("expected nested");
        };
        assert!(matches!(
            &inner[0].value,
            FieldValue::Reference(Reference::Bookmark(b)) if b == "person-name"
        ));
    }

    #[dialog_common::test]
    fn empty_head_after_bang_is_an_error() {
        let parsed = parse("!:\n  x: 1\n");
        assert!(!parsed.diagnostics.is_empty());
    }

    #[dialog_common::test]
    fn integer_literal_field_value() {
        let syntax = parse_clean(
            "person ?alice:\n\
             \x20 age: 28\n",
        );
        let Expression::Query(q) = &syntax.expressions[0] else {
            panic!("expected Query");
        };
        assert!(matches!(
            &q.fields[0].value,
            FieldValue::Literal(Scalar::Integer(28))
        ));
    }

    #[dialog_common::test]
    fn sequence_value_is_an_error() {
        let parsed = parse(
            "person! nick:\n\
             \x20 name:\n\
             \x20   - Alice\n\
             \x20   - Bob\n",
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(
            parsed.diagnostics[0]
                .message
                .to_lowercase()
                .contains("sequence")
        );
    }
}
