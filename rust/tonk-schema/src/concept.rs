//! User-defined concepts and the attributes that name their fields.
//!
//! Concepts in dialog are identified structurally — by the set of
//! attribute URIs they require — and the canonical identifier is
//! produced by [`ConceptDescriptor::this`]. Field names are *not*
//! part of that identity; two concepts that require the same
//! attributes under different field names converge on the same
//! `concept:…` entity.
//!
//! Field names are nevertheless useful — they let callers say
//! "the `title` of this recipe" rather than "the value at attribute
//! `recipe/title` of this entity". This module captures that link
//! as a separate fact: for each field of a concept, an EAV claim
//! whose `the` is `dialog.concept.with/{fieldName}` (or
//! `dialog.concept.maybe/{fieldName}` for optional fields) and
//! whose value is the attribute entity URI.
//!
//! The relation namespaces (`dialog.concept.with`,
//! `dialog.concept.maybe`) match those used by the `carry` CLI so
//! that field-name facts written by either tool describe the same
//! concept identically.

use dialog_artifacts::Attribute as ArtifactsAttribute;

pub use dialog_query::{AttributeDescriptor, ConceptDescriptor};

/// Domain prefix for required-field claims.
const WITH_DOMAIN: &str = "dialog.concept.with";

/// Domain prefix for optional-field claims.
const MAYBE_DOMAIN: &str = "dialog.concept.maybe";

/// Build the claim relation that names a required field of a
/// concept.
///
/// The returned [`ArtifactsAttribute`] has the form
/// `dialog.concept.with/{field_name}`. Used as the `the` of an EAV
/// claim `concept_entity --with(name)--> attribute_entity` to
/// record that the concept has a required field named `name`
/// pointing at the given attribute.
///
/// Field names are passed through verbatim — dialog's lower-level
/// [`ArtifactsAttribute`] only enforces a `domain/name` shape and a
/// length cap, so any field name a YAML or JSON schema accepts is
/// accepted here.
pub fn with(field_name: &str) -> Result<ArtifactsAttribute, dialog_artifacts::DialogArtifactsError> {
    format!("{WITH_DOMAIN}/{field_name}").parse()
}

/// Build the claim relation that names an optional field of a
/// concept.
///
/// Same shape as [`with`] but in the `dialog.concept.maybe` domain.
/// Currently informational — `dialog_query`'s engine does not yet
/// deduce over `maybe` fields (per the doc comment on
/// [`ConceptDescriptor::maybe`]) — but the namespace is reserved
/// here so concept definitions written today carry their optional
/// fields in the form the engine will eventually understand.
pub fn maybe(field_name: &str) -> Result<ArtifactsAttribute, dialog_artifacts::DialogArtifactsError> {
    format!("{MAYBE_DOMAIN}/{field_name}").parse()
}

/// Recover the field name from a relation in the
/// `dialog.concept.with` domain. Returns `None` if `the` is in any
/// other domain.
pub fn parse_with(the: &ArtifactsAttribute) -> Option<String> {
    let s = String::from(the);
    s.strip_prefix(WITH_DOMAIN)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_owned)
}

/// Recover the field name from a relation in the
/// `dialog.concept.maybe` domain. Returns `None` if `the` is in any
/// other domain.
pub fn parse_maybe(the: &ArtifactsAttribute) -> Option<String> {
    let s = String::from(the);
    s.strip_prefix(MAYBE_DOMAIN)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_constructs_namespaced_relation() {
        let the = with("title").unwrap();
        assert_eq!(String::from(&the), "dialog.concept.with/title");
    }

    #[test]
    fn maybe_constructs_namespaced_relation() {
        let the = maybe("subtitle").unwrap();
        assert_eq!(String::from(&the), "dialog.concept.maybe/subtitle");
    }

    #[test]
    fn parse_with_round_trips() {
        let the = with("ingredient-name").unwrap();
        assert_eq!(parse_with(&the).as_deref(), Some("ingredient-name"));
    }

    #[test]
    fn parse_maybe_round_trips() {
        let the = maybe("notes").unwrap();
        assert_eq!(parse_maybe(&the).as_deref(), Some("notes"));
    }

    #[test]
    fn parse_with_rejects_other_domains() {
        let the: ArtifactsAttribute = "dialog.meta/name".parse().unwrap();
        assert_eq!(parse_with(&the), None);
        assert_eq!(parse_maybe(&the), None);
    }

    #[test]
    fn parse_with_rejects_maybe_domain() {
        let the = maybe("x").unwrap();
        assert_eq!(parse_with(&the), None);
    }

    #[test]
    fn descriptor_round_trips_through_json() {
        let json = r#"{
            "description": "A cooking recipe",
            "with": {
                "title": { "the": "recipe/title", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let entity = descriptor.this();
        assert!(entity.to_string().starts_with("concept:"));
    }
}
