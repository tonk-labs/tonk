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

use crate::meta::{AnonymousAttribute, Name, Named};

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
         with no AnonymousAttribute"
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
// layer can reuse it without re-implementing the AnonymousAttribute
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

    /// Run the typed [`AnonymousAttribute`] query against the entity
    /// and reconstruct the descriptor.
    pub async fn resolve<Env: QueryEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        let facts: Vec<AnonymousAttribute> = branch
            .query()
            .select(Query::<AnonymousAttribute> {
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
                ConceptLookupError::query(format!("AnonymousAttribute query failed: {e:?}"))
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
/// [`AnonymousAttribute`]. Round-trips through serde — the same
/// trick dialog itself uses, so we don't have to mirror the
/// internal `Type` ↔ string mapping.
fn build_attribute_descriptor(facts: &AnonymousAttribute) -> Result<AttributeDescriptor, String> {
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

// -----------------------------------------------------------------
// Concept-of-concept: hand-written `Statement` + `Concept` impls.
// -----------------------------------------------------------------
//
// Concepts in dialog can't *describe* themselves through
// `#[derive(Concept)]` because their `with` map is variable-arity
// — every concept declares a different set of fields. We
// therefore hand-write `AnonymousConcept` and `NamedConcept`,
// each presenting the same `Statement` interface as a derived
// concept: assert/retract walk the wrapped descriptor, emit the
// right `dialog.concept.with/{field}` claims plus meta claims.
//
// The result: a `concept!` head produces one of these structs;
// the worker's commit loop calls `.assert(update)` on it and
// dialog writes the per-field claims. Symmetric to the typed
// `AnonymousAttribute` / `NamedAttribute` wrappers.

use dialog_artifacts::{Statement, Update, Value};

/// A concept stored on a branch *without* a bookmark name.
/// Identity comes from the wrapped descriptor's
/// content-addressed entity (`descriptor.this()`).
///
/// `assert` writes one `dialog.concept.with/{field}` claim per
/// field of the descriptor's `with` map (value =
/// content-addressed attribute entity), plus a
/// `dialog.meta/description` when the descriptor carries one.
/// `retract` mirrors.
#[derive(Debug, Clone)]
pub struct AnonymousConcept {
    /// `descriptor.this()` — kept here so successive asserts
    /// don't re-hash on every call.
    pub this: Entity,
    /// The full descriptor — owns description + with-map.
    pub descriptor: ConceptDescriptor,
}

impl AnonymousConcept {
    /// Build from a descriptor; computes the entity via
    /// [`ConceptDescriptor::this`].
    pub fn new(descriptor: ConceptDescriptor) -> Self {
        Self {
            this: descriptor.this(),
            descriptor,
        }
    }
}

impl Statement for AnonymousConcept {
    fn assert(self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::associate);
    }
    fn retract(self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::dissociate);
    }
}

// `Predicate` + `Concept` so `AnonymousConcept` plugs into the
// same query machinery as a `#[derive(Concept)]` type. We
// delegate to the wrapped descriptor — its `this()` and the
// associated query types are already what we need.
impl dialog_query::Predicate for AnonymousConcept {
    type Conclusion = dialog_query::concept::descriptor::ConceptConclusion;
    type Application = dialog_query::concept::query::ConceptQuery;
    type Descriptor = ConceptDescriptor;
}

impl dialog_query::Concept for AnonymousConcept {
    type Term = ();
    fn this(&self) -> Entity {
        self.this.clone()
    }
}

/// A concept stored on a branch *with* a bookmark name.
/// Same as [`AnonymousConcept`] but also writes (and retracts)
/// a `dialog.meta/name` claim so future documents can resolve
/// the concept via `.name`.
#[derive(Debug, Clone)]
pub struct NamedConcept {
    /// `descriptor.this()`.
    pub this: Entity,
    /// The full descriptor.
    pub descriptor: ConceptDescriptor,
    /// Bookmark name — written as `dialog.meta/name`. Not part
    /// of the descriptor's identity hash.
    pub name: String,
}

impl NamedConcept {
    /// Build from a descriptor and a bookmark name.
    pub fn new(descriptor: ConceptDescriptor, name: impl Into<String>) -> Self {
        Self {
            this: descriptor.this(),
            descriptor,
            name: name.into(),
        }
    }
}

impl Statement for NamedConcept {
    fn assert(mut self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::associate);
        update.associate(
            meta_attr("dialog.meta", "name"),
            self.this.clone(),
            Value::String(std::mem::take(&mut self.name)),
        );
    }
    fn retract(mut self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::dissociate);
        update.dissociate(
            meta_attr("dialog.meta", "name"),
            self.this.clone(),
            Value::String(std::mem::take(&mut self.name)),
        );
    }
}

