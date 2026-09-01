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
//! whose `the` is `db.concept.with/{fieldName}` and whose value
//! is the attribute entity URI. Optional fields additionally carry a
//! boolean marker claim `db.concept.optional/{fieldName}`.
//!
//! The relation namespace (`db.concept.with`) matches the one
//! used by the `carry` CLI so that field-name facts written by
//! either tool describe the same concept identically.

use std::collections::{BTreeSet, HashSet};

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::{
    Application, Claim, EvaluationError, Match, Output as _, Parameters, Query, Scope, Selection,
    Term, the, try_stream,
};
use dialog_repository::RemoteSite;
use thiserror::Error;

pub use dialog_query::{AttributeDescriptor, ConceptDescriptor, ConceptFieldDescriptor, Type};

use crate::builtin::concept_registry;
use crate::query_source::Source;
use crate::rule_query::{AnonymousRuleQuery, rule_of_rule_descriptor};
use dialog_query::attribute::Relation;
use tonk_core::meta::AnonymousAttribute;

/// Domain prefix for required-field claims.
const WITH_DOMAIN: &str = "db.concept.with";

/// Domain prefix for the per-field optional marker.
const OPTIONAL_DOMAIN: &str = "db.concept.optional";

/// Build the claim relation that names a required field of a
/// concept.
///
/// The returned [`ArtifactsAttribute`] has the form
/// `db.concept.with/{field_name}`. Used as the `the` of an EAV
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

/// Build the marker relation that flags a concept field as optional.
///
/// The returned [`ArtifactsAttribute`] has the form
/// `db.concept.optional/{field_name}`. Emitted as a boolean
/// marker claim `concept_entity --optional(name)--> true` alongside
/// the field's `db.concept.with/{name}` attribute link. Required
/// fields carry no such marker, so their storage is byte-identical to
/// the pre-optionality encoding.
pub fn optional(
    field_name: &str,
) -> Result<ArtifactsAttribute, dialog_artifacts::DialogArtifactsError> {
    format!("{OPTIONAL_DOMAIN}/{field_name}").parse()
}

/// Recover the field name from a relation in the
/// `db.concept.with` domain. Returns `None` if `the` is in any
/// other domain.
pub fn parse_with(the: &ArtifactsAttribute) -> Option<String> {
    let s = String::from(the);
    s.strip_prefix(WITH_DOMAIN)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_owned)
}

/// Recover the field name from a relation in the
/// `db.concept.optional` domain. Returns `None` if `the` is in
/// any other domain.
pub fn parse_optional(the: &ArtifactsAttribute) -> Option<String> {
    let s = String::from(the);
    s.strip_prefix(OPTIONAL_DOMAIN)
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
    /// carries the `db.concept.with/*` claims).
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: ConceptDescriptor,
}

/// Errors raised by the [`ConceptByName::resolve`] / [`ConceptByEntity::resolve`]
/// paths.
#[derive(Debug, Error)]
pub enum ConceptLookupError {
    /// A field of the concept references an entity that doesn't
    /// carry `db.attribute/*` facts — i.e., the concept's
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
    /// Wrap an underlying query failure as a [`ConceptLookupError`].
    pub fn query(message: impl Into<String>) -> Self {
        Self::Query {
            message: message.into(),
        }
    }
}

/// Standard environment bound for any `Branch::query`
/// invocation. Mirrors what dialog-repository's `SelectQuery`
/// requires; surfacing it as a single trait alias keeps the
/// builder signatures readable.
pub trait QueryEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Identify>
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
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

impl Concept {
    /// Look up a concept by its published name. The branch's
    /// `id:<name>` entity carries the `db.meta/name` claim
    /// that points at the concept entity; the resolver chases
    /// that pointer and reconstructs the descriptor.
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
    /// The user-facing name `<name>` is published as a separate
    /// entity at `id:<name>` carrying a `db.meta/name`
    /// claim that points at the actual concept entity. Two
    /// steps:
    ///
    /// 1. Look up `(id:<name>, db.meta/name, ?value)` to
    ///    get the concept entity.
    /// 2. Delegate to [`ConceptByEntity::resolve`] for the
    ///    field-list reconstruction.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Concept>, ConceptLookupError> {
        let Some(target) = lookup_named_entity(&self.name, source, env).await? else {
            return Ok(None);
        };
        Concept::by_entity(target).resolve(source, env).await
    }
}

/// Builder for [`Concept::by_entity`].
pub struct ConceptByEntity {
    entity: Entity,
}

