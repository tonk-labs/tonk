//! Built-in concept registry.
//!
//! Some concepts are not user-defined facts on the branch — they
//! describe the meta-schema itself ([`attribute`], [`concept`]) or
//! repository state written by the worker as native Rust types
//! ([`branch`], [`replica`], [`remote`], [`tracking-branch`]). The
//! [`Resolver`] can't fetch them because nothing on the branch
//! tags them as concepts; instead the analyzer consults this
//! registry first and falls through to the resolver only when no
//! built-in matches.
//!
//! Built-ins win over branch-defined concepts of the same name.
//! In-document `concept!` definitions still win over built-ins
//! within their own document, so users can shadow a built-in
//! locally for testing.
//!
//! [`Resolver`]: crate::analyzer::Resolver

use std::sync::OnceLock;

use dialog_artifacts::Entity;
use dialog_query::ConceptDescriptor;

use crate::analyzer::ResolvedConcept;
use crate::{BranchQuery, RemoteQuery, ReplicaQuery, TrackingBranchQuery};

/// Look up a built-in concept by head-name. Returns `None` for
/// names that fall through to the resolver.
pub fn lookup_concept(name: &str) -> Option<ResolvedConcept> {
    REGISTRY
        .get_or_init(build_registry)
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, concept)| concept.clone())
}

/// Iterate every built-in concept as `(name, ResolvedConcept)`
/// pairs. Used by the concept-of-concept query path to surface
/// built-ins in a `concept:` query result.
pub fn concept_registry() -> &'static [(&'static str, ResolvedConcept)] {
    REGISTRY.get_or_init(build_registry).as_slice()
}

static REGISTRY: OnceLock<Vec<(&'static str, ResolvedConcept)>> = OnceLock::new();

fn build_registry() -> Vec<(&'static str, ResolvedConcept)> {
    vec![
        ("attribute", attribute_descriptor()),
        ("concept", concept_descriptor()),
        ("name", name_descriptor()),
        ("branch", concept_from_query::<BranchQuery>()),
        ("replica", concept_from_query::<ReplicaQuery>()),
        ("remote", concept_from_query::<RemoteQuery>()),
        (
            "tracking-branch",
            concept_from_query::<TrackingBranchQuery>(),
        ),
    ]
}

/// Built-in `attribute` view — anonymous attribute shape:
/// `id`, `type`, `cardinality`, `description`. Every attribute
/// (bookmark, variable, or inline in `concept!`'s `with:`) carries
/// these four claims with non-empty defaults, so the descriptor's
/// match-all-fields semantics surfaces every defined attribute.
///
/// `dialog.meta/name` is *not* part of this view: only bookmark-
/// form attributes carry that claim, and dialog's
/// [`ConceptDescriptor::maybe`] is parsed but not yet consulted
/// by the engine — moving `name` to `maybe:` would make every
/// query miss anonymous attrs. To recover the name when needed,
/// join with `dialog.meta/name` separately.
fn attribute_descriptor() -> ResolvedConcept {
    let json = serde_json::json!({
        "with": {
            "id":          { "the": "dialog.attribute/id",          "as": "Text", "cardinality": "one" },
            "type":        { "the": "dialog.attribute/type",        "as": "Text", "cardinality": "one" },
            "cardinality": { "the": "dialog.attribute/cardinality", "as": "Text", "cardinality": "one" },
            "description": { "the": "dialog.meta/description",      "as": "Text", "cardinality": "one" },
        }
    });
    let descriptor: ConceptDescriptor =
        serde_json::from_value(json).expect("attribute schema is well-formed");
    descriptor_to_resolved(descriptor)
}

/// Built-in `concept` view — the concept-of-concept descriptor.
///
/// Resolves to the sentinel descriptor whose `this()` triggers
/// dispatch to [`crate::concept::AnonymousConceptQuery`] in
/// [`crate::concept::QueryPlan::from`], so a `concept:` head at
/// query time enumerates *every* concept (built-in + branch) with
/// a synthesised `source` field.
///
/// The entity is fixed at the well-known `db:concept` URI
/// (rather than `descriptor.this()`'s content hash) so the row
/// for the concept-of-concept built-in is identifiable without
/// knowing the descriptor's hash. This is the same URI used as
/// the value of every `dialog.meta/concept` marker claim — the
/// symmetry is intentional.
fn concept_descriptor() -> ResolvedConcept {
    ResolvedConcept {
        entity: "db:concept"
            .parse()
            .expect("`db:concept` is a valid entity URI"),
        descriptor: crate::concept::concept_of_concept_descriptor().clone(),
    }
}

/// Built-in `name` view — a name entity carries an `entity:`
/// claim pointing at the entity it currently identifies.
///
/// The schema is a single-field concept whose backing attribute
/// is `dialog.meta/name` (cardinality one). User-published names
/// live at `id:<name>` URIs; built-in names live at `db:<name>`
/// URIs and aren't writable.
///
/// The concept's own entity is fixed at `db:name` (rather than a
/// content-derived hash) so the row for the name-of-name built-in
/// is identifiable by URI and the `db:` scheme protection covers
/// it.
fn name_descriptor() -> ResolvedConcept {
    let json = serde_json::json!({
        "with": {
            "entity": { "the": "dialog.meta/name", "as": "Entity", "cardinality": "one" },
        }
    });
    let descriptor: ConceptDescriptor =
        serde_json::from_value(json).expect("name schema is well-formed");
    ResolvedConcept {
        entity: "db:name".parse().expect("`db:name` is a valid entity URI"),
        descriptor,
    }
}

/// Build a `ResolvedConcept` from a `#[derive(Concept)]` Rust
/// type's `Query` newtype. The `Query` type's `Default` impl is
/// generated by the derive and the `From<Query>` conversion drops
/// values, so this is a pure schema-only path.
fn concept_from_query<Q>() -> ResolvedConcept
where
    Q: Default,
    ConceptDescriptor: From<Q>,
{
    descriptor_to_resolved(ConceptDescriptor::from(Q::default()))
}

fn descriptor_to_resolved(descriptor: ConceptDescriptor) -> ResolvedConcept {
    let entity: Entity = descriptor.this();
    ResolvedConcept { entity, descriptor }
}