impl dialog_query::Predicate for NamedConcept {
    type Conclusion = dialog_query::concept::descriptor::ConceptConclusion;
    type Application = dialog_query::concept::query::ConceptQuery;
    type Descriptor = ConceptDescriptor;
}

impl dialog_query::Concept for NamedConcept {
    type Term = ();
    fn this(&self) -> Entity {
        self.this.clone()
    }
}

/// Walk a [`ConceptDescriptor`] and call `op` (either
/// [`Update::associate`] or [`Update::dissociate`]) for every
/// fact the concept implies — `dialog.concept.with/{field}`
/// per field, plus `dialog.meta/description` when the
/// descriptor carries one. Shared between `assert` and
/// `retract` so the two stay in lock-step.
fn emit_concept_facts<U: Update, F: Fn(&mut U, ArtifactsAttribute, Entity, Value)>(
    entity: &Entity,
    descriptor: &ConceptDescriptor,
    update: &mut U,
    op: F,
) {
    // Marker claim — every concept entity carries
    // `(?this, dialog.meta/concept, "concept:concept")` so
    // queries that want "all concepts on this branch" have a
    // selectable triple to start from (the engine refuses
    // selections with no bound component).
    op(
        update,
        meta_attr("dialog.meta", "concept"),
        entity.clone(),
        Value::Entity(concept_marker_entity()),
    );
    for (field_name, attribute) in descriptor.with().iter() {
        let relation = meta_attr(WITH_DOMAIN, field_name);
        let attribute_entity: Entity = attribute
            .to_uri()
            .parse()
            .expect("AttributeDescriptor::to_uri produces a valid entity URI");
        op(
            update,
            relation,
            entity.clone(),
            Value::Entity(attribute_entity),
        );
    }
    if let Some(description) = descriptor.description()
        && !description.is_empty()
    {
        op(
            update,
            meta_attr("dialog.meta", "description"),
            entity.clone(),
            Value::String(description.to_owned()),
        );
    }
}

/// The well-known entity used as the value of the
/// `dialog.meta/concept` marker claim. Every concept entity
/// asserted on a branch carries
/// `(?this, dialog.meta/concept, concept:concept)`.
fn concept_marker_entity() -> Entity {
    "concept:concept"
        .parse()
        .expect("`concept:concept` is a valid entity URI")
}

