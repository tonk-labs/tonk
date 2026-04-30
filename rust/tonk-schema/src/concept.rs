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

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::memory::Resolve;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch, RemoteSite};
use thiserror::Error;

pub use dialog_query::{AttributeDescriptor, ConceptDescriptor};

use crate::meta::{AttributeFacts, Name, Named};

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
pub fn with(
    field_name: &str,
) -> Result<ArtifactsAttribute, dialog_artifacts::DialogArtifactsError> {
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
pub fn maybe(
    field_name: &str,
) -> Result<ArtifactsAttribute, dialog_artifacts::DialogArtifactsError> {
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

// -----------------------------------------------------------------
// Concept builder — branch-side lookup of concept definitions.
// -----------------------------------------------------------------

/// A concept definition resolved from a branch — the entity URI
/// of the concept plus the reconstructed [`ConceptDescriptor`].
#[derive(Debug, Clone)]
pub struct Concept {
    /// The concept entity URI (`concept:…` or whatever entity
    /// carries the `dialog.concept.with/*` claims).
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: ConceptDescriptor,
}

/// Errors raised by the [`ConceptByName::resolve`] / [`ConceptByEntity::resolve`]
/// paths.
#[derive(Debug, Error)]
pub enum ConceptLookupError {
    /// A field of the concept references an entity that doesn't
    /// carry `dialog.attribute/*` facts — i.e., the concept's
    /// schema is corrupt or out of sync.
    #[error(
        "concept field {field:?} references entity {entity} \
         with no AttributeFacts"
    )]
    MissingAttribute {
        /// Field name on the concept.
        field: String,
        /// Entity URI that should have been an attribute.
        entity: String,
    },
    /// Underlying query failure (I/O, planner, etc.).
    #[error("{message}")]
    Query {
        /// Human-readable description.
        message: String,
    },
}

impl ConceptLookupError {
    fn query(message: impl Into<String>) -> Self {
        Self::Query {
            message: message.into(),
        }
    }
}

/// Standard environment bound for any [`Branch::query`]
/// invocation. Mirrors what dialog-repository's `SelectQuery`
/// requires; surfacing it as a single trait alias keeps the
/// builder signatures readable.
pub trait QueryEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> QueryEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

impl Concept {
    /// Look up a concept by its bookmark name (the value of a
    /// `dialog.meta/name` claim).
    pub fn by_name(name: impl Into<String>) -> ConceptByName {
        ConceptByName { name: name.into() }
    }

    /// Look up a concept by its entity URI directly — useful
    /// when the caller already knows it (e.g. from a previous
    /// query result) and just needs the descriptor reconstructed.
    pub fn by_entity(entity: Entity) -> ConceptByEntity {
        ConceptByEntity { entity }
    }
}

/// Builder for [`Concept::by_name`].
pub struct ConceptByName {
    name: String,
}

impl ConceptByName {
    /// Resolve the concept against a branch.
    ///
    /// Two-step query:
    /// 1. Find the entity carrying `dialog.meta/name = <name>`
    ///    via the typed [`Named`] concept query.
    /// 2. Delegate to [`ConceptByEntity::resolve`] for the
    ///    field-list reconstruction.
    pub async fn resolve<Env: QueryEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Concept>, ConceptLookupError> {
        let named: Vec<Named> = branch
            .query()
            .select(Query::<Named> {
                this: Term::var("this"),
                name: Term::from(Name(self.name.clone())),
            })
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ConceptLookupError::query(format!("Named query failed: {e:?}")))?;

        let Some(found) = named.into_iter().next() else {
            return Ok(None);
        };
        Concept::by_entity(found.this).resolve(branch, env).await
    }
}

/// Builder for [`Concept::by_entity`].
pub struct ConceptByEntity {
    entity: Entity,
}

impl ConceptByEntity {
    /// Resolve the concept's full descriptor by enumerating its
    /// `dialog.concept.with/*` claims and reconstructing each
    /// referenced [`AttributeDescriptor`].
    pub async fn resolve<Env: QueryEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Concept>, ConceptLookupError> {
        // Pull every claim where `(*entity, the, value)` matches
        // — `the` is left as a variable so the engine returns the
        // full set; we filter to the `dialog.concept.with/*`
        // namespace in Rust.
        let raw_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::var("the")
                    .of(Term::from(self.entity.clone()))
                    .is(Term::<Entity>::var("attribute")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ConceptLookupError::query(format!("concept-with query failed: {e:?}")))?;

        let mut fields: Vec<(String, AttributeDescriptor)> = Vec::new();
        for claim in raw_claims {
            let the: ArtifactsAttribute = claim.the.into();
            let Some(field_name) = parse_with(&the) else {
                continue;
            };
            let Ok(attribute_entity) = Entity::try_from(claim.is) else {
                continue;
            };
            let Some(facts) = AttributeByEntity::new(attribute_entity.clone())
                .resolve(branch, env)
                .await?
            else {
                return Err(ConceptLookupError::MissingAttribute {
                    field: field_name,
                    entity: attribute_entity.to_string(),
                });
            };
            fields.push((field_name, facts.descriptor));
        }

        if fields.is_empty() {
            return Ok(None);
        }

        Ok(Some(Concept {
            entity: self.entity,
            descriptor: ConceptDescriptor::from(fields),
        }))
    }
}