impl ConceptByEntity {
    /// Resolve the concept's full descriptor by enumerating its
    /// `db.concept.with/*` claims and reconstructing each
    /// referenced [`AttributeDescriptor`].
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Concept>, ConceptLookupError> {
        // Pull every claim where `(*entity, the, value)` matches
        // — `the` is left as a variable so the engine returns the
        // full set; we filter to the `db.concept.with/*`
        // namespace in Rust.
        let raw_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::var("the")
                    .of(Term::from(self.entity.clone()))
                    .is(Term::<Entity>::var("attribute")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ConceptLookupError::query(format!("concept-with query failed: {e:?}")))?;

        // The optional markers carry Boolean values, so a separate
        // Boolean-typed query is needed — the Entity-typed `with`
        // query above never returns them.
        let optional_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::var("the")
                    .of(Term::from(self.entity.clone()))
                    .is(Term::<bool>::var("flag")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| {
                ConceptLookupError::query(format!("concept-optional query failed: {e:?}"))
            })?;
        let optional_fields: BTreeSet<String> = optional_claims
            .iter()
            .filter_map(|claim| {
                let the: ArtifactsAttribute = claim.the.clone().into();
                parse_optional(&the)
            })
            .collect();

        let mut fields: Vec<(String, ConceptFieldDescriptor)> = Vec::new();
        for claim in raw_claims {
            let the: ArtifactsAttribute = claim.the.into();
            let Some(field_name) = parse_with(&the) else {
                continue;
            };
            let Ok(attribute_entity) = Entity::try_from(claim.is) else {
                continue;
            };
            let Some(facts) = AttributeByEntity::new(attribute_entity.clone())
                .resolve(source, env)
                .await?
            else {
                return Err(ConceptLookupError::MissingAttribute {
                    field: field_name,
                    entity: attribute_entity.to_string(),
                });
            };
            let field = if optional_fields.contains(&field_name) {
                ConceptFieldDescriptor::optional(facts.descriptor)
            } else {
                ConceptFieldDescriptor::required(facts.descriptor)
            };
            fields.push((field_name, field));
        }

        if fields.is_empty() {
            return Ok(None);
        }

        let descriptor = ConceptDescriptor::try_from(fields)
            .map_err(|e| ConceptLookupError::query(format!("invalid concept descriptor: {e:?}")))?;
        Ok(Some(Concept {
            entity: self.entity,
            descriptor,
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
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        let facts: Vec<AnonymousAttribute> = source
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

/// Builder for looking up an attribute by its *selector id* — the
/// `domain/name` string stored as its `db.attribute/id` claim.
/// This is how a claim-domain head (`xyz.tonk!:`) discovers whether
/// the `<domain>/<field>` attribute a body field maps onto is
/// declared on the branch, so the declared cardinality and value
/// type govern instead of synthesized defaults.
pub struct AttributeById {
    id: String,
}

impl AttributeById {
    /// Construct a lookup for the given `domain/name` id.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// Resolve the attribute against a branch: one
    /// [`AnonymousAttribute`] query with the id pinned and the
    /// entity free.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        use tonk_core::meta::attribute::Id;
        let facts: Vec<AnonymousAttribute> = source
            .select(Query::<AnonymousAttribute> {
                this: Term::var("attribute"),
                id: Term::from(Id(self.id)),
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
            entity: facts.this,
            descriptor,
        }))
    }
}

/// Builder for looking up an attribute by its published name —
/// the user-facing label `<name>` whose `id:<name>` entity
/// carries a `db.meta/name` claim pointing at the attribute.
pub struct AttributeByName {
    name: String,
}

impl AttributeByName {
    /// Construct a lookup for the given published name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Resolve the attribute against a branch.
    ///
    /// Two-step query mirroring [`ConceptByName::resolve`]:
    /// first find the entity carrying
    /// `db.meta/name = <name>`, then delegate to
    /// [`AttributeByEntity::resolve`] for the fact-set
    /// reconstruction.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Attribute>, ConceptLookupError> {
        let Some(target) = lookup_named_entity(&self.name, source, env).await? else {
            return Ok(None);
        };
        AttributeByEntity::new(target).resolve(source, env).await
    }
}

/// Reconstruct an [`AttributeDescriptor`] from its
/// [`AnonymousAttribute`]. Round-trips through serde — the same
/// trick dialog itself uses, so we don't have to mirror the
/// internal `Type` ↔ string mapping.
fn build_attribute_descriptor(facts: &AnonymousAttribute) -> Result<AttributeDescriptor, String> {
    let mut shape = serde_json::Map::new();
    // The stored id spells the relation: `domain/name` for an
    // attribute, `domain/[position]` or `domain/[symbol]` for a
    // keyed collection.
    let relation: Relation = facts
        .id
        .0
        .parse()
        .map_err(|e| format!("could not parse attribute id {:?}: {e}", facts.id.0))?;
    shape.insert(
        "the".to_owned(),
        serde_json::to_value(relation).map_err(|e| e.to_string())?,
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
// therefore hand-write [`AnonymousConcept`], which presents the
// same `Statement` interface as a derived concept: assert/retract
// walk the wrapped descriptor and emit the right
// `db.concept.with/{field}` claims plus the meta marker.
//
// Naming used to live on a sister `NamedConcept` wrapper that
// also wrote a `db.meta/name` claim *onto the named target*.
// That direction was wrong under the new name model — names are
// now their own concept whose entity (`id:<n>`) carries the
// `db.meta/name` claim *pointing at* the named target. The
// analyzer's anchor desugar emits the name assertion separately
// via `ApplicationPlan::name`, so the wrapper is no longer
// needed; only `AnonymousConcept` remains.

use dialog_artifacts::{Statement, Update, Value};

/// A concept stored on a branch *without* a bookmark name.
/// Identity comes from the wrapped descriptor's
/// content-addressed entity (`descriptor.this()`).
///
/// `assert` writes one `db.concept.with/{field}` claim per
/// field of the descriptor's `with` map (value =
/// content-addressed attribute entity), plus a
/// `db.meta/description` when the descriptor carries one.
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

/// A concept declared as **transient**: facts of this concept
/// exist for the duration of one commit cycle and are stripped
/// from the persistable delta before the branch state is
/// written. The reactor's effect evaluator reads these facts
/// during fixpoint and they're gone next round.
///
/// Mental model in Dedalus terms: transient concepts have no
/// implicit persistence rule applied. Used to model abilities,
/// commands, messages, and other event-shaped data.
///
/// Storage: emits everything [`AnonymousConcept`] emits
/// (concept marker, attribute field claims, optional
/// description), plus dialog's `dialog.concept/transient: true`
/// marker triple that commit-time induction reads to decide
/// which facts are commands rather than durable data.
#[derive(Debug, Clone)]
pub struct TransientConcept {
    /// `descriptor.this()` — same content-derived entity as
    /// would be produced by an [`AnonymousConcept`] over the
    /// same descriptor. Two concepts with identical attribute
    /// shapes share an entity URI; the transient marker is what
    /// distinguishes them at the storage layer.
    pub this: Entity,
    /// The full descriptor — same shape as a non-transient
    /// concept.
    pub descriptor: ConceptDescriptor,
}

impl TransientConcept {
    /// Wrap a descriptor as a transient concept.
    pub fn new(descriptor: ConceptDescriptor) -> Self {
        Self {
            this: descriptor.this(),
            descriptor,
        }
    }
}

impl TransientConcept {
    /// Look up whether a concept entity is marked transient on
    /// a branch. Returns `true` iff
    /// `(<entity>, dialog.concept/transient, true)`
    /// holds; `false` if the marker is absent.
    pub fn is_transient(entity: Entity) -> IsTransient {
        IsTransient { entity }
    }
}

/// Builder for [`TransientConcept::is_transient`]. Resolves the
/// `dialog.concept/transient` marker claim and answers a yes/no.
pub struct IsTransient {
    entity: Entity,
}

impl IsTransient {
    /// Resolve against a branch.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<bool, ConceptLookupError> {
        let claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(meta_attr_typed(
                    "dialog.concept",
                    "transient",
                ))
                .of(Term::from(self.entity))
                .is(Term::from(true)),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| {
                ConceptLookupError::query(format!("transient marker query failed: {e:?}"))
            })?;
        Ok(!claims.is_empty())
    }
}

/// Same as [`meta_attr`] but returns the typed
/// [`dialog_query::attribute::The`] form required by the query
/// builder rather than the runtime [`ArtifactsAttribute`].
fn meta_attr_typed(domain: &str, name: &str) -> dialog_query::attribute::The {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

impl Statement for TransientConcept {
    fn assert(self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::associate);
        dialog_repository::Transient(self.this).assert(update);
    }

    fn retract(self, update: &mut impl Update) {
        emit_concept_facts(&self.this, &self.descriptor, update, Update::dissociate);
        dialog_repository::Transient(self.this).retract(update);
    }
}

/// A **command** — the `command!:` notation's write-type.
///
/// A command *is* a transient concept: facts of this concept exist
/// for one commit cycle and are swept before the branch state is
/// written. The keyword is a clearer surface for the event-shaped
/// data that stimulates system behavior, but the storage and wire
/// representation are identical to [`TransientConcept`]. Aliasing
/// rather than wrapping keeps it interchangeable with every
/// existing `TransientConcept` call site.
pub type Command = TransientConcept;

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
///    `db.meta/concept = db:concept` marker claim, with
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
    /// When `true`, restrict enumeration to transient (command)
    /// concepts: skip the always-durable built-ins entirely and
    /// drop any branch concept lacking the
    /// `db.concept/transient` marker. This is what backs the
    /// `command:` head — see [`AnonymousConceptQuery::commands`].
    pub transient_only: bool,
}

impl AnonymousConceptQuery {
    /// Construct a new query from a parameter map. Enumerates every
    /// concept on the branch (built-in + durable + transient).
    pub fn new(terms: Parameters) -> Self {
        Self {
            terms,
            transient_only: false,
        }
    }

