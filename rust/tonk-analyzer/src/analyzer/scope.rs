//! [`Scope`] — layered name index built during analysis.
//!
//! Holds the live source the analyzer resolves against. env is
//! *not* stored, it is passed to each resolve_* call so the
//! resolution chain takes it at `.perform` time (the dialog
//! idiom). Source is a borrowed handle to a branch / txn that
//! lives for the analyze call; env is a per-execution context.

use std::collections::HashMap;

use dialog_artifacts::Entity;
use parking_lot::Mutex;

use super::error::{AnalyzeError, AnalyzeErrorKind};
use tonk_schema::concept::{QueryEnv, lookup_named_entity};
use tonk_schema::query_source::Source;
use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, NamedReference,
    ResolveError,
};
use tonk_schema::rule::{Rule, RuleResolveError};

/// Layered name index built during analysis.
///
/// Each map is wrapped in a `parking_lot::Mutex` so the analyzer
/// can mutate the scope from inside `&self` methods (the same
/// scope is shared across the analyzer's phases). The guards are
/// `Send`, so axum handlers stay happy on native; on wasm the
/// runtime is single-threaded and the lock is uncontended.
/// Critical sections never cross an `.await`: `lookup_entity` and
/// the prefetch helpers drop their guards before recursing into
/// the branch resolution chain.
pub(crate) struct Scope<'a> {
    source: Source<'a>,
    /// Anchor/variable → entity for non-meta head bindings
    /// (every head except `attribute!` / `concept!` whose
    /// declarations live in the dedicated maps below). One map
    /// per source (anchor vs variable), surfaced separately
    /// because `Analysis` keeps them separate.
    pub(crate) declarations: Mutex<HashMap<String, Entity>>,
    pub(crate) variables: Mutex<HashMap<String, Entity>>,
    /// `attribute!` definitions made in the document, indexed by
    /// the anchor/variable name on the head. Used by later
    /// `concept!` heads in the same document so their `with:`
    /// map can resolve bare-symbol / `?var` references against
    /// uncommitted attributes.
    pub(crate) in_doc_attributes: Mutex<HashMap<String, AttributeDefinition>>,
    /// `concept!` definitions made in the document, indexed by
    /// the anchor/variable name on the head. Used by later
    /// `person!: &alice` heads in the same document.
    pub(crate) in_doc_concepts: Mutex<HashMap<String, ConceptDefinition>>,
    /// Reverse index: attribute entity → resolved attribute.
    /// Used when a concept body references an attribute via URI
    /// instead of by name.
    pub(crate) in_doc_attributes_by_entity: Mutex<HashMap<String, AttributeDefinition>>,
    /// Reverse index: concept entity → resolved concept.
    pub(crate) in_doc_concepts_by_entity: Mutex<HashMap<String, ConceptDefinition>>,
    /// Name → entity for symbols prefetched from the branch that
    /// resolved to a plain named entity (not a concept or
    /// attribute). Populated by
    /// [`prefetch_named_entity`](Self::prefetch_named_entity).
    pub(crate) named_entities: Mutex<HashMap<String, Entity>>,
    /// Entity → installed [`Rule`] read off the branch for a
    /// `rule!: ..: _` retract. Populated by
    /// [`prefetch_rule`](Self::prefetch_rule) during the resolve
    /// pass so the retract lowering reads it synchronously. A key
    /// present with no entry means the entity holds no installed
    /// rule (retracting something absent).
    pub(crate) resolved_rules: Mutex<HashMap<String, Option<Rule>>>,
}

impl<'a> Scope<'a> {
    pub(crate) fn new(source: Source<'a>) -> Self {
        Self {
            source,
            declarations: Mutex::new(HashMap::new()),
            variables: Mutex::new(HashMap::new()),
            in_doc_attributes: Mutex::new(HashMap::new()),
            in_doc_concepts: Mutex::new(HashMap::new()),
            in_doc_attributes_by_entity: Mutex::new(HashMap::new()),
            in_doc_concepts_by_entity: Mutex::new(HashMap::new()),
            named_entities: Mutex::new(HashMap::new()),
            resolved_rules: Mutex::new(HashMap::new()),
        }
    }

