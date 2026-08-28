//! The resolution surface — chain handles that reconstruct schema
//! definitions from a source.
//!
//! `resolve` reconstructs a definition from a branch: a
//! [`ConceptReference`] resolves to a [`ConceptDefinition`], an
//! [`AttributeReference`] to an [`AttributeDefinition`]. Each lookup
//! is a chain handle — a `resolve` method stages the work against a
//! source, `.perform(env)` runs it — matching the `induce` idiom
//! used elsewhere in the runtime.
//!
//! ```ignore
//! let definition = ConceptReference::from(entity)
//!     .resolve(&branch).perform(&operator).await?;
//! // or, starting from a published name:
//! let definition = ConceptReference::from(NamedReference("person".into()))
//!     .resolve(&branch).perform(&operator).await?;
//! ```
//!
//! # Source generality
//!
//! `Branch` and `Transaction` do not implement
//! `dialog_query::Source` (an engine-internal trait carrying
//! `acquire`), so the handles cannot be generic over that trait.
//! Instead they resolve against a [`Source`] — a tonk-local enum
//! that unifies `branch.query().select(q)` and
//! `transaction.query().select(q)` behind one `select` method (see
//! [`crate::query_source`]). A caller resolves against a branch or a
//! transaction overlay by passing whichever — both `&Branch` and
//! `&Transaction` are `Into<Source>`.

pub use dialog_artifacts::Entity;

use crate::concept::{
    AttributeByEntity, Concept, ConceptLookupError, QueryEnv, TransientConcept, lookup_named_entity,
};
use crate::query_source::Source;
use tonk_core::claim::ConceptDescriptor;
use tonk_core::meta::Name;

pub use dialog_query::AttributeDescriptor;

/// Errors raised while resolving a reference to a definition.
pub type ResolveError = ConceptLookupError;

// -----------------------------------------------------------------
// References — names of things to resolve.
// -----------------------------------------------------------------

/// A published name — `id:<n>` carries a `db.name/referent`
/// claim pointing at whatever the name currently identifies.
///
/// Distinct from the [`Name`] concept schema type: this is the
/// reference (the `<n>` string), not the stored claim. It is the
/// home for the name → entity lookup and a `From` source for the
/// typed references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedReference(pub String);

/// Either side of a [`ConceptReference`] / [`AttributeReference`] —
/// a direct entity or a name to look up. Private; callers construct
/// the typed references via `From` and never match a variant.
#[derive(Debug, Clone)]
enum Target {
    /// A concept or attribute entity, used directly.
    Entity(Entity),
    /// A published name, resolved to an entity first.
    Named(NamedReference),
}

/// Names a concept — by entity, or by published name. Constructed
/// via [`From`], so the caller never matches a variant.
#[derive(Debug, Clone)]
pub struct ConceptReference(Target);

impl From<Entity> for ConceptReference {
    fn from(entity: Entity) -> Self {
        Self(Target::Entity(entity))
    }
}

impl From<NamedReference> for ConceptReference {
    fn from(name: NamedReference) -> Self {
        Self(Target::Named(name))
    }
}

impl ConceptReference {
    /// Stage resolution of this reference against a source — a
    /// `&Branch` or a `&Transaction` (anything `Into<Source>`).
    pub fn resolve<'a>(self, source: impl Into<Source<'a>>) -> ResolveConcept<'a> {
        ResolveConcept {
            source: source.into(),
            reference: self,
        }
    }
}

/// Names an attribute — by entity, or by published name. Same shape
/// as [`ConceptReference`]; per-kind types keep resolution
/// type-correct, so an attribute entity cannot be resolved as a
/// concept.
#[derive(Debug, Clone)]
pub struct AttributeReference(Target);

impl From<Entity> for AttributeReference {
    fn from(entity: Entity) -> Self {
        Self(Target::Entity(entity))
    }
}

impl From<NamedReference> for AttributeReference {
    fn from(name: NamedReference) -> Self {
        Self(Target::Named(name))
    }
}

impl AttributeReference {
    /// Stage resolution of this reference against a source — a
    /// `&Branch` or a `&Transaction` (anything `Into<Source>`).
    pub fn resolve<'a>(self, source: impl Into<Source<'a>>) -> ResolveAttribute<'a> {
        ResolveAttribute {
            source: source.into(),
            reference: self,
        }
    }
}

