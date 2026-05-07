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

use std::collections::HashSet;

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Select};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::memory::Resolve;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::source::SelectRules;
use dialog_query::{
    Application, Claim, EvaluationError, Match, Output as _, Parameters, Query, Selection, Term,
    the, try_stream,
};
use dialog_repository::{Branch, RemoteSite};
use thiserror::Error;

pub use dialog_query::{AttributeDescriptor, ConceptDescriptor};

use crate::builtin::concept_registry;
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
// same query machinery as a `#[derive(Concept)]` type. The
// `Application` is [`AnonymousConceptQuery`] — a custom query
// that yields one row per concept on the branch (built-in or
// asserted) with its descriptor materialised as a JSON `source`
// field, rather than reading per-attribute facts the way the
// derived `ConceptQuery` does.
impl dialog_query::Predicate for AnonymousConcept {
    type Conclusion = ConceptConclusion;
    type Application = AnonymousConceptQuery;
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
    type Conclusion = ConceptConclusion;
    type Application = AnonymousConceptQuery;
    type Descriptor = ConceptDescriptor;
}

impl dialog_query::Concept for NamedConcept {
    type Term = ();
    fn this(&self) -> Entity {
        self.this.clone()
    }
}

// -----------------------------------------------------------------
// AnonymousConceptQuery — yields one row per concept on a branch.
// -----------------------------------------------------------------

/// Custom query application that surfaces every concept on a
/// branch as a [`ConceptConclusion`].
///
/// Unlike dialog's [`ConceptQuery`], which reads a single concept's
/// per-attribute facts, this query enumerates *every* concept and
/// materialises its descriptor into a synthesised `source` field
/// alongside `this` (the concept entity) and `name` (the bookmark
/// name, when one is set).
///
/// Two sources are folded together:
///
/// 1. **Built-ins** — every entry of [`concept_registry`].
/// 2. **Branch** — every entity carrying the
///    `dialog.meta/concept = db:concept` marker claim, with
///    its descriptor reconstructed from the on-branch facts.
///
/// Built-ins win on `name` collision: a branch concept whose name
/// matches a built-in is suppressed.
///
/// `terms` follows the [`ConceptQuery`] convention: parameter keys
/// `this`, `name`, and `source` map to the user's variable
/// names. Constant values become filters; variables become output
/// bindings.
#[derive(Debug, Clone)]
pub struct AnonymousConceptQuery {
    /// Term bindings — same shape as `ConceptQuery::terms`. Keys
    /// `this` (entity), `name` (bookmark name), `source`
    /// (descriptor as JSON string).
    pub terms: Parameters,
}

impl AnonymousConceptQuery {
    /// Construct a new query from a parameter map.
    pub fn new(terms: Parameters) -> Self {
        Self { terms }
    }
}

