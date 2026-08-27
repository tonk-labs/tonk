//! Reading a `<ui-element>`'s inline concept into a descriptor.
//!
//! Split from the element itself because this half is pure text — no DOM, no
//! subscription — so it builds and tests on every target, while the element
//! is wasm-only.

use serde_json::{Value, json};
use tonk_notation::{Expression, Field, FieldValue, Scalar};

/// Parse the inline `concept!:` block into the descriptor JSON the query
/// builders take — its `the` / `with` / `maybe` map.
///
/// Parsed with [`tonk_notation`], the same parser the editor and the worker
/// use, so a concept written here is the same language written anywhere else
/// — including its diagnostics, which is why malformed source is rejected
/// rather than half-read.
///
/// This is NOT an analysis pass: the guest deliberately does not link the
/// analyzer, and it does not need to. A supplied concept is already the
/// descriptor; there is no name to resolve and no branch to resolve against.
pub fn parse_concept(source: &str) -> Option<String> {
    let parsed = tonk_notation::parse(source);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    // The block is written under a head, the way a concept is written
    // anywhere else. `concept!:` is a claim; a plain `concept:` head parses
    // as a query. Both carry the same fields, so accept either rather than
    // making the `!` load-bearing for something that is never committed.
    //
    // A headless mapping (just `with:` at the top level) is NOT accepted:
    // notation requires a head, so it does not parse, and inventing a
    // fallback here would mean this element read a dialect nothing else does.
    let expression = parsed.syntax?.expressions.into_iter().next()?;
    let fields = match expression {
        Expression::Claim(claim) => claim.inner.fields,
        Expression::Query(application) => application.fields,
    };

    let descriptor = fields_to_json(&fields);
    // A descriptor without fields projects nothing, so the template could
    // never bind anything — surface it as a parse failure rather than
    // silently rendering an empty frame forever.
    descriptor.get("with")?;
    serde_json::to_string(&descriptor).ok()
}

/// Convert a parsed field list to the descriptor's JSON object.
fn fields_to_json(fields: &[Field]) -> Value {
    let mut out = serde_json::Map::new();
    for field in fields {
        if let Some(value) = field_value_to_json(&field.value) {
            out.insert(field.name.clone(), value);
        }
    }
    Value::Object(out)
}

/// Convert one field value. Variables, blanks and premise lists have no place
/// in a descriptor, so they are dropped rather than guessed at.
fn field_value_to_json(value: &FieldValue) -> Option<Value> {
    Some(match value {
        FieldValue::Nested(fields) => fields_to_json(fields),
        // A descriptor's leaves are attribute names (`xyz.tonk.sync/state`)
        // and type words (`entity`, `text`), which the parser classifies as
        // URIs and symbols respectively. Both are plain strings here.
        FieldValue::Uri(text) | FieldValue::Symbol(text) => json!(text),
        FieldValue::Literal(scalar) => match scalar {
            Scalar::String(text) => json!(text),
            Scalar::Integer(number) => json!(number),
            Scalar::UnsignedInteger(number) => json!(number),
            Scalar::Float(number) => json!(number),
            Scalar::Boolean(flag) => json!(flag),
            Scalar::Null => Value::Null,
        },
        FieldValue::Variable(_) | FieldValue::Blank | FieldValue::Premises(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The authoring shape: a `concept!:` head over a `with:` map.
    #[dialog_common::test]
    fn it_parses_a_concept_block_into_a_descriptor() {
        let descriptor = parse_concept(
            r#"
concept!:
  with:
    state:
      the: xyz.tonk.sync/state
      as: entity
"#,
        )
        .expect("parses");
        let value: Value = serde_json::from_str(&descriptor).unwrap();
        assert_eq!(value["with"]["state"]["the"], "xyz.tonk.sync/state");
        assert_eq!(value["with"]["state"]["as"], "entity");
    }

    /// A `concept:` query head carries the same fields as `concept!:`, and
    /// nothing here is ever committed, so the `!` is not load-bearing.
    #[dialog_common::test]
    fn it_accepts_a_query_head_as_well_as_a_claim() {
        let descriptor = parse_concept(
            r#"
concept:
  with:
    name:
      the: xyz.tonk.space/name
      as: text
"#,
        )
        .expect("parses");
        let value: Value = serde_json::from_str(&descriptor).unwrap();
        assert_eq!(value["with"]["name"]["as"], "text");
    }

    /// Notation requires a head, so a bare mapping does not parse. Rejected
    /// rather than special-cased: a fallback here would be a dialect only
    /// this element reads.
    #[dialog_common::test]
    fn it_rejects_a_headless_mapping() {
        assert!(parse_concept("with:\n  name:\n    as: text\n").is_none());
    }

    /// A pinned `the:` rides along — the descriptor is passed through whole.
    #[dialog_common::test]
    fn it_keeps_a_pinned_concept_entity() {
        let descriptor = parse_concept(
            r#"
concept!:
  the: tonk:sync
  with:
    state:
      the: xyz.tonk.sync/state
      as: entity
"#,
        )
        .expect("parses");
        let value: Value = serde_json::from_str(&descriptor).unwrap();
        assert_eq!(value["the"], "tonk:sync");
    }

    /// No fields means nothing to bind, which is a mistake worth reporting
    /// rather than rendering an empty frame forever.
    #[dialog_common::test]
    fn it_rejects_a_concept_with_no_fields() {
        assert!(parse_concept("concept!:\n  the: tonk:sync\n").is_none());
    }

    #[dialog_common::test]
    fn it_rejects_unparseable_source() {
        assert!(parse_concept("this: [is: not: yaml").is_none());
    }
}