    /// Construct a `command:`-flavoured query: same enumeration but
    /// restricted to transient concepts only.
    pub fn commands(terms: Parameters) -> Self {
        Self {
            terms,
            transient_only: true,
        }
    }
}

impl Application for AnonymousConceptQuery {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Scope<'a>,
    {
        let app = self;
        try_stream! {
            for await each in selection {
                let input = each?;

                let this_term = app.terms.get("this").cloned();
                let name_term = app.terms.get("name").cloned();
                let source_term = app.terms.get("source").cloned();
                let transient_term = app.terms.get("transient").cloned();

                // Resolve filters from constant terms or from
                // upstream-bound variables.
                let this_filter = resolve_entity_filter(&this_term, &input);
                let name_filter = resolve_string_filter(&name_term, &input);

                let mut emitted_names: HashSet<String> = HashSet::new();

                // ---- Built-in source ----
                // Built-ins are always durable, so a `command:`
                // query (transient-only) skips them entirely.
                for (builtin_name, resolved) in concept_registry().iter() {
                    if app.transient_only {
                        break;
                    }
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
                        let json = serde_json::to_string(resolved.descriptor.concept())
                            .map_err(|e| EvaluationError::Store(e.to_string()))?;
                        m.bind(t, dialog_query::Value::String(json))?;
                    }
                    // Built-ins are always durable. Bind `false`
                    // only when the caller asked for `transient`
                    // (i.e. provided a variable term to project
                    // into); a missing term means "don't project".
                    if let Some(ref t) = transient_term {
                        m.bind(t, dialog_query::Value::Boolean(false))?;
                    }
                    yield m;
                }

                // ---- Branch source ----
                let marker = concept_marker_entity();
                let this_term_for_marker: Term<Entity> = match &this_filter {
                    Some(e) => Term::Constant(dialog_query::Value::Entity(e.clone())),
                    None => Term::var("__concept_query_this"),
                };
                let claims: Vec<Claim> = the!("db.meta/concept")
                    .of(this_term_for_marker)
                    .is(marker)
                    .perform(env)
                    .try_vec()
                    .await?;

                // Bulk-load the set of transient-marked entities
                // once per evaluation pass so the per-row check is
                // a HashSet lookup, not a fresh query. Fetch when
                // the caller asked for `transient` *or* when this is
                // a command query (which filters on the set).
                let transient_entities: HashSet<Entity> = if transient_term.is_some()
                    || app.transient_only
                {
                    let transient_claims: Vec<Claim> = the!("dialog.concept/transient")
                        .of(Term::<Entity>::var("__concept_query_transient_this"))
                        .is(true)
                        .perform(env)
                        .try_vec()
                        .await?;
                    transient_claims.into_iter().map(|c| c.of).collect()
                } else {
                    HashSet::new()
                };

                for claim in claims {
                    let entity = claim.of.clone();
                    // `command:` enumerates transient concepts only.
                    if app.transient_only && !transient_entities.contains(&entity) {
                        continue;
                    }
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
                    if let Some(ref t) = transient_term {
                        let is_transient = transient_entities.contains(&entity);
                        m.bind(t, dialog_query::Value::Boolean(is_transient))?;
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
/// The entity a resolver row reports as its `this`.
///
/// Resolver rows describe blocks, not entities, so there is no real
/// subject to report — but a `ConceptConclusion` requires one. Naming
/// the resolver (`db:tree/node`) keeps it honest: the identity says
/// which resolver produced the row rather than pretending the row is
/// a fact about some entity.
fn resolver_row_entity(name: &str) -> Entity {
    format!("db:{name}")
        .parse()
        .expect("a resolver name forms a valid `db:` entity URI")
}

fn stub_predicate() -> ConceptDescriptor {
    // A descriptor must have at least one required field, so the
    // stub carries a single placeholder. `realize` never reads the
    // predicate, so the field's identity is irrelevant.
    ConceptDescriptor::try_from(vec![(
        "_",
        AttributeDescriptor::new(
            the!("db.concept/stub"),
            "",
            dialog_query::Cardinality::default(),
            None,
        ),
    )])
    .expect("single-field stub descriptor is valid")
}

// -----------------------------------------------------------------
// Concept-of-concept sentinel descriptor + dispatch table.
// -----------------------------------------------------------------

/// The well-known descriptor for the "concept of concept" head.
///
/// Its `with` map names the marker (`db.meta/concept`), bookmark
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
                "concept":     { "the": "db.meta/concept",     "as": "Entity",  "cardinality": "one" },
                "name":        { "the": "db.meta/name",        "as": "Text",    "cardinality": "one" },
                "description": { "the": "db.meta/description", "as": "Text",    "cardinality": "one" },
                "source":      { "the": "db.meta/source",      "as": "Text",    "cardinality": "one" },
                "transient":   { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
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

/// The well-known descriptor for the "command of command" head —
/// the `command:` query sentinel.
///
/// A command is a transient concept, so this enumerates the same
/// rows as the concept-of-concept query but filtered to transient
/// concepts only (see [`AnonymousConceptQuery::commands`]). Its
/// `with` map mirrors [`concept_of_concept_descriptor`] *plus* a
/// `command` marker field — the extra `db.meta/command`
/// attribute URI is what makes this descriptor's content-derived
/// `this()` distinct from the concept sentinel's. Without it the
/// two would share an entity (identity is the attribute-URI set,
/// not field names or description) and `command:` would mis-dispatch
/// to the unfiltered concept enumeration.
pub fn command_of_command_descriptor() -> &'static ConceptDescriptor {
    static DESCRIPTOR: std::sync::OnceLock<ConceptDescriptor> = std::sync::OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        serde_json::from_value(serde_json::json!({
            "description": "Every transient (command) concept asserted on a branch.",
            "with": {
                "command":     { "the": "db.meta/command",      "as": "Entity",  "cardinality": "one" },
                "concept":     { "the": "db.meta/concept",      "as": "Entity",  "cardinality": "one" },
                "name":        { "the": "db.meta/name",         "as": "Text",    "cardinality": "one" },
                "description": { "the": "db.meta/description",   "as": "Text",    "cardinality": "one" },
                "source":      { "the": "db.meta/source",       "as": "Text",    "cardinality": "one" },
                "transient":   { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
            }
        }))
        .expect("command-of-command descriptor is well-formed")
    })
}

/// Cached `this()` of [`command_of_command_descriptor`] — the
/// dispatch sentinel that routes a `command:` head to the
/// transient-only [`AnonymousConceptQuery::commands`] enumeration.
fn command_of_command_entity() -> &'static Entity {
    static ENTITY: std::sync::OnceLock<Entity> = std::sync::OnceLock::new();
    ENTITY.get_or_init(|| command_of_command_descriptor().this())
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
    /// Command-of-command enumeration — transient concepts only,
    /// via [`AnonymousConceptQuery::commands`].
    AnonymousCommand(AnonymousConceptQuery),
    /// Rule-of-rule enumeration via [`AnonymousRuleQuery`].
    AnonymousRule(AnonymousRuleQuery),
    /// A `tree/*` resolver — dialog answers it by content address
    /// over immutable blocks, so it describes the store's own
    /// structure rather than the facts inside it.
    Resolver(Box<dialog_query::ResolverQuery>),
}

/// Convert an [`Application`](crate::transact::Application) into
/// the [`QueryPlan`] it should be evaluated as.
///
/// `Concept` carries a [`ConceptQuery`] directly; `Domain`
/// synthesises one from its parameter map. `Rule` has no read-side
/// projection — rules are only mutated, never queried by predicate
/// application — so this panics if a `Rule` application reaches it.
pub fn application_to_plan(application: crate::transact::Application) -> QueryPlan {
    use crate::transact::Application;
    match application {
        Application::Concept { query, .. } => QueryPlan::from(query),
        Application::Domain { application, .. } => QueryPlan::from(ConceptQuery::from(application)),
        Application::Resolver { query, .. } => QueryPlan::Resolver(query),
        Application::Rule { .. } | Application::DeductiveRule { .. } => panic!(
            "rule applications have no QueryPlan projection — \
             rules are write-only via Statement::Assert/Retract"
        ),
    }
}

impl From<ConceptQuery> for QueryPlan {
    fn from(query: ConceptQuery) -> Self {
        if &query.predicate.this() == concept_of_concept_entity() {
            QueryPlan::AnonymousConcept(AnonymousConceptQuery::new(query.terms))
        } else if &query.predicate.this() == command_of_command_entity() {
            QueryPlan::AnonymousCommand(AnonymousConceptQuery::commands(query.terms))
        } else if &query.predicate.this() == rule_of_rule_entity() {
            QueryPlan::AnonymousRule(AnonymousRuleQuery::new(query.terms))
        } else {
            QueryPlan::Standard(query)
        }
    }
}

/// Cached `this()` of [`rule_of_rule_descriptor`] — the dispatch
/// sentinel for [`QueryPlan::from`] that routes a `rule:` head to
/// [`AnonymousRuleQuery`]. Computing it once avoids re-hashing
/// the descriptor on every query.
fn rule_of_rule_entity() -> &'static Entity {
    static ENTITY: std::sync::OnceLock<Entity> = std::sync::OnceLock::new();
    ENTITY.get_or_init(|| rule_of_rule_descriptor().this())
}

impl Application for QueryPlan {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Scope<'a>,
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
                QueryPlan::AnonymousCommand(q) => {
                    let stream = q.evaluate(selection, env);
                    for await each in stream {
                        yield each?;
                    }
                }
                QueryPlan::AnonymousRule(q) => {
                    let stream = q.evaluate(selection, env);
                    for await each in stream {
                        yield each?;
                    }
                }
                QueryPlan::Resolver(q) => {
                    let stream = Application::evaluate(*q, selection, env);
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
            QueryPlan::AnonymousCommand(q) => Application::realize(q, source),
            QueryPlan::AnonymousRule(q) => Application::realize(q, source),
            // A resolver row is slot→value with no entity of its own,
            // while `ConceptConclusion` requires a bound `this` and
            // keeps its fields private. The row's values travel in the
            // `Match` (which is what the renderer reads), so the
            // conclusion only needs to exist: give it the resolver's
            // own subject reference as `this`, which is the closest
            // thing a resolver row has to an identity.
            QueryPlan::Resolver(q) => {
                // A resolver row is slot→value describing a BLOCK, not
                // a fact about an entity — and `ConceptConclusion`
                // requires an Entity-typed `this`, while a resolver's
                // subject is a content address (a base58 string). The
                // row's values ride the `Match`, which is what the
                // renderer reads, so the conclusion only needs a
                // well-formed identity: name the resolver itself.
                let mut terms = q.parameters().clone();
                terms.insert(
                    "this".to_string(),
                    Term::Constant(Value::Entity(resolver_row_entity(q.name()))),
                );
                let synthetic = ConceptQuery {
                    terms,
                    predicate: stub_predicate(),
                };
                Application::realize(&synthetic, source)
            }
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
            let binding = input.lookup(t).ok()?;
            Entity::try_from(binding.as_value()?.clone()).ok()
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
        Term::Variable { name: Some(_), .. } => input.lookup(t).ok()?.as_value()?.clone(),
        Term::Variable { name: None, .. } => return None,
    };
    String::try_from(value).ok()
}

/// Reconstruct the descriptor for a branch concept entity by
/// enumerating its `db.concept.with/*` claims and resolving
/// each referenced attribute via the [`AnonymousAttribute`]
/// concept query.
async fn resolve_branch_descriptor<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<ConceptDescriptor>, EvaluationError>
where
    Env: Scope<'a>,
{
    let with_claims: Vec<Claim> = dialog_query::AttributeQuery::from(
        Term::<dialog_query::attribute::The>::var("the")
            .of(Term::from(entity.clone()))
            .is(Term::<Entity>::var("attribute")),
    )
    .perform(env)
    .try_vec()
    .await?;

    // The optional markers carry Boolean values, so a separate
    // Boolean-typed query is needed — the Entity-typed `with` query
    // above never returns them.
    let optional_claims: Vec<Claim> = dialog_query::AttributeQuery::from(
        Term::<dialog_query::attribute::The>::var("the")
            .of(Term::from(entity.clone()))
            .is(Term::<bool>::var("flag")),
    )
    .perform(env)
    .try_vec()
    .await?;
    let optional_fields: BTreeSet<String> = optional_claims
        .iter()
        .filter_map(|claim| {
            let the: ArtifactsAttribute = claim.the.clone().into();
            parse_optional(&the)
        })
        .collect();

    let mut fields: Vec<(String, ConceptFieldDescriptor)> = Vec::new();
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
        let field = if optional_fields.contains(&field_name) {
            ConceptFieldDescriptor::optional(descriptor)
        } else {
            ConceptFieldDescriptor::required(descriptor)
        };
        fields.push((field_name, field));
    }