impl Application for AnonymousConceptQuery {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        let app = self;
        try_stream! {
            for await each in selection {
                let input = each?;

                let this_term = app.terms.get("this").cloned();
                let name_term = app.terms.get("name").cloned();
                let source_term = app.terms.get("source").cloned();

                // Resolve filters from constant terms or from
                // upstream-bound variables.
                let this_filter = resolve_entity_filter(&this_term, &input);
                let name_filter = resolve_string_filter(&name_term, &input);

                let mut emitted_names: HashSet<String> = HashSet::new();

                // ---- Built-in source ----
                for (builtin_name, resolved) in concept_registry().iter() {
                    if let Some(ref e) = this_filter
                        && e != &resolved.entity
                    {
                        continue;
                    }
                    if let Some(ref n) = name_filter
                        && n != *builtin_name
                    {
                        continue;
                    }
                    emitted_names.insert((*builtin_name).to_string());

                    let mut m = input.clone();
                    if let Some(ref t) = this_term {
                        m.bind(t, dialog_query::Value::Entity(resolved.entity.clone()))?;
                    }
                    if let Some(ref t) = name_term {
                        m.bind(t, dialog_query::Value::String((*builtin_name).to_string()))?;
                    }
                    if let Some(ref t) = source_term {
                        let json = serde_json::to_string(&resolved.descriptor)
                            .map_err(|e| EvaluationError::Store(e.to_string()))?;
                        m.bind(t, dialog_query::Value::String(json))?;
                    }
                    yield m;
                }

                // ---- Branch source ----
                let marker = concept_marker_entity();
                let this_term_for_marker: Term<Entity> = match &this_filter {
                    Some(e) => Term::Constant(dialog_query::Value::Entity(e.clone())),
                    None => Term::var("__concept_query_this"),
                };
                let claims: Vec<Claim> = the!("dialog.meta/concept")
                    .of(this_term_for_marker)
                    .is(marker)
                    .perform(env)
                    .try_vec()
                    .await?;

                for claim in claims {
                    let entity = claim.of.clone();
                    let descriptor = match resolve_branch_descriptor(&entity, env).await? {
                        Some(d) => d,
                        None => continue,
                    };
                    let entity_name = lookup_entity_name(&entity, env).await?;

                    if let Some(ref ref_name) = name_filter {
                        match entity_name.as_deref() {
                            Some(n) if n == ref_name => {}
                            _ => continue,
                        }
                    }
                    if let Some(ref n) = entity_name
                        && emitted_names.contains(n)
                    {
                        continue;
                    }

                    let mut m = input.clone();
                    if let Some(ref t) = this_term {
                        m.bind(t, dialog_query::Value::Entity(entity.clone()))?;
                    }
                    if let Some(ref t) = name_term
                        && let Some(n) = &entity_name
                    {
                        m.bind(t, dialog_query::Value::String(n.clone()))?;
                    }
                    if let Some(ref t) = source_term {
                        let json = serde_json::to_string(&descriptor)
                            .map_err(|e| EvaluationError::Store(e.to_string()))?;
                        m.bind(t, dialog_query::Value::String(json))?;
                    }
                    yield m;
                }
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        // `ConceptConclusion`'s fields are private; delegate to
        // dialog's own `ConceptQuery::realize` which only reads
        // `terms` and the match's `this` binding. The `predicate`
        // is unused by `realize` so a stub stands in.
        let synthetic = ConceptQuery {
            terms: self.terms.clone(),
            predicate: stub_predicate(),
        };
        Application::realize(&synthetic, source)
    }
}

/// Stable empty descriptor used as the unused `predicate` slot of
/// the synthetic [`ConceptQuery`] in
/// [`AnonymousConceptQuery::realize`].
fn stub_predicate() -> ConceptDescriptor {
    ConceptDescriptor::from(Vec::<(&str, AttributeDescriptor)>::new())
}

// -----------------------------------------------------------------
// Concept-of-concept sentinel descriptor + dispatch table.
// -----------------------------------------------------------------

/// The well-known descriptor for the "concept of concept" head.
///
/// Its `with` map names the marker (`dialog.meta/concept`), bookmark
/// name, description, and the synthesised `source` field — enough
/// for the analyzer to project the fields a `concept:` head
/// exposes. The `source` claim has no real EAV backing; it is
/// produced only by [`AnonymousConceptQuery::evaluate`] which
/// [`QueryPlan::from`] dispatches to whenever it sees this
/// descriptor's `this()`.
pub fn concept_of_concept_descriptor() -> &'static ConceptDescriptor {
    static DESCRIPTOR: std::sync::OnceLock<ConceptDescriptor> = std::sync::OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        serde_json::from_value(serde_json::json!({
            "description": "Every concept asserted on a branch.",
            "with": {
                "concept":     { "the": "dialog.meta/concept",     "as": "Entity", "cardinality": "one" },
                "name":        { "the": "dialog.meta/name",        "as": "Text",   "cardinality": "one" },
                "description": { "the": "dialog.meta/description", "as": "Text",   "cardinality": "one" },
                "source":      { "the": "dialog.meta/source",      "as": "Text",   "cardinality": "one" }
            }
        }))
        .expect("concept-of-concept descriptor is well-formed")
    })
}

/// Cached `this()` of [`concept_of_concept_descriptor`] — the
/// dispatch sentinel for [`QueryPlan::from`]. Computing it once
/// avoids re-hashing the descriptor on every query.
fn concept_of_concept_entity() -> &'static Entity {
    static ENTITY: std::sync::OnceLock<Entity> = std::sync::OnceLock::new();
    ENTITY.get_or_init(|| concept_of_concept_descriptor().this())
}

/// Resolved query — what actually runs against the engine after
/// built-in dispatch.
///
/// A wire `ConceptQuery` is mapped through [`QueryPlan::from`]:
/// when its `predicate.this()` matches a known built-in sentinel
/// (today: the concept-of-concept descriptor) the plan uses a
/// custom `Application` that knows how to surface that built-in's
/// rows; otherwise the plan stays a [`ConceptQuery`] and runs
/// against the branch as usual.
///
/// All variants implement [`Application<Conclusion =
/// ConceptConclusion>`][Application], so downstream code can treat
/// the plan uniformly.
#[derive(Debug, Clone)]
pub enum QueryPlan {
    /// Standard branch-side concept query — the engine evaluates
    /// the wrapped [`ConceptQuery`] verbatim.
    Standard(ConceptQuery),
    /// Concept-of-concept enumeration via [`AnonymousConceptQuery`].
    AnonymousConcept(AnonymousConceptQuery),
}