// -----------------------------------------------------------------
// AttributeByEntity — sister builder used internally by the
// concept resolver. Exposed publicly so the analyzer / route
// layer can reuse it without re-implementing the AttributeFacts
// → AttributeDescriptor reconstruction.
// -----------------------------------------------------------------

/// Resolved attribute — the entity plus the reconstructed
/// [`AttributeDescriptor`]. Same shape used by the analyzer's
/// `Resolver` trait.
#[derive(Debug, Clone)]
pub struct Attribute {
    /// The attribute entity URI.
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: AttributeDescriptor,
}

/// Builder for looking up an attribute's full fact-set by entity.
pub struct AttributeByEntity {
    entity: Entity,
}

impl AttributeByEntity {
    /// Construct a lookup for the given attribute entity.
    pub fn new(entity: Entity) -> Self {
        Self { entity }
    }

    /// Run the typed [`AttributeFacts`] query against the entity
    /// and reconstruct the descriptor.
    pub async fn resolve<Env: QueryEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        let facts: Vec<AttributeFacts> = branch
            .query()
            .select(Query::<AttributeFacts> {
                this: Term::from(self.entity.clone()),
                id: Term::var("id"),
                r#type: Term::var("type"),
                cardinality: Term::var("cardinality"),
                description: Term::var("description"),
            })
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| {
                ConceptLookupError::query(format!("AttributeFacts query failed: {e:?}"))
            })?;

        let Some(facts) = facts.into_iter().next() else {
            return Ok(None);
        };
        let descriptor = build_attribute_descriptor(&facts).map_err(ConceptLookupError::query)?;
        Ok(Some(Attribute {
            entity: self.entity,
            descriptor,
        }))
    }
}

/// Builder for looking up an attribute by its bookmark name
/// (the value of a `dialog.meta/name` claim).
pub struct AttributeByName {
    name: String,
}

impl AttributeByName {
    /// Construct a lookup for the given bookmark name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Resolve the attribute against a branch.
    ///
    /// Two-step query mirroring [`ConceptByName::resolve`]:
    /// first find the entity carrying
    /// `dialog.meta/name = <name>`, then delegate to
    /// [`AttributeByEntity::resolve`] for the fact-set
    /// reconstruction.
    pub async fn resolve<Env: QueryEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        let named: Vec<Named> = branch
            .query()
            .select(Query::<Named> {
                this: Term::var("this"),
                name: Term::from(Name(self.name.clone())),
            })
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ConceptLookupError::query(format!("Named query failed: {e:?}")))?;
        let Some(found) = named.into_iter().next() else {
            return Ok(None);
        };
        AttributeByEntity::new(found.this)
            .resolve(branch, env)
            .await
    }
}

/// Reconstruct an [`AttributeDescriptor`] from its
/// [`AttributeFacts`]. Round-trips through serde — the same
/// trick dialog itself uses, so we don't have to mirror the
/// internal `Type` ↔ string mapping.
fn build_attribute_descriptor(facts: &AttributeFacts) -> Result<AttributeDescriptor, String> {
    let mut shape = serde_json::Map::new();
    shape.insert(
        "the".to_owned(),
        serde_json::Value::String(facts.id.0.clone()),
    );
    if !facts.r#type.0.is_empty() {
        shape.insert(
            "as".to_owned(),
            serde_json::Value::String(facts.r#type.0.clone()),
        );
    }
    if !facts.cardinality.0.is_empty() {
        shape.insert(
            "cardinality".to_owned(),
            serde_json::Value::String(facts.cardinality.0.clone()),
        );
    }
    if !facts.description.0.is_empty() {
        shape.insert(
            "description".to_owned(),
            serde_json::Value::String(facts.description.0.clone()),
        );
    }
    serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| format!("could not reconstruct AttributeDescriptor: {e}"))
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
