//! Three-level shape validator.
//!
//! Validates that a parsed YAML document follows asserted notation's
//! entity → context → fields hierarchy as documented in the carry
//! RFC (`rfc/carry.md`, "Asserted Notation"). The check is purely
//! structural — concept-schema validation (does an `attribute`
//! block carry a valid `as:` value, do `?vars` unify, etc.) lives
//! on top of this one.
//!
//! ## Rules
//!
//! 1. **Document root** must be a YAML mapping. A bare scalar or
//!    sequence at the root is an error.
//! 2. **Level 1 keys** classify as one of:
//!    - DID/URI (the key string contains `:`)
//!    - bookmark name (any identifier without `:`)
//!    - `_` — anonymous fresh entity
//!    - `?<name>` — variable bound across the document
//!
//!    Non-string keys (numbers, booleans, sequences-as-keys) are
//!    flagged: asserted notation has no use for them.
//! 3. **Level 1 values** must be mappings. A scalar would mean
//!    "entity → atom" which has no claim representation.
//! 4. **Level 2 keys** classify as:
//!    - domain (contains `.`) — fields under it expand to claims
//!    - concept (no `.`) — fields are concept attributes
//!
//!    Domain keys starting with `dialog.` are flagged as **errors**
//!    per the RFC's reservation note.
//! 5. **Level 2 values** must be mappings.
//! 6. **Level 3** values are scalars or mappings. Sequences are
//!    legal under specific concept fields (`when:`, `unless:`)
//!    but flagging them generally would over-reach; this pass
//!    accepts them silently.
//!
//! Each violation becomes one [`Diagnostic`] with a precise range.

use lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use saphyr::{MarkedYaml, YamlData};

use crate::parse::{position_at, range_of};

/// Reserved domain prefix. The RFC carves out the `dialog.` namespace
/// for runtime internals; assertions using it should be rejected.
const RESERVED_DOMAIN_PREFIX: &str = "dialog.";

/// Run the full shape pass over a parsed document stream.
///
/// `documents` is the slice from [`crate::parse::Parsed::documents`].
/// The well-formedness check has already run by this point; if
/// parsing failed `documents` will be empty and this function
/// produces no diagnostics (the parse error already explains the
/// problem).
pub fn validate(documents: &[MarkedYaml<'_>]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for doc in documents {
        validate_document(doc, &mut diagnostics);
    }
    diagnostics
}

fn validate_document(doc: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) {
    let YamlData::Mapping(top) = &doc.data else {
        out.push(error(
            range_of(doc),
            "Asserted notation expects a mapping at the document root \
             (entity → context → field). Found a scalar or sequence.",
        ));
        return;
    };
    for (entity_key, entity_value) in top {
        validate_entity(entity_key, entity_value, out);
    }
}

/// Level-1 (entity) check: validate the key form, then descend into
/// the per-context mapping.
fn validate_entity(key: &MarkedYaml<'_>, value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) {
    let Some(name) = string_of(key) else {
        out.push(error(
            range_of(key),
            "Entity identifier must be a string (a DID/URI, a bookmark \
             name, `_`, or `?var`).",
        ));
        return;
    };
    // No further check beyond "is a string" — DID/bookmark/`_`/`?var`
    // all parse as plain YAML strings, and distinguishing them
    // semantically (e.g. flagging unbound bookmark names) needs
    // schema work that lives on top of this pass.
    let _ = name;

    let YamlData::Mapping(contexts) = &value.data else {
        out.push(error(
            range_of(value),
            "Entity value must be a mapping of context → fields. \
             Asserted notation has no representation for an entity \
             that is itself a scalar or sequence.",
        ));
        return;
    };
    for (context_key, context_value) in contexts {
        validate_context(context_key, context_value, out);
    }
}

/// Level-2 (context) check: domain vs. concept classification, the
/// reserved-domain guard, and "value must be a mapping."
fn validate_context(key: &MarkedYaml<'_>, value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) {
    let Some(name) = string_of(key) else {
        out.push(error(
            range_of(key),
            "Context name must be a string (a domain like \
             `io.gozala.person`, or a concept name like `attribute`).",
        ));
        return;
    };

    if name.contains('.') && name.starts_with(RESERVED_DOMAIN_PREFIX) {
        out.push(error(
            range_of(key),
            format!(
                "Domain `{name}` uses the reserved `dialog.` prefix. \
                 The runtime may reject assertions in this namespace."
            ),
        ));
        // Continue — the structural check below is still useful.
    }

    let YamlData::Mapping(fields) = &value.data else {
        out.push(error(
            range_of(value),
            "Context value must be a mapping of field → value.",
        ));
        return;
    };
    for (field_key, field_value) in fields {
        validate_field(field_key, field_value, out);
    }
}