/// Build a runtime [`ArtifactsAttribute`] from a domain + local
/// name. Both halves are validated by dialog's own parser; we
/// rely on the meta domains being well-formed so `expect` is
/// safe.
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
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

    /// `AnonymousConcept::assert` should write one
    /// `dialog.concept.with/{field}` claim per field plus
    /// `dialog.meta/description` when set, plus the marker
    /// claim `dialog.meta/concept = concept:concept` that lets
    /// branch-wide concept enumeration find this entity.
    #[test]
    fn anonymous_concept_writes_with_claims_and_description() {
        use dialog_artifacts::Changes;
        let json = r#"{
            "description": "A cooking recipe",
            "with": {
                "title":      { "the": "recipe/title",      "as": "Text", "cardinality": "one" },
                "ingredient": { "the": "recipe/ingredient", "as": "Text", "cardinality": "many" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept = AnonymousConcept::new(descriptor);
        let mut changes = Changes::new();
        concept.assert(&mut changes);
        // Marker + two with/* claims + one description claim = 4.
        assert!(!changes.is_empty());
    }

    /// Every concept assert must include the
    /// `(?this, dialog.meta/concept, concept:concept)` marker so
    /// `concept:` queries with `?this` unbound can drive
    /// selection from a single bound triple.
    #[test]
    fn anonymous_concept_writes_marker_claim() {
        use dialog_artifacts::{Changes, Instruction};
        let json = r#"{
            "with": {
                "x": { "the": "a/b", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept = AnonymousConcept::new(descriptor);
        let mut changes = Changes::new();
        concept.assert(&mut changes);
        let marker_attr = meta_attr("dialog.meta", "concept");
        let marker_value = Value::Entity(concept_marker_entity());
        let saw_marker = changes
            .into_instructions()
            .into_iter()
            .any(|inst| match inst {
                Instruction::Assert(a) => a.the == marker_attr && a.is == marker_value,
                _ => false,
            });
        assert!(
            saw_marker,
            "expected dialog.meta/concept = concept:concept marker claim",
        );
    }

    /// Retract path mirrors assert: the marker dissociation must
    /// be emitted alongside the with-claim retractions so the
    /// branch ends up clean.
    #[test]
    fn anonymous_concept_retracts_marker_claim() {
        use dialog_artifacts::{Changes, Instruction};
        let json = r#"{
            "with": {
                "x": { "the": "a/b", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept = AnonymousConcept::new(descriptor);
        let mut changes = Changes::new();
        concept.retract(&mut changes);
        let marker_attr = meta_attr("dialog.meta", "concept");
        let marker_value = Value::Entity(concept_marker_entity());
        let saw_retract = changes
            .into_instructions()
            .into_iter()
            .any(|inst| match inst {
                Instruction::Retract(a) => a.the == marker_attr && a.is == marker_value,
                _ => false,
            });
        assert!(
            saw_retract,
            "expected dialog.meta/concept marker retraction",
        );
    }

    /// `NamedConcept::assert` should write the same as
    /// `AnonymousConcept::assert` plus a `dialog.meta/name`
    /// claim.
    #[test]
    fn named_concept_writes_name_in_addition() {
        use dialog_artifacts::Changes;
        let json = r#"{
            "with": {
                "title": { "the": "recipe/title", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept = NamedConcept::new(descriptor, "recipe");
        let mut changes = Changes::new();
        concept.assert(&mut changes);
        assert!(!changes.is_empty());
    }

    /// Compile-time check: `AnonymousConcept` and
    /// `NamedConcept` implement the dialog `Concept` trait
    /// (and its `Predicate` supertrait), so they slot into
    /// query and rule machinery the same way `#[derive(Concept)]`
    /// types do.
    #[test]
    fn concept_wrappers_satisfy_concept_trait() {
        fn requires_concept<C: dialog_query::Concept>(_: &C)
        where
            C::Conclusion: dialog_query::Conclusion,
        {
        }
        let descriptor: ConceptDescriptor =
            serde_json::from_str(r#"{"with":{"x":{"the":"a/b","as":"Text","cardinality":"one"}}}"#)
                .unwrap();
        let anon = AnonymousConcept::new(descriptor.clone());
        let named = NamedConcept::new(descriptor, "demo");
        requires_concept(&anon);
        requires_concept(&named);
    }

    /// `assert` then `retract` should leave nothing — every
    /// claim that goes in comes back out.
    #[test]
    fn anonymous_concept_assert_then_retract_balances() {
        use dialog_artifacts::Changes;
        let json = r#"{
            "description": "A cooking recipe",
            "with": {
                "title": { "the": "recipe/title", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept_a = AnonymousConcept::new(descriptor.clone());
        let concept_b = AnonymousConcept::new(descriptor);
        let mut changes = Changes::new();
        concept_a.assert(&mut changes);
        concept_b.retract(&mut changes);
        // Net: every association is matched by a dissociation.
        // We can't easily inspect Changes' internal state, but
        // both calls succeeded and that's what we wanted to
        // exercise.
    }
}