// -----------------------------------------------------------------
// Definitions — a descriptor reconstructed from a source.
// -----------------------------------------------------------------

/// A concept descriptor reconstructed from a source, paired with
/// the entity it was reconstructed from.
///
/// `descriptor` is tonk's durability-tagged [`ConceptDescriptor`]:
/// [`resolve`](ConceptReference::resolve)'s `perform` reconstructs
/// the dialog field-set from the entity's EAV facts and reads the
/// `dialog.concept/transient` marker to pick the `Durable` /
/// `Transient` variant.
#[derive(Debug, Clone)]
pub struct ConceptDefinition {
    /// The concept entity URI.
    pub entity: Entity,
    /// The reconstructed, durability-tagged descriptor.
    pub descriptor: ConceptDescriptor,
}

/// An attribute descriptor reconstructed from a source, paired with
/// the entity it was reconstructed from. Attributes have no
/// durability, so `descriptor` is dialog's plain
/// [`AttributeDescriptor`].
#[derive(Debug, Clone)]
pub struct AttributeDefinition {
    /// The attribute entity URI.
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: AttributeDescriptor,
}

// -----------------------------------------------------------------
// Resolve chain handles.
// -----------------------------------------------------------------

/// Staged concept resolution — call [`perform`](Self::perform) to
/// run it against an environment.
pub struct ResolveConcept<'a> {
    source: Source<'a>,
    reference: ConceptReference,
}

impl ResolveConcept<'_> {
    /// Resolve the reference to a [`ConceptDefinition`].
    ///
    /// A name reference is resolved to an entity first
    /// (`db.name/referent` on `id:<n>`); the entity is then
    /// reconstructed by enumerating its `db.concept.with/*`
    /// claims, resolving each field attribute, and reading the
    /// `dialog.concept/transient` marker to tag durability.
    ///
    /// Returns `None` when the name has no published claim, or the
    /// entity carries no concept facts.
    pub async fn perform<Env: QueryEnv>(
        self,
        env: &Env,
    ) -> Result<Option<ConceptDefinition>, ResolveError> {
        let Some(entity) = resolve_target(self.reference.0, &self.source, env).await? else {
            return Ok(None);
        };

        let Some(concept) = Concept::by_entity(entity.clone())
            .resolve(&self.source, env)
            .await?
        else {
            return Ok(None);
        };

        let transient = TransientConcept::is_transient(entity)
            .resolve(&self.source, env)
            .await?;
        let descriptor = if transient {
            ConceptDescriptor::Transient(concept.descriptor)
        } else {
            ConceptDescriptor::Durable(concept.descriptor)
        };

        Ok(Some(ConceptDefinition {
            entity: concept.entity,
            descriptor,
        }))
    }
}

/// Staged attribute resolution — call [`perform`](Self::perform) to
/// run it against an environment.
pub struct ResolveAttribute<'a> {
    source: Source<'a>,
    reference: AttributeReference,
}

impl ResolveAttribute<'_> {
    /// Resolve the reference to an [`AttributeDefinition`].
    ///
    /// A name reference is resolved to an entity first; the entity
    /// is then reconstructed from its `db.attribute/*` facts.
    /// An entity that carries no attribute facts but does carry a
    /// published-name referent (an `id:<name>` URI used directly)
    /// is chased one step: the referent is the attribute, exactly
    /// as if the reference had been the bare name. Returns `None`
    /// when neither yields attribute facts.
    pub async fn perform<Env: QueryEnv>(
        self,
        env: &Env,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        let Some(entity) = resolve_target(self.reference.0, &self.source, env).await? else {
            return Ok(None);
        };

        let attribute = match AttributeByEntity::new(entity.clone())
            .resolve(&self.source, env)
            .await?
        {
            Some(attribute) => attribute,
            None => {
                let Some(referent) = lookup_referent(&entity, &self.source, env).await? else {
                    return Ok(None);
                };
                let Some(attribute) = AttributeByEntity::new(referent)
                    .resolve(&self.source, env)
                    .await?
                else {
                    return Ok(None);
                };
                attribute
            }
        };

        Ok(Some(AttributeDefinition {
            entity: attribute.entity,
            descriptor: attribute.descriptor,
        }))
    }
}