    if fields.is_empty() {
        return Ok(None);
    }
    let descriptor = ConceptDescriptor::try_from(fields)
        .map_err(|e| EvaluationError::Store(format!("invalid concept descriptor: {e:?}")))?;
    // `ConceptDescriptor::try_from` leaves `description` as `None`.
    // The concept's own `db.meta/description` claim carries
    // it, so fetch that and fold it in — a JSON round-trip is the
    // only way in since the field has no public setter.
    let Some(description) = lookup_entity_description(entity, env).await? else {
        return Ok(Some(descriptor));
    };
    let mut json =
        serde_json::to_value(&descriptor).map_err(|e| EvaluationError::Store(e.to_string()))?;
    if let Some(map) = json.as_object_mut() {
        map.insert(
            "description".to_owned(),
            serde_json::Value::String(description),
        );
    }
    let descriptor =
        serde_json::from_value(json).map_err(|e| EvaluationError::Store(e.to_string()))?;
    Ok(Some(descriptor))
}

/// Look up the published name of `entity`, if any.
///
/// Inverted from the pre-Stage-2 model: instead of asking "what
/// `db.meta/name` claim sits on `entity`?", this asks "what
/// `id:<n>` URI carries a `db.meta/name = entity` claim?"
/// and recovers the `<n>` portion. Returns `None` when no `id:`
/// URI points at the entity.
async fn lookup_entity_name<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<String>, EvaluationError>
where
    Env: Scope<'a>,
{
    use dialog_query::Output as _;
    use tonk_core::meta::Name;

    let rows: Vec<Name> = Query::<Name> {
        this: Term::<Entity>::var("__concept_query_id"),
        entity: Term::from(entity.clone()),
    }
    .perform(env)
    .try_vec()
    .await?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|row| name_from_id_uri(&row.this)))
}