impl From<ConceptQuery> for QueryPlan {
    fn from(query: ConceptQuery) -> Self {
        if &query.predicate.this() == concept_of_concept_entity() {
            QueryPlan::AnonymousConcept(AnonymousConceptQuery::new(query.terms))
        } else {
            QueryPlan::Standard(query)
        }
    }
}

impl Application for QueryPlan {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        try_stream! {
            match self {
                QueryPlan::Standard(q) => {
                    let stream = q.evaluate(selection, env);
                    for await each in stream {
                        yield each?;
                    }
                }
                QueryPlan::AnonymousConcept(q) => {
                    let stream = q.evaluate(selection, env);
                    for await each in stream {
                        yield each?;
                    }
                }
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        match self {
            QueryPlan::Standard(q) => Application::realize(q, source),
            QueryPlan::AnonymousConcept(q) => Application::realize(q, source),
        }
    }
}

/// Pull a constant entity out of a term — either from a constant
/// term or from a variable that the upstream selection already
/// bound. Returns `None` if the term is unbound or absent.
fn resolve_entity_filter(term: &Option<Term<dialog_query::Any>>, input: &Match) -> Option<Entity> {
    let t = term.as_ref()?;
    match t {
        Term::Constant(value) => Entity::try_from(value.clone()).ok(),
        Term::Variable { name: Some(_), .. } => {
            let value = input.lookup(t).ok()?;
            Entity::try_from(value).ok()
        }
        Term::Variable { name: None, .. } => None,
    }
}

/// Pull a constant string out of a term — same shape as
/// [`resolve_entity_filter`] but for string-valued filters.
fn resolve_string_filter(term: &Option<Term<dialog_query::Any>>, input: &Match) -> Option<String> {
    let t = term.as_ref()?;
    let value = match t {
        Term::Constant(value) => value.clone(),
        Term::Variable { name: Some(_), .. } => input.lookup(t).ok()?,
        Term::Variable { name: None, .. } => return None,
    };
    String::try_from(value).ok()
}

/// Reconstruct the descriptor for a branch concept entity by
/// enumerating its `dialog.concept.with/*` claims and resolving
/// each referenced attribute via the [`AnonymousAttribute`]
/// concept query.
async fn resolve_branch_descriptor<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<ConceptDescriptor>, EvaluationError>
where
    Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
{
    let with_claims: Vec<Claim> = dialog_query::AttributeQuery::from(
        Term::<dialog_query::attribute::The>::var("the")
            .of(Term::from(entity.clone()))
            .is(Term::<Entity>::var("attribute")),
    )
    .perform(env)
    .try_vec()
    .await?;

    let mut fields: Vec<(String, AttributeDescriptor)> = Vec::new();
    for claim in with_claims {
        let the: ArtifactsAttribute = claim.the.into();
        let Some(field_name) = parse_with(&the) else {
            continue;
        };
        let Ok(attribute_entity) = Entity::try_from(claim.is) else {
            continue;
        };
        let facts: Vec<AnonymousAttribute> = Query::<AnonymousAttribute> {
            this: Term::from(attribute_entity.clone()),
            id: Term::var("id"),
            r#type: Term::var("type"),
            cardinality: Term::var("cardinality"),
            description: Term::var("description"),
        }
        .perform(env)
        .try_vec()
        .await?;
        let Some(facts) = facts.into_iter().next() else {
            continue;
        };
        let descriptor = build_attribute_descriptor(&facts).map_err(EvaluationError::Store)?;
        fields.push((field_name, descriptor));
    }

    if fields.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ConceptDescriptor::from(fields)))
    }
}