/// Read the `db.name/referent` claim attached to `entity`, if
/// any — the one-step name indirection behind `id:<name>` URIs.
async fn lookup_referent<Env: QueryEnv>(
    entity: &Entity,
    source: &Source<'_>,
    env: &Env,
) -> Result<Option<Entity>, ResolveError> {
    use dialog_query::{Output as _, Query, Term};
    let rows: Vec<Name> = source
        .select(Query::<Name> {
            this: Term::from(entity.clone()),
            entity: Term::<Entity>::var("__referent"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| ResolveError::query(format!("referent lookup failed: {e:?}")))?;
    Ok(rows.into_iter().next().map(|row| row.entity.0))
}

/// Resolve a [`Target`] to the entity it names — a direct entity
/// passes through, a name is chased through `db.name/referent`.
async fn resolve_target<Env: QueryEnv>(
    target: Target,
    source: &Source<'_>,
    env: &Env,
) -> Result<Option<Entity>, ResolveError> {
    match target {
        Target::Entity(entity) => Ok(Some(entity)),
        Target::Named(NamedReference(name)) => lookup_named_entity(&name, source, env).await,
    }
}

// -----------------------------------------------------------------
// Enumeration — `list` answers "all of them".
// -----------------------------------------------------------------

impl ConceptDefinition {
    /// Stage enumeration of every concept the source holds — a
    /// `&Branch` or a `&Transaction` (anything `Into<Source>`).
    pub fn list<'a>(source: impl Into<Source<'a>>) -> ListConcepts<'a> {
        ListConcepts {
            source: source.into(),
        }
    }
}

impl NamedReference {
    /// Stage enumeration of every published name and the entity it
    /// points at — against a `&Branch` or a `&Transaction`.
    pub fn list<'a>(source: impl Into<Source<'a>>) -> ListNames<'a> {
        ListNames {
            source: source.into(),
        }
    }
}

/// Staged concept enumeration — call [`perform`](Self::perform) to
/// run it.
pub struct ListConcepts<'a> {
    source: Source<'a>,
}

impl ListConcepts<'_> {
    /// Resolve every concept the source holds.
    ///
    /// Enumerates the branch via the `db.meta/concept` marker,
    /// then resolves each entity through
    /// [`ConceptReference::resolve`] so every returned definition
    /// carries its durability tag.
    pub async fn perform<Env: QueryEnv>(
        self,
        env: &Env,
    ) -> Result<Vec<ConceptDefinition>, ResolveError> {
        use dialog_query::{AttributeQuery, Output as _, Term, attribute::The};

        let marker: Entity = "db:concept"
            .parse()
            .expect("`db:concept` is a valid entity URI");
        let claims: Vec<dialog_query::Claim> = self
            .source
            .select(AttributeQuery::from(
                Term::<The>::from(
                    "db.meta/concept"
                        .parse::<The>()
                        .expect("`db.meta/concept` is a valid attribute"),
                )
                .of(Term::<Entity>::var("concept"))
                .is(Term::from(marker)),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ResolveError::query(format!("concept enumeration failed: {e:?}")))?;

        let mut definitions = Vec::with_capacity(claims.len());
        for claim in claims {
            if let Some(definition) = ConceptReference::from(claim.of)
                .resolve(self.source.clone())
                .perform(env)
                .await?
            {
                definitions.push(definition);
            }
        }
        Ok(definitions)
    }
}

/// Staged published-name enumeration — call
/// [`perform`](Self::perform) to run it.
pub struct ListNames<'a> {
    source: Source<'a>,
}