/// Level-3 (field) check: keys must be strings; values can be
/// scalar (direct claim) or mapping (nested entity). Sequences
/// pass through silently — they're meaningful under specific
/// concept fields and over-rejecting them here would produce
/// noisy false positives.
fn validate_field(key: &MarkedYaml<'_>, _value: &MarkedYaml<'_>, out: &mut Vec<Diagnostic>) {
    if string_of(key).is_none() {
        out.push(error(range_of(key), "Field name must be a string."));
    }
}

/// Extract the underlying string form of a YAML scalar, if the node
/// is one. Saphyr models YAML scalars in two ways:
///
/// - `YamlData::Value(Scalar::String(s))` — a node whose value the
///   parser has interpreted as a string.
/// - `YamlData::Representation(repr, _, _)` — the *raw* text token,
///   useful for keys before tag interpretation runs.
///
/// We accept both and return the underlying `&str` so the caller
/// can pattern-match independently of which form saphyr produced.
fn string_of<'a>(node: &'a MarkedYaml<'_>) -> Option<&'a str> {
    use saphyr::Scalar;
    match &node.data {
        YamlData::Value(Scalar::String(s)) => Some(s.as_ref()),
        YamlData::Representation(text, _, _) => Some(text.as_ref()),
        _ => None,
    }
}

/// Build an error diagnostic with the asserted-notation source tag.
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

/// Quiet the unused-import warning when neither helper is actually
/// reached during a particular build configuration. Both are used
/// transitively below; the `position_at` is named explicitly so
/// `cargo check` doesn't flag it during minimal-feature builds.
#[allow(dead_code)]
fn _silence_position_at(m: &saphyr::Marker) {
    let _ = position_at(m);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn diagnose(text: &str) -> Vec<Diagnostic> {
        let parsed = parse::parse(text);
        validate(&parsed.documents)
    }

    #[dialog_common::test]
    fn well_formed_document_passes() {
        let diags = diagnose(
            "did:key:zAlice:\n\
             \x20 io.gozala.person:\n\
             \x20   name: Alice\n\
             \x20   age: 28\n",
        );
        assert!(diags.is_empty(), "unexpected diags: {diags:#?}");
    }

    #[dialog_common::test]
    fn anonymous_entity_passes() {
        let diags = diagnose(
            "_:\n\
             \x20 diy.cook:\n\
             \x20   quantity: 2\n",
        );
        assert!(diags.is_empty(), "unexpected diags: {diags:#?}");
    }

    #[dialog_common::test]
    fn variable_entity_passes() {
        let diags = diagnose(
            "?meal:\n\
             \x20 diy.planner:\n\
             \x20   recipe: pasta\n",
        );
        assert!(diags.is_empty(), "unexpected diags: {diags:#?}");
    }

    #[dialog_common::test]
    fn bookmark_concept_passes() {
        let diags = diagnose(
            "person:\n\
             \x20 concept:\n\
             \x20   description: A person\n",
        );
        assert!(diags.is_empty(), "unexpected diags: {diags:#?}");
    }

    #[dialog_common::test]
    fn root_scalar_is_an_error() {
        let diags = diagnose("just a string\n");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("mapping at the document root"));
    }

    #[dialog_common::test]
    fn entity_with_scalar_value_is_an_error() {
        let diags = diagnose("did:key:zAlice: hello\n");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("mapping of context"));
    }

    #[dialog_common::test]
    fn dialog_prefix_is_reserved() {
        let diags = diagnose(
            "did:key:zAlice:\n\
             \x20 dialog.attribute:\n\
             \x20   id: foo\n",
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`dialog.` prefix"));
        assert_eq!(diags[0].range.start.line, 1);
    }

    #[dialog_common::test]
    fn context_with_scalar_value_is_an_error() {
        let diags = diagnose(
            "did:key:zAlice:\n\
             \x20 io.gozala.person: hello\n",
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("mapping of field"));
    }
}