    /// Record an anchor-form head's entity.
    pub(crate) fn declare(
        &self,
        name: &str,
        entity: Entity,
        range: lsp_types::Range,
    ) -> Result<(), AnalyzeError> {
        if self.variables.lock().contains_key(name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::NameShadowing {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        let prior = self.declarations.lock().insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::DuplicateName {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        Ok(())
    }

    /// Record a variable-form head's entity.
    pub(crate) fn bind_variable(
        &self,
        name: &str,
        entity: Entity,
        range: lsp_types::Range,
    ) -> Result<(), AnalyzeError> {
        if self.declarations.lock().contains_key(name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::NameShadowing {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        let prior = self.variables.lock().insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::DuplicateName {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        Ok(())
    }

    /// Record an in-document `attribute!` definition for the
    /// given declaration / variable name.
    pub(crate) fn record_attribute(&self, name: Option<&str>, attribute: AttributeDefinition) {
        if let Some(name) = name {
            self.in_doc_attributes
                .lock()
                .insert(name.to_owned(), attribute.clone());
        }
        self.in_doc_attributes_by_entity
            .lock()
            .insert(attribute.entity.to_string(), attribute);
    }

    /// Record an in-document `concept!` definition.
    pub(crate) fn record_concept(&self, name: Option<&str>, concept: ConceptDefinition) {
        if let Some(name) = name {
            self.in_doc_concepts
                .lock()
                .insert(name.to_owned(), concept.clone());
        }
        self.in_doc_concepts_by_entity
            .lock()
            .insert(concept.entity.to_string(), concept);
    }

    /// Look up the entity bound to an anchor or `?var` name,
    /// regardless of which side it lives on. Returns `None` if
    /// the name isn't known yet.
    pub(crate) fn lookup_entity(&self, name: &str) -> Option<Entity> {
        if let Some(e) = self.declarations.lock().get(name) {
            return Some(e.clone());
        }
        self.variables.lock().get(name).cloned()
    }

    /// Sync concept lookup against the populated table — in-doc
    /// declarations and builtins only, no env fallback. Returns
    /// `None` for names that would require a branch query;
    /// [`prefetch_concept`](Self::prefetch_concept) is what pulls
    /// those into the table ahead of time.
    pub(crate) fn concept(&self, name: &str) -> Option<ConceptDefinition> {
        if let Some(found) = self.in_doc_concepts.lock().get(name).cloned() {
            return Some(found);
        }
        tonk_schema::builtin::lookup_concept(name)
    }

    /// Prefetch a concept from the branch into the in-doc table so
    /// the sync [`concept`](Self::concept) accessor finds it later.
    /// No-op when the name already resolves locally. This is the
    /// only concept path that touches env.
    pub(crate) async fn prefetch_concept<Env: QueryEnv>(
        &self,
        name: &str,
        env: Option<&Env>,
    ) -> Result<(), ResolveError> {
        if self.concept(name).is_some() {
            return Ok(());
        }
        // Local-only mode (`env: None`, e.g. compile-time `claim!`)
        // leaves the miss unresolved; expand surfaces it as an
        // unknown-concept error.
        let Some(env) = env else {
            return Ok(());
        };
        let found = ConceptReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(env)
            .await?;
        if let Some(definition) = found {
            self.in_doc_concepts
                .lock()
                .insert(name.to_owned(), definition.clone());
            self.in_doc_concepts_by_entity
                .lock()
                .insert(definition.entity.to_string(), definition);
        }
        Ok(())
    }

    /// Sync named-entity lookup — in-doc concepts only. Externals
    /// must be prefetched first via
    /// [`prefetch_named_entity`](Self::prefetch_named_entity).
    pub(crate) fn named_entity(&self, name: &str) -> Option<Entity> {
        self.in_doc_concepts
            .lock()
            .get(name)
            .map(|c| c.entity.clone())
    }

    /// Prefetch a named entity from the branch. Stores it as an
    /// in-doc concept entry when found so the sync accessors
    /// (`named_entity`, `concept`) pick it up. No-op when local.
    pub(crate) async fn prefetch_named_entity<Env: QueryEnv>(
        &self,
        name: &str,
        env: Option<&Env>,
    ) -> Result<(), ResolveError> {
        if self.named_entity(name).is_some() {
            return Ok(());
        }
        let Some(env) = env else {
            return Ok(());
        };
        if let Some(entity) = lookup_named_entity(name, self.source.clone(), env).await? {
            // We only have an entity, not a full descriptor; stash
            // it under a synthetic concept entry so `named_entity`
            // finds it. The descriptor is never read for a
            // name-only resolution.
            self.named_entities.lock().insert(name.to_owned(), entity);
        }
        Ok(())
    }

    /// Sync attribute-by-name lookup — in-doc attributes only.
    pub(crate) fn attribute(&self, name: &str) -> Option<AttributeDefinition> {
        self.in_doc_attributes.lock().get(name).cloned()
    }

    /// Prefetch an attribute by name from the branch into the
    /// in-doc table. No-op when local.
    pub(crate) async fn prefetch_attribute<Env: QueryEnv>(
        &self,
        name: &str,
        env: Option<&Env>,
    ) -> Result<(), ResolveError> {
        if self.attribute(name).is_some() {
            return Ok(());
        }
        let Some(env) = env else {
            return Ok(());
        };
        let found = AttributeReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(env)
            .await?;
        if let Some(definition) = found {
            self.in_doc_attributes
                .lock()
                .insert(name.to_owned(), definition.clone());
            self.in_doc_attributes_by_entity
                .lock()
                .insert(definition.entity.to_string(), definition);
        }
        Ok(())
    }

    /// Sync attribute-by-entity lookup — in-doc table only.
    pub(crate) fn attribute_by_entity(&self, entity: &Entity) -> Option<AttributeDefinition> {
        self.in_doc_attributes_by_entity
            .lock()
            .get(&entity.to_string())
            .cloned()
    }

    /// Prefetch an attribute by entity from the branch. No-op when
    /// local.
    pub(crate) async fn prefetch_attribute_by_entity<Env: QueryEnv>(
        &self,
        entity: &Entity,
        env: Option<&Env>,
    ) -> Result<(), ResolveError> {
        if self.attribute_by_entity(entity).is_some() {
            return Ok(());
        }
        let Some(env) = env else {
            return Ok(());
        };
        let found = AttributeReference::from(entity.clone())
            .resolve(self.source.clone())
            .perform(env)
            .await?;
        if let Some(definition) = found {
            self.in_doc_attributes_by_entity
                .lock()
                .insert(entity.to_string(), definition);
        }
        Ok(())
    }

    /// Look up a bare symbol to an entity using only the populated
    /// table: in-doc declarations / variables (anchor heads), then
    /// in-doc attributes, then prefetched named entities. This is
    /// the sync resolution order [`super::field::field_value_to_term`]
    /// uses; anything not in the table needs a prior prefetch.
    pub(crate) fn symbol(&self, name: &str) -> Option<Entity> {
        if let Some(entity) = self.lookup_entity(name) {
            return Some(entity);
        }
        if let Some(attribute) = self.in_doc_attributes.lock().get(name) {
            return Some(attribute.entity.clone());
        }
        if let Some(concept) = self.in_doc_concepts.lock().get(name) {
            return Some(concept.entity.clone());
        }
        self.named_entities.lock().get(name).cloned()
    }

    /// Prefetch everything a bare symbol might resolve to: the
    /// attribute table and the named-entity table. Mirrors the
    /// fallback order in `symbol`. No-op for symbols already local.
    pub(crate) async fn prefetch_symbol<Env: QueryEnv>(
        &self,
        name: &str,
        env: Option<&Env>,
    ) -> Result<(), ResolveError> {
        if self.lookup_entity(name).is_some() {
            return Ok(());
        }
        self.prefetch_attribute(name, env).await?;
        if self.attribute(name).is_some() {
            return Ok(());
        }
        self.prefetch_named_entity(name, env).await
    }

    /// Sync lookup of an installed rule prefetched for a retract.
    /// `Some(Some(rule))` is an installed rule; `Some(None)` is a
    /// confirmed-absent entity (retract of something not
    /// installed); `None` means the entity was never prefetched.
    pub(crate) fn resolved_rule(&self, entity: &Entity) -> Option<Option<Rule>> {
        self.resolved_rules.lock().get(&entity.to_string()).cloned()
    }

    /// Prefetch the installed rule at `entity` off the branch so a
    /// `rule!: ..: _` retract can read it synchronously during
    /// expand. Records `None` when nothing is installed there.
    pub(crate) async fn prefetch_rule<Env: QueryEnv>(
        &self,
        entity: &Entity,
        env: Option<&Env>,
    ) -> Result<(), RuleResolveError> {
        let key = entity.to_string();
        if self.resolved_rules.lock().contains_key(&key) {
            return Ok(());
        }
        let Some(env) = env else {
            return Ok(());
        };
        let rule = Rule::retracting(entity.clone())
            .resolve(&self.source, env)
            .await?;
        self.resolved_rules.lock().insert(key, rule);
        Ok(())
    }
}