impl ListNames<'_> {
    /// Resolve every published name and the entity it points at.
    ///
    /// Yields [`Name`] — the concept that models a name → target
    /// binding — not definitions: a published name can point at
    /// any entity, not only a concept.
    pub async fn perform<Env: QueryEnv>(self, env: &Env) -> Result<Vec<Name>, ResolveError> {
        use dialog_query::{Output as _, Query, Term};

        let names: Vec<Name> = self
            .source
            .select(Query::<Name> {
                this: Term::<Entity>::var("name"),
                entity: Term::<Entity>::var("entity"),
            })
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| ResolveError::query(format!("name enumeration failed: {e:?}")))?;
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concept::{AnonymousConcept, TransientConcept};
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_query::ConceptDescriptor as DialogConceptDescriptor;
    use dialog_query::the;
    use tonk_core::meta::name;

    /// Assert a concept's backing attribute facts plus the concept
    /// itself onto a branch. Mirrors the inline emission used by
    /// `concept.rs`'s own round-trip test.
    async fn seed_concept<Env>(
        branch: &dialog_repository::Branch,
        operator: &Env,
        descriptor: &DialogConceptDescriptor,
        transient: bool,
    ) -> anyhow::Result<Entity>
    where
        Env: QueryEnv
            + dialog_capability::Provider<dialog_effects::memory::Publish>
            + dialog_capability::Provider<dialog_effects::archive::Import>
            + dialog_capability::Provider<dialog_effects::authority::Attest>,
    {
        let txn = branch.transaction();
        let mut txn = txn;
        for (_, attr) in descriptor.with().iter() {
            let attr_entity: Entity = attr.to_uri().parse()?;
            txn = txn
                .assert(
                    the!("db.attribute/id")
                        .of(attr_entity.clone())
                        .is(attr.the().to_string()),
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

        let entity = descriptor.this();
        if transient {
            txn = txn.assert(TransientConcept::new(descriptor.clone()));
        } else {
            txn = txn.assert(AnonymousConcept::new(descriptor.clone()));
        }
        txn.commit().perform(operator).await?;
        Ok(entity)
    }

    fn descriptor() -> DialogConceptDescriptor {
        serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )
        .expect("well-formed descriptor")
    }

    #[dialog_common::test]
    async fn it_resolves_a_concept_by_entity() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor = descriptor();
        let entity = seed_concept(&branch, &operator, &descriptor, false).await?;

        let definition = ConceptReference::from(entity.clone())
            .resolve(&branch)
            .perform(&operator)
            .await?
            .expect("concept resolves");

        assert_eq!(definition.entity, entity);
        assert!(matches!(
            definition.descriptor,
            ConceptDescriptor::Durable(_)
        ));
        assert_eq!(definition.descriptor.concept().with().iter().count(), 1);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_a_concept_by_name() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor = descriptor();
        let entity = seed_concept(&branch, &operator, &descriptor, false).await?;

        // Publish the name `person` pointing at the concept entity.
        let id_person: Entity = "id:person".parse()?;
        branch
            .transaction()
            .assert(Name {
                this: id_person,
                entity: name::Referent(entity.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        let definition = ConceptReference::from(NamedReference("person".into()))
            .resolve(&branch)
            .perform(&operator)
            .await?
            .expect("named concept resolves");

        assert_eq!(definition.entity, entity);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_a_transient_concept_to_the_transient_variant() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor: DialogConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "subject": { "the": "command/subject", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )?;
        let entity = seed_concept(&branch, &operator, &descriptor, true).await?;

        let definition = ConceptReference::from(entity)
            .resolve(&branch)
            .perform(&operator)
            .await?
            .expect("transient concept resolves");

        assert!(
            matches!(definition.descriptor, ConceptDescriptor::Transient(_)),
            "transient-marked concept must resolve to ConceptDescriptor::Transient",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_none_for_an_unknown_named_concept() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let definition = ConceptReference::from(NamedReference("ghost".into()))
            .resolve(&branch)
            .perform(&operator)
            .await?;
        assert!(definition.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lists_every_concept_on_the_branch() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let descriptor = descriptor();
        let entity = seed_concept(&branch, &operator, &descriptor, false).await?;

        let concepts = ConceptDefinition::list(&branch).perform(&operator).await?;
        assert!(
            concepts.iter().any(|c| c.entity == entity),
            "listed concepts must include the seeded entity",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lists_every_published_name() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let target: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv".parse()?;
        let id_alice: Entity = "id:alice".parse()?;
        branch
            .transaction()
            .assert(Name {
                this: id_alice.clone(),
                entity: name::Referent(target.clone()),
            })
            .commit()
            .perform(&operator)
            .await?;

        let names = NamedReference::list(&branch).perform(&operator).await?;
        assert!(
            names
                .iter()
                .any(|n| n.this == id_alice && n.entity.0 == target),
            "listed names must include the published `alice` binding",
        );
        Ok(())
    }
}