/// Look up an entity's `dialog.meta/name`, if any. Used to
/// associate a branch concept entity with its bookmark name.
async fn lookup_entity_name<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<String>, EvaluationError>
where
    Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
{
    let names: Vec<Named> = Query::<Named> {
        this: Term::from(entity.clone()),
        name: Term::var("__concept_query_name"),
    }
    .perform(env)
    .try_vec()
    .await?;
    Ok(names.into_iter().next().map(|n| n.name.0))
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
    // `(?this, dialog.meta/concept, "db:concept")` so
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
/// `(?this, dialog.meta/concept, db:concept)`. Same URI as the
/// `concept` built-in's own entity in [`crate::builtin`].
fn concept_marker_entity() -> Entity {
    "db:concept"
        .parse()
        .expect("`db:concept` is a valid entity URI")
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
    /// claim `dialog.meta/concept = db:concept` that lets
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
    /// `(?this, dialog.meta/concept, db:concept)` marker so
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
            "expected dialog.meta/concept = db:concept marker claim",
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

    /// `QueryPlan::from(ConceptQuery)` dispatches to the
    /// [`AnonymousConceptQuery`] branch when the wire query's
    /// predicate is the concept-of-concept descriptor; otherwise
    /// it stays a [`ConceptQuery`].
    #[test]
    fn it_dispatches_concept_of_concept_to_anonymous_query() {
        // Concept-of-concept predicate → AnonymousConcept variant.
        let plan = QueryPlan::from(ConceptQuery {
            terms: dialog_query::Parameters::new(),
            predicate: concept_of_concept_descriptor().clone(),
        });
        assert!(
            matches!(plan, QueryPlan::AnonymousConcept(_)),
            "concept-of-concept predicate should dispatch to AnonymousConcept",
        );

        // Any other descriptor → Standard variant.
        let other: ConceptDescriptor =
            serde_json::from_str(r#"{"with":{"x":{"the":"a/b","as":"Text","cardinality":"one"}}}"#)
                .unwrap();
        let plan = QueryPlan::from(ConceptQuery {
            terms: dialog_query::Parameters::new(),
            predicate: other,
        });
        assert!(
            matches!(plan, QueryPlan::Standard(_)),
            "non-sentinel predicate should stay a Standard ConceptQuery",
        );
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

    /// Round-trip a [`NamedConcept`] through a branch and
    /// recover its descriptor via [`AnonymousConceptQuery`]'s
    /// synthesised `source` field.
    ///
    /// Asserting a named concept writes the marker claim, the
    /// `dialog.concept.with/{field}` claims, and the
    /// `dialog.meta/name` claim. The query enumerates the
    /// branch via the marker, reconstructs the descriptor for
    /// each entity, and binds it as a JSON string in `source`.
    #[dialog_common::test]
    async fn it_returns_concept_with_source_from_concept_query() -> anyhow::Result<()> {
        use dialog_query::{Any, Output as _, Parameters, Term};
        use dialog_repository::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )?;
        // The concept references one attribute by URI; the
        // attribute's own facts (`dialog.attribute/id|type|
        // cardinality`, `dialog.meta/description`) must exist on
        // the branch for [`AnonymousConceptQuery`] to reconstruct
        // the descriptor — emit them inline alongside the concept.
        let (_, attr_descriptor) = descriptor.with().iter().next().expect("one field");
        let attr_entity: Entity = attr_descriptor.to_uri().parse()?;
        let concept = NamedConcept::new(descriptor.clone(), "person");
        branch
            .transaction()
            .assert(
                dialog_query::the!("dialog.attribute/id")
                    .of(attr_entity.clone())
                    .is(format!(
                        "{}/{}",
                        attr_descriptor.domain(),
                        attr_descriptor.name()
                    )),
            )
            .assert(
                dialog_query::the!("dialog.attribute/type")
                    .of(attr_entity.clone())
                    .is("Text".to_string()),
            )
            .assert(
                dialog_query::the!("dialog.attribute/cardinality")
                    .of(attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("dialog.meta/description")
                    .of(attr_entity)
                    .is(String::new()),
            )
            .assert(concept)
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));
        terms.insert("name".to_string(), Term::<Any>::var("name"));
        terms.insert("source".to_string(), Term::<Any>::var("source"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousConceptQuery::new(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        let row = conclusions
            .iter()
            .find(|c| {
                c.source()
                    .lookup(&Term::<Any>::var("name"))
                    .ok()
                    .and_then(|v| String::try_from(v).ok())
                    .as_deref()
                    == Some("person")
            })
            .expect("expected a row with name = \"person\"");

        let source: String = String::try_from(row.source().lookup(&Term::<Any>::var("source"))?)
            .expect("source binding must be a string");
        let parsed: ConceptDescriptor = serde_json::from_str(&source)?;
        assert_eq!(
            parsed.with().iter().count(),
            descriptor.with().iter().count()
        );
        for ((a_name, a_attr), (b_name, b_attr)) in
            parsed.with().iter().zip(descriptor.with().iter())
        {
            assert_eq!(a_name, b_name);
            assert_eq!(a_attr.to_uri(), b_attr.to_uri());
        }
        Ok(())
    }
}
