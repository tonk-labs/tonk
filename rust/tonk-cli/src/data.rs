//! Notation builders for the argument-based data verbs. Each verb
//! collects (field, raw-value) pairs from clap, renders them into an
//! asserted-notation document per the field's schema type, and hands
//! the document to `eval::run_against_site` — so the verbs are a
//! constrained front-end over the same analyze→commit pipeline as
//! `tonk eval`, not a second write path.

use dialog_query::{ConceptDescriptor, Type};

/// Error rendering a raw CLI value or building a notation document
/// against a concept's descriptor.
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    /// A `--field` name isn't in the concept's `with:` map.
    #[error("unknown field '{field}' on {concept}; valid fields: {}", valid.join(", "))]
    UnknownField {
        /// Concept the field was looked up against.
        concept: String,
        /// The offending field name.
        field: String,
        /// Field names the descriptor does recognize.
        valid: Vec<String>,
    },
    /// A raw CLI value failed to parse as the field's declared type.
    #[error("value '{raw}' is not a valid {ty} for field '{field}'")]
    BadValue {
        /// Field the value was destined for.
        field: String,
        /// Declared type name, for the error message.
        ty: String,
        /// The offending raw value.
        raw: String,
    },
}

/// Render one raw CLI value into its notation form given the field's
/// declared type. Text is always quoted (a bare value that parses as a
/// symbol/bool would be misread); numerics/bools are bare literals
/// (validated); entities/symbols are emitted verbatim (a bare name or a
/// URI). An untyped field (`None`) is quoted as text.
pub fn render_value(ty: Option<Type>, raw: &str) -> Result<String, DataError> {
    let bad = |ty: &str| DataError::BadValue {
        field: String::new(),
        ty: ty.into(),
        raw: raw.into(),
    };
    match ty {
        Some(Type::UnsignedInt) => {
            raw.parse::<u128>().map_err(|_| bad("UnsignedInteger"))?;
            Ok(raw.to_string())
        }
        Some(Type::SignedInt) => {
            raw.parse::<i128>().map_err(|_| bad("SignedInteger"))?;
            // Spell the signedness: bare digits parse as unsigned, so
            // a non-negative signed value carries an explicit `+`.
            if raw.starts_with('+') || raw.starts_with('-') {
                Ok(raw.to_string())
            } else {
                Ok(format!("+{raw}"))
            }
        }
        Some(Type::Float) => {
            raw.parse::<f64>().map_err(|_| bad("Float"))?;
            Ok(raw.to_string())
        }
        Some(Type::Boolean) => {
            raw.parse::<bool>().map_err(|_| bad("Boolean"))?;
            Ok(raw.to_string())
        }
        Some(Type::Entity) | Some(Type::Symbol) => Ok(raw.to_string()),
        _ => Ok(quote_string(raw)), // String/Bytes/Record/None → quoted text
    }
}

/// Double-quote and escape a string for notation (mirrors the emitter
/// in `output.rs`/`schema.rs`; kept local to avoid widening their API).
fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn valid_fields(descriptor: &ConceptDescriptor) -> Vec<String> {
    descriptor
        .with()
        .iter()
        .map(|(f, _)| f.to_string())
        .collect()
}

fn render_pairs(
    descriptor: &ConceptDescriptor,
    concept: &str,
    fields: &[(String, String)],
) -> Result<Vec<String>, DataError> {
    let mut lines = Vec::with_capacity(fields.len());
    for (field, raw) in fields {
        let Some((_, fd)) = descriptor.with().iter().find(|(f, _)| f == field) else {
            return Err(DataError::UnknownField {
                concept: concept.to_string(),
                field: field.clone(),
                valid: valid_fields(descriptor),
            });
        };
        let value = render_value(fd.content_type(), raw).map_err(|e| match e {
            DataError::BadValue { ty, raw, .. } => DataError::BadValue {
                field: field.clone(),
                ty,
                raw,
            },
            other => other,
        })?;
        lines.push(format!("  {field}: {value}"));
    }
    Ok(lines)
}

/// Build a `<concept>!: { … }` assertion document from (field, raw)
/// pairs, resolving each field's type through `descriptor` — the
/// mint form of `tonk assert` (no entity).
pub fn build_assert(
    descriptor: &ConceptDescriptor,
    concept: &str,
    fields: &[(String, String)],
) -> Result<String, DataError> {
    let body = render_pairs(descriptor, concept, fields)?.join("\n");
    Ok(format!("{concept}!:\n{body}\n"))
}

/// Build a `<concept>!: { this: <entity>, … }` assertion document —
/// superseding claims against an existing entity (the entity form
/// of `tonk assert`).
pub fn build_supersede(
    descriptor: &ConceptDescriptor,
    concept: &str,
    entity: &str,
    fields: &[(String, String)],
) -> Result<String, DataError> {
    let body = render_pairs(descriptor, concept, fields)?.join("\n");
    Ok(format!("{concept}!:\n  this: {entity}\n{body}\n"))
}

/// Build a retraction document: a single field (`field: _`) or the
/// whole entity (`..: _`) when `field` is `None`. A retraction is
/// itself an assertion — a claim invalidating an old one — not a
/// deletion.
pub fn build_retract(concept: &str, entity: &str, field: Option<&str>) -> String {
    match field {
        Some(f) => format!("{concept}!:\n  this: {entity}\n  {f}: _\n"),
        None => format!("{concept}!:\n  this: {entity}\n  ..: _\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_query::Type;

    #[test]
    fn it_quotes_text_values() {
        assert_eq!(
            render_value(Some(Type::String), "hi there").unwrap(),
            "\"hi there\""
        );
    }
    #[test]
    fn it_renders_numeric_values_bare() {
        assert_eq!(render_value(Some(Type::UnsignedInt), "42").unwrap(), "42");
    }
    #[test]
    fn it_renders_boolean_bare() {
        assert_eq!(render_value(Some(Type::Boolean), "true").unwrap(), "true");
    }
    #[test]
    fn it_renders_entity_values_bare() {
        assert_eq!(render_value(Some(Type::Entity), "run").unwrap(), "run");
        assert_eq!(
            render_value(Some(Type::Entity), "did:key:z6Mk").unwrap(),
            "did:key:z6Mk"
        );
    }
    #[test]
    fn it_rejects_a_non_numeric_for_a_numeric_field() {
        assert!(render_value(Some(Type::UnsignedInt), "notanumber").is_err());
    }
    #[test]
    fn it_renders_a_u128_scale_unsigned_integer() {
        assert_eq!(
            render_value(
                Some(Type::UnsignedInt),
                "340282366920938463463374607431768211455"
            )
            .unwrap(),
            "340282366920938463463374607431768211455"
        );
    }
    #[test]
    fn it_escapes_control_characters_in_text() {
        assert_eq!(
            render_value(Some(Type::String), "a\tb\r\nc").unwrap(),
            "\"a\\tb\\r\\nc\""
        );
    }
    #[test]
    fn it_builds_a_field_retraction() {
        assert_eq!(
            build_retract("task", "t1", Some("done")),
            "task!:\n  this: t1\n  done: _\n"
        );
    }
    #[test]
    fn it_builds_a_whole_entity_retraction() {
        assert_eq!(
            build_retract("task", "t1", None),
            "task!:\n  this: t1\n  ..: _\n"
        );
    }
}