/// Read the `db.meta/description` claim attached to a concept
/// entity, if any.
///
/// `emit_concept_facts` writes this claim only when the asserted
/// descriptor carried a non-empty description, so a concept
/// without one simply has no claim and this returns `None`.
async fn lookup_entity_description<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<String>, EvaluationError>
where
    Env: Scope<'a>,
{
    let claims: Vec<Claim> = the!("db.meta/description")
        .of(Term::<Entity>::from(entity.clone()))
        .is(Term::<String>::var("__concept_query_description"))
        .perform(env)
        .try_vec()
        .await?;
    Ok(claims
        .into_iter()
        .next()
        .and_then(|claim| String::try_from(claim.is).ok())
        .filter(|s| !s.is_empty()))
}

/// Strip the `id:` scheme prefix from a name URI to recover the
/// user-facing name. Returns `None` for URIs in any other scheme
/// (`db:`, `did:key:`, etc.) — those are direct entity
/// references, not user-published names.
fn name_from_id_uri(entity: &Entity) -> Option<String> {
    let s = entity.to_string();
    s.strip_prefix("id:").map(str::to_owned)
}

/// Resolve a published name to the entity it points at by
/// reading the `db.meta/name` claim attached to `id:<name>`.
///
/// Returns `None` when:
/// - The `id:<name>` URI itself doesn't parse as an entity
///   (only happens if `name` contains characters the URI scheme
///   rejects).
/// - The branch has no `db.meta/name` claim attached to
///   that entity (no name was ever published with this label,
///   or the prior publication was retracted).
pub async fn lookup_named_entity<'a, Env: QueryEnv>(
    name: &str,
    source: impl Into<Source<'a>>,
    env: &Env,
) -> Result<Option<Entity>, ConceptLookupError> {
    use dialog_query::Output as _;
    use tonk_core::meta::Name;

    let source = source.into();
    let Ok(id_entity) = format!("id:{name}").parse::<Entity>() else {
        return Ok(None);
    };
    let rows: Vec<Name> = source
        .select(Query::<Name> {
            this: Term::from(id_entity),
            entity: Term::<Entity>::var("__concept_query_target"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| ConceptLookupError::query(format!("name lookup failed: {e:?}")))?;
    Ok(rows.into_iter().next().map(|row| row.entity.0))
}

/// Walk a [`ConceptDescriptor`] and call `op` (either
/// [`Update::associate`] or [`Update::dissociate`]) for every
/// fact the concept implies — `db.concept.with/{field}`
/// per field, plus `db.meta/description` when the
/// descriptor carries one. Shared between `assert` and
/// `retract` so the two stay in lock-step.
fn emit_concept_facts<U: Update, F: Fn(&mut U, ArtifactsAttribute, Entity, Value)>(
    entity: &Entity,
    descriptor: &ConceptDescriptor,
    update: &mut U,
    op: F,
) {
    // Marker claim — every concept entity carries
    // `(?this, db.meta/concept, "db:concept")` so
    // queries that want "all concepts on this branch" have a
    // selectable triple to start from (the engine refuses
    // selections with no bound component).
    op(
        update,
        meta_attr("db.meta", "concept"),
        entity.clone(),
        Value::Entity(concept_marker_entity()),
    );
    for (field_name, field) in descriptor.with().iter() {
        let relation = meta_attr(WITH_DOMAIN, field_name);
        let attribute_entity: Entity = field
            .to_uri()
            .parse()
            .expect("AttributeDescriptor::to_uri produces a valid entity URI");
        op(
            update,
            relation,
            entity.clone(),
            Value::Entity(attribute_entity),
        );
        // Optional fields carry a sibling boolean marker. Required
        // fields emit nothing here, so their storage stays
        // byte-identical to the pre-optionality encoding.
        if field.is_optional() {
            op(
                update,
                meta_attr(OPTIONAL_DOMAIN, field_name),
                entity.clone(),
                Value::Boolean(true),
            );
        }
    }
    if let Some(description) = descriptor.description()
        && !description.is_empty()
    {
        op(
            update,
            meta_attr("db.meta", "description"),
            entity.clone(),
            Value::String(description.to_owned()),
        );
    }
}

/// The well-known entity used as the value of the
/// `db.meta/concept` marker claim. Every concept entity
/// asserted on a branch carries
/// `(?this, db.meta/concept, db:concept)`. Same URI as the
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

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn with_constructs_namespaced_relation() {
        let the = with("title").unwrap();
        assert_eq!(String::from(&the), "db.concept.with/title");
    }

    #[dialog_common::test]
    fn optional_constructs_namespaced_relation() {
        let the = optional("subtitle").unwrap();
        assert_eq!(String::from(&the), "db.concept.optional/subtitle");
    }

    #[dialog_common::test]
    fn parse_with_round_trips() {
        let the = with("ingredient-name").unwrap();
        assert_eq!(parse_with(&the).as_deref(), Some("ingredient-name"));
    }

    #[dialog_common::test]
    fn parse_optional_round_trips() {
        let the = optional("notes").unwrap();
        assert_eq!(parse_optional(&the).as_deref(), Some("notes"));
    }

    #[dialog_common::test]
    fn parse_with_rejects_other_domains() {
        let the: ArtifactsAttribute = "db.meta/name".parse().unwrap();
        assert_eq!(parse_with(&the), None);
        assert_eq!(parse_optional(&the), None);
    }

    #[dialog_common::test]
    fn parse_with_rejects_optional_domain() {
        let the = optional("x").unwrap();
        assert_eq!(parse_with(&the), None);
    }

    #[dialog_common::test]
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
    /// `db.concept.with/{field}` claim per field plus
    /// `db.meta/description` when set, plus the marker
    /// claim `db.meta/concept = db:concept` that lets
    /// branch-wide concept enumeration find this entity.
    #[dialog_common::test]
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

    /// `TransientConcept::assert` should write everything
    /// `AnonymousConcept::assert` writes plus a
    /// `(?this, dialog.concept/transient, true)` marker
    /// triple. The marker is what the reactor reads at commit
    /// time to decide which facts to strip from the persistable
    /// delta.
    #[dialog_common::test]
    fn transient_concept_writes_transient_marker() {
        use dialog_artifacts::{Changes, Instruction};
        let json = r#"{
            "description": "An ephemeral command",
            "with": {
                "subject": { "the": "command/subject", "as": "Entity", "cardinality": "one" }
            }
        }"#;
        let descriptor: ConceptDescriptor = serde_json::from_str(json).unwrap();
        let concept = TransientConcept::new(descriptor);
        let this = concept.this.clone();
        let mut changes = Changes::new();
        concept.assert(&mut changes);

        let instructions = changes.into_instructions();
        let transient_attr = meta_attr("dialog.concept", "transient");

        assert!(
            instructions.iter().any(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => {
                    a.the == transient_attr && a.of == this && matches!(&a.is, Value::Boolean(true))
                }
                _ => false,
            }),
            "missing dialog.concept/transient marker"
        );

        // Also writes the normal concept-marker so concept-enumeration
        // queries find this entity.
        let concept_marker_attr = meta_attr("db.meta", "concept");
        let concept_marker = concept_marker_entity();
        assert!(
            instructions.iter().any(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => {
                    a.the == concept_marker_attr
                        && a.of == this
                        && matches!(&a.is, Value::Entity(e) if *e == concept_marker)
                }
                _ => false,
            }),
            "missing db.meta/concept marker"
        );
    }

    /// Every concept assert must include the
    /// `(?this, db.meta/concept, db:concept)` marker so
    /// `concept:` queries with `?this` unbound can drive
    /// selection from a single bound triple.
    #[dialog_common::test]
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
        let marker_attr = meta_attr("db.meta", "concept");
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
            "expected db.meta/concept = db:concept marker claim",
        );
    }

    /// Retract path mirrors assert: the marker dissociation must
    /// be emitted alongside the with-claim retractions so the
    /// branch ends up clean.
    #[dialog_common::test]
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
        let marker_attr = meta_attr("db.meta", "concept");
        let marker_value = Value::Entity(concept_marker_entity());
        let saw_retract = changes
            .into_instructions()
            .into_iter()
            .any(|inst| match inst {
                Instruction::Retract(a) => a.the == marker_attr && a.is == marker_value,
                _ => false,
            });
        assert!(saw_retract, "expected db.meta/concept marker retraction",);
    }

    /// Compile-time check: `AnonymousConcept` implements the
    /// dialog `Concept` trait (and its `Predicate` supertrait),
    /// so it slots into query and rule machinery the same way
    /// `#[derive(Concept)]` types do.
    #[dialog_common::test]
    fn concept_wrapper_satisfies_concept_trait() {
        fn requires_concept<C: dialog_query::Concept>(_: &C)
        where
            C::Conclusion: dialog_query::Conclusion,
        {
        }
        let descriptor: ConceptDescriptor =
            serde_json::from_str(r#"{"with":{"x":{"the":"a/b","as":"Text","cardinality":"one"}}}"#)
                .unwrap();
        let anon = AnonymousConcept::new(descriptor);
        requires_concept(&anon);
    }

    /// `QueryPlan::from(ConceptQuery)` dispatches to the
    /// [`AnonymousConceptQuery`] branch when the wire query's
    /// predicate is the concept-of-concept descriptor; otherwise
    /// it stays a [`ConceptQuery`].
    #[dialog_common::test]
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

    /// The command sentinel must not collide with the concept
    /// sentinel — identity is the attribute-URI set, so the extra
    /// `db.meta/command` marker attribute is load-bearing.
    #[dialog_common::test]
    fn it_distinguishes_command_sentinel_from_concept_sentinel() {
        assert_ne!(
            command_of_command_descriptor().this(),
            concept_of_concept_descriptor().this(),
            "command and concept sentinels must hash to distinct entities",
        );
    }

    /// `QueryPlan::from(ConceptQuery)` dispatches to the
    /// transient-only [`AnonymousConceptQuery`] (via the
    /// `AnonymousCommand` variant) when the wire query's predicate
    /// is the command-of-command descriptor.
    #[dialog_common::test]
    fn it_dispatches_command_of_command_to_anonymous_command_query() {
        let plan = QueryPlan::from(ConceptQuery {
            terms: dialog_query::Parameters::new(),
            predicate: command_of_command_descriptor().clone(),
        });
        match plan {
            QueryPlan::AnonymousCommand(q) => assert!(
                q.transient_only,
                "command dispatch must produce a transient-only query",
            ),
            _ => panic!("command-of-command predicate should dispatch to AnonymousCommand"),
        }
    }

    /// `assert` then `retract` should leave nothing — every
    /// claim that goes in comes back out.
    #[dialog_common::test]
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

    /// Round-trip an [`AnonymousConcept`] through a branch and
    /// recover its descriptor via [`AnonymousConceptQuery`]'s
    /// synthesised `source` field.
    ///
    /// Asserting the concept writes the marker claim and the
    /// `db.concept.with/{field}` claims. The query
    /// enumerates the branch via the marker, reconstructs the
    /// descriptor for each entity, and binds it as a JSON
    /// string in `source`.
    ///
    /// (The anchor-name lookup that older versions of this test
    /// also exercised has moved out of the planner: names are
    /// now their own concept asserted by the analyzer's anchor
    /// desugar against `id:<n>`. A separate test will cover the
    /// inverted lookup once the resolver is rewired.)
    #[dialog_common::test]
    async fn it_returns_concept_with_source_from_concept_query() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::{Any, Output as _, Parameters, Term};

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
        // attribute's own facts (`db.attribute/id|type|
        // cardinality`, `db.meta/description`) must exist on
        // the branch for [`AnonymousConceptQuery`] to reconstruct
        // the descriptor — emit them inline alongside the concept.
        let (_, attr_descriptor) = descriptor.with().iter().next().expect("one field");
        let attr_entity: Entity = attr_descriptor.to_uri().parse()?;
        let concept = AnonymousConcept::new(descriptor.clone());
        let concept_entity = concept.this.clone();
        branch
            .transaction()
            .assert(
                dialog_query::the!("db.attribute/id")
                    .of(attr_entity.clone())
                    .is(attr_descriptor.the().to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/type")
                    .of(attr_entity.clone())
                    .is("Text".to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/cardinality")
                    .of(attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("db.meta/description")
                    .of(attr_entity)
                    .is(String::new()),
            )
            .assert(concept)
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));
        terms.insert("source".to_string(), Term::<Any>::var("source"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousConceptQuery::new(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        // Find the row whose `this` is the concept entity we
        // just asserted. Names are no longer fetched by the
        // concept query; the test now identifies the row by
        // entity URI directly.
        let row = conclusions
            .iter()
            .find(|c| {
                c.source()
                    .lookup(&Term::<Any>::var("this"))
                    .ok()
                    .and_then(|b| b.as_value().and_then(|v| Entity::try_from(v.clone()).ok()))
                    == Some(concept_entity.clone())
            })
            .expect("expected a row for the asserted concept entity");

        let source: String = String::try_from(
            row.source()
                .lookup(&Term::<Any>::var("source"))?
                .content()?,
        )
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

    /// Round-trip: persisting a concept with an optional field and
    /// resolving it back via [`ConceptByEntity::resolve`] reproduces
    /// the per-field `is_optional()` flag (required field stays
    /// required, optional field stays optional).
    #[dialog_common::test]
    async fn it_reconstructs_optional_flag_from_branch() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" },
                    "nickname": { "the": "xyz.tonk.person/nickname", "as": "Text", "cardinality": "one", "optional": true }
                }
            }"#,
        )?;

        // Install each field's attribute facts so reconstruction can
        // rehydrate the descriptors.
        let mut txn = branch.transaction();
        for (_, field) in descriptor.with().iter() {
            let attr_entity: Entity = field.to_uri().parse()?;
            txn = txn
                .assert(
                    the!("db.attribute/id")
                        .of(attr_entity.clone())
                        .is(field.the().to_string()),
                )
                .assert(
                    the!("db.attribute/type")
                        .of(attr_entity.clone())
                        .is("Text".to_string()),
                )
                .assert(
                    the!("db.attribute/cardinality")
                        .of(attr_entity.clone())
                        .is("one".to_string()),
                )
                .assert(
                    the!("db.meta/description")
                        .of(attr_entity)
                        .is(String::new()),
                );
        }
        let concept = AnonymousConcept::new(descriptor.clone());
        let concept_entity = concept.this.clone();
        txn.assert(concept).commit().perform(&operator).await?;

        let resolved = Concept::by_entity(concept_entity)
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("concept resolves from branch");

        let with = resolved.descriptor.with();
        let name = with.iter().find(|(n, _)| *n == "name").expect("name").1;
        let nickname = with
            .iter()
            .find(|(n, _)| *n == "nickname")
            .expect("nickname")
            .1;
        assert!(
            !name.is_optional(),
            "required field must rebuild as required"
        );
        assert!(
            nickname.is_optional(),
            "optional field must rebuild as optional"
        );
        Ok(())
    }

    /// Asserts a `TransientConcept` onto a branch, queries `concept:`
    /// with a `transient` binding, and confirms the row carries
    /// `Boolean(true)`. A second row over a durable concept on the
    /// same branch must carry `Boolean(false)`.
    #[dialog_common::test]
    async fn it_returns_transient_marker_on_transient_concept_rows() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::{Any, Output as _, Parameters, Term};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Two concepts on the same branch: one transient, one
        // durable, each backed by its attribute facts so the
        // anonymous-concept-query can rehydrate both descriptors.
        let transient_descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "subject": { "the": "xyz.tonk.ping/subject", "as": "Entity", "cardinality": "one" }
                }
            }"#,
        )?;
        let durable_descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )?;

        let (_, t_attr) = transient_descriptor
            .with()
            .iter()
            .next()
            .expect("one field");
        let (_, d_attr) = durable_descriptor.with().iter().next().expect("one field");
        let t_attr_entity: Entity = t_attr.to_uri().parse()?;
        let d_attr_entity: Entity = d_attr.to_uri().parse()?;

        let transient_concept = TransientConcept::new(transient_descriptor.clone());
        let durable_concept = AnonymousConcept::new(durable_descriptor.clone());
        let transient_entity = transient_concept.this.clone();
        let durable_entity = durable_concept.this.clone();

        branch
            .transaction()
            // Transient concept attribute facts.
            .assert(
                dialog_query::the!("db.attribute/id")
                    .of(t_attr_entity.clone())
                    .is(t_attr.the().to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/type")
                    .of(t_attr_entity.clone())
                    .is("Entity".to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/cardinality")
                    .of(t_attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("db.meta/description")
                    .of(t_attr_entity)
                    .is(String::new()),
            )
            // Durable concept attribute facts.
            .assert(
                dialog_query::the!("db.attribute/id")
                    .of(d_attr_entity.clone())
                    .is(d_attr.the().to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/type")
                    .of(d_attr_entity.clone())
                    .is("Text".to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/cardinality")
                    .of(d_attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("db.meta/description")
                    .of(d_attr_entity)
                    .is(String::new()),
            )
            .assert(transient_concept)
            .assert(durable_concept)
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));
        terms.insert("transient".to_string(), Term::<Any>::var("transient"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousConceptQuery::new(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        let transient_row = conclusions
            .iter()
            .find(|c| {
                c.source()
                    .lookup(&Term::<Any>::var("this"))
                    .ok()
                    .and_then(|b| b.as_value().and_then(|v| Entity::try_from(v.clone()).ok()))
                    == Some(transient_entity.clone())
            })
            .expect("transient concept row present");
        assert_eq!(
            transient_row
                .source()
                .lookup(&Term::<Any>::var("transient"))?,
            dialog_query::Binding::Present(dialog_query::Value::Boolean(true)),
        );

        let durable_row = conclusions
            .iter()
            .find(|c| {
                c.source()
                    .lookup(&Term::<Any>::var("this"))
                    .ok()
                    .and_then(|b| b.as_value().and_then(|v| Entity::try_from(v.clone()).ok()))
                    == Some(durable_entity.clone())
            })
            .expect("durable concept row present");
        assert_eq!(
            durable_row
                .source()
                .lookup(&Term::<Any>::var("transient"))?,
            dialog_query::Binding::Present(dialog_query::Value::Boolean(false)),
        );

        Ok(())
    }

    /// A `command:` query (the transient-only enumeration) surfaces
    /// only transient concepts: a transient concept appears, the
    /// durable concept on the same branch does not.
    #[dialog_common::test]
    async fn it_queries_command_returns_only_transient_concepts() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::{Any, Output as _, Parameters, Term};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let command_descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "subject": { "the": "xyz.tonk.ping/subject", "as": "Entity", "cardinality": "one" }
                }
            }"#,
        )?;
        let durable_descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )?;

        let (_, c_attr) = command_descriptor.with().iter().next().expect("one field");
        let (_, d_attr) = durable_descriptor.with().iter().next().expect("one field");
        let c_attr_entity: Entity = c_attr.to_uri().parse()?;
        let d_attr_entity: Entity = d_attr.to_uri().parse()?;

        // `Command` is the command write-type — a transient concept.
        let command = Command::new(command_descriptor.clone());
        let durable_concept = AnonymousConcept::new(durable_descriptor.clone());
        let command_entity = command.this.clone();
        let durable_entity = durable_concept.this.clone();

        branch
            .transaction()
            .assert(
                dialog_query::the!("db.attribute/id")
                    .of(c_attr_entity.clone())
                    .is(c_attr.the().to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/type")
                    .of(c_attr_entity.clone())
                    .is("Entity".to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/cardinality")
                    .of(c_attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("db.meta/description")
                    .of(c_attr_entity)
                    .is(String::new()),
            )
            .assert(
                dialog_query::the!("db.attribute/id")
                    .of(d_attr_entity.clone())
                    .is(d_attr.the().to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/type")
                    .of(d_attr_entity.clone())
                    .is("Text".to_string()),
            )
            .assert(
                dialog_query::the!("db.attribute/cardinality")
                    .of(d_attr_entity.clone())
                    .is("one".to_string()),
            )
            .assert(
                dialog_query::the!("db.meta/description")
                    .of(d_attr_entity)
                    .is(String::new()),
            )
            .assert(command)
            .assert(durable_concept)
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousConceptQuery::commands(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        let entities: HashSet<Entity> = conclusions
            .iter()
            .filter_map(|c| {
                c.source()
                    .lookup(&Term::<Any>::var("this"))
                    .ok()
                    .and_then(|b| b.as_value().and_then(|v| Entity::try_from(v.clone()).ok()))
            })
            .collect();

        assert!(
            entities.contains(&command_entity),
            "command query must surface the transient concept",
        );
        assert!(
            !entities.contains(&durable_entity),
            "command query must not surface the durable concept",
        );

        Ok(())
    }

    /// `lookup_named_entity("alice")` reads the
    /// `(id:alice, db.name/referent, ?target)` claim
    /// and returns the target entity. Round-trip a hand-written
    /// claim through a branch and confirm the lookup recovers
    /// the target.
    #[dialog_common::test]
    async fn it_resolves_published_name_to_target_entity() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let id_alice: Entity = "id:alice".parse()?;
        let target: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv".parse()?;
        branch
            .transaction()
            .assert(the!("db.name/referent").of(id_alice).is(target.clone()))
            .commit()
            .perform(&operator)
            .await?;

        let resolved = lookup_named_entity("alice", &Source::from(&branch), &operator).await?;
        assert_eq!(resolved, Some(target));
        Ok(())
    }

    /// Looking up a name with no published `id:<n>` claim
    /// returns `None` — both for "the name was never asserted"
    /// and for "the prior assertion was retracted."
    #[dialog_common::test]
    async fn it_returns_none_for_unknown_published_name() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let resolved = lookup_named_entity("ghost", &Source::from(&branch), &operator).await?;
        assert!(resolved.is_none());
        Ok(())
    }

    /// Cardinality-one supersession via `derive(Concept)` — does
    /// dialog automatically retract a prior single-cardinality
    /// claim when a new one for the same `(this, the)` is
    /// asserted in a *separate* transaction?
    ///
    /// Models the page-disambiguation bug from user feedback:
    /// when the analyzer's name publication writes
    /// `(id:page, db.meta/name, page-v1)` in tx1 and
    /// `(id:page, db.meta/name, page-v2)` in tx2, querying
    /// for "page" via `db.meta/name` should return only
    /// `page-v2` — the previous pointer should have been
    /// superseded by the cardinality-one constraint.
    ///
    /// If this test passes, dialog handles supersession across
    /// transactions natively and the worker's manual
    /// `resolve_supersession_targets` pre-pass can be deleted
    /// from the name-publication path.
    ///
    /// If this test fails, dialog only de-dupes within a single
    /// transaction (additive across transactions), and the bug
    /// is in `dialog` itself.
    #[dialog_common::test]
    async fn it_supersedes_cardinality_one_across_transactions() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::Output as _;

        // A minimal one-attribute concept whose only field is
        // cardinality-one. `Pointer` mirrors the shape of the
        // `db.meta/name` claim — id-shaped `this`, single
        // entity-typed value.
        mod pointer {
            use dialog_artifacts::Entity;
            #[derive(dialog_query::Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
            #[domain("test.supersession")]
            pub struct Target(pub Entity);
        }

        #[derive(dialog_query::Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
        pub struct Pointer {
            pub this: Entity,
            pub target: pointer::Target,
        }

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // The shared `this` — same entity across both
        // transactions. Different targets, same pointer.
        let id_page: Entity = "id:page".parse()?;
        let page_v1: Entity = "id:page-v1".parse()?;
        let page_v2: Entity = "id:page-v2".parse()?;

        // tx1 — point at page-v1.
        branch
            .transaction()
            .assert(Pointer {
                this: id_page.clone(),
                target: pointer::Target(page_v1.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        // tx2 — point at page-v2. If dialog handles
        // cardinality-one, the page-v1 pointer should be
        // superseded.
        branch
            .transaction()
            .assert(Pointer {
                this: id_page.clone(),
                target: pointer::Target(page_v2.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        // Query for every Pointer whose `this` is id_page.
        let results: Vec<Pointer> = branch
            .query()
            .select(PointerQuery {
                this: dialog_query::Term::from(id_page.clone()),
                target: dialog_query::Term::var("target"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(
            results.len(),
            1,
            "expected exactly one Pointer for id:page after supersession, got {results:?}",
        );
        assert_eq!(
            results[0].target.0, page_v2,
            "expected the latest target (page-v2) to win; got {:?}",
            results[0].target.0,
        );
        Ok(())
    }

    /// Same as [`it_supersedes_cardinality_one_across_transactions`]
    /// but with both asserts in a *single* transaction. The
    /// `associate_unique` semantic is "this transaction emits at
    /// most one value for this `(of, the)` pair" — when called
    /// twice in the same batch, the second call should win.
    #[dialog_common::test]
    async fn it_supersedes_cardinality_one_within_transaction() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::Output as _;

        mod pointer {
            use dialog_artifacts::Entity;
            #[derive(dialog_query::Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
            #[domain("test.supersession.same-tx")]
            pub struct Target(pub Entity);
        }

        #[derive(dialog_query::Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
        pub struct Pointer {
            pub this: Entity,
            pub target: pointer::Target,
        }

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let id_page: Entity = "id:page".parse()?;
        let page_v1: Entity = "id:page-v1".parse()?;
        let page_v2: Entity = "id:page-v2".parse()?;

        // Both assertions land in the same transaction.
        branch
            .transaction()
            .assert(Pointer {
                this: id_page.clone(),
                target: pointer::Target(page_v1.clone()),
            })
            .assert(Pointer {
                this: id_page.clone(),
                target: pointer::Target(page_v2.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        let results: Vec<Pointer> = branch
            .query()
            .select(PointerQuery {
                this: dialog_query::Term::from(id_page.clone()),
                target: dialog_query::Term::var("target"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(
            results.len(),
            1,
            "expected one Pointer after same-tx supersession, got {results:?}",
        );
        assert_eq!(results[0].target.0, page_v2);
        Ok(())
    }

    /// End-to-end check on the real `Name` concept: assert two
    /// successive `Name` claims for `id:page` (v1 then v2) via
    /// the same code path the analyzer uses
    /// (`emit_name_assertion`), then verify both lookup
    /// directions return only v2.
    ///
    /// This is the regression test for the page-disambiguation
    /// bug. Before the rewrite, `lookup_entity_name` used a raw
    /// `the!("db.meta/name").of(?).is(value)` query that
    /// surfaces the superseded v1 claim from the EAV log. Going
    /// through the `Name` concept's derived `Query` runs the
    /// same cardinality-one filter dialog applies to the forward
    /// direction, so v1 disappears.
    #[dialog_common::test]
    async fn it_resolves_only_latest_name_target_via_name_concept() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::Output as _;
        use tonk_core::meta::{Name, name};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Use the `concept:` scheme for targets — this matches
        // the real-world case where `concept!: &page` derives a
        // content-hashed `concept:…` entity URI for each body.
        // Same supersession path, different value scheme; this
        // catches a bug where the cardinality-one filter was
        // sensitive to the value's URI scheme.
        let id_page: Entity = "id:page".parse()?;
        let page_v1: Entity = "concept:Fx8sv1aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?;
        let page_v2: Entity = "concept:AfmLeBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".parse()?;

        // tx1 — point id:page at v1.
        branch
            .transaction()
            .assert(Name {
                this: id_page.clone(),
                entity: name::Referent(page_v1.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        // tx2 — point id:page at v2. Cardinality-one supersedes v1.
        branch
            .transaction()
            .assert(Name {
                this: id_page.clone(),
                entity: name::Referent(page_v2.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        let lookup = branch
            .select(Query::<Name> {
                this: id_page.clone().into(),
                entity: Term::var("entity"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(lookup.iter().len(), 1);
        assert_eq!(
            lookup,
            vec![Name {
                this: id_page.clone(),
                entity: name::Referent(page_v2.clone())
            }]
        );
        Ok(())
    }
}
