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
use super::rule::builtin_kind;
use tonk_schema::resolution::{AttributeDefinition, ConceptDefinition};
use tonk_schema::rule::Rule;

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
pub(crate) struct Scope {
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
    /// Selector id (`domain/name`) → branch-declared attribute.
    /// Populated by the graph's resolve phase for claim-domain
    /// head fields; consulted when synthesizing the domain's
    /// descriptor so declared cardinality and value type govern.
    pub(crate) attributes_by_id: Mutex<HashMap<String, AttributeDefinition>>,
    /// Reverse index: concept entity → resolved concept.
    pub(crate) in_doc_concepts_by_entity: Mutex<HashMap<String, ConceptDefinition>>,
    /// Name → entity for symbols the graph resolved to a plain
    /// named entity (not a concept or attribute). Populated by
    /// [`record_named_entity`](Self::record_named_entity).
    pub(crate) named_entities: Mutex<HashMap<String, Entity>>,
    /// Entity → installed [`Rule`] read off the branch for a
    /// `rule!: ..: _` retract. Populated by
    /// [`record_resolved_rule`](Self::record_resolved_rule) during
    /// the graph's resolve phase so the retract lowering reads it
    /// synchronously. A key present with no entry means the entity
    /// holds no installed rule (retracting something absent); the
    /// decoded body's head field decided its kind.
    pub(crate) resolved_rules: Mutex<HashMap<String, Option<Rule>>>,
    /// Entity → [`ConceptDefinition`] read off the branch for a
    /// `concept!:` field retraction (`with: { f: _ }` / `..: _`).
    /// Populated by
    /// [`record_resolved_concept`](Self::record_resolved_concept)
    /// during the graph's resolve phase so the retract lowering reads
    /// the stored fields synchronously. A key present with `None`
    /// means the entity holds no concept (retracting from something
    /// absent).
    pub(crate) resolved_concepts: Mutex<HashMap<String, Option<ConceptDefinition>>>,
}

impl Scope {
    pub(crate) fn new() -> Self {
        Self {
            declarations: Mutex::new(HashMap::new()),
            variables: Mutex::new(HashMap::new()),
            in_doc_attributes: Mutex::new(HashMap::new()),
            in_doc_concepts: Mutex::new(HashMap::new()),
            in_doc_attributes_by_entity: Mutex::new(HashMap::new()),
            attributes_by_id: Mutex::new(HashMap::new()),
            in_doc_concepts_by_entity: Mutex::new(HashMap::new()),
            named_entities: Mutex::new(HashMap::new()),
            resolved_rules: Mutex::new(HashMap::new()),
            resolved_concepts: Mutex::new(HashMap::new()),
        }
    }

    /// Record an anchor-form head's entity.
    pub(crate) fn declare(
        &self,
        name: &str,
        entity: Entity,
        range: lsp_types::Range,
    ) -> Result<(), AnalyzeError> {
        // Premise heads resolve built-in formulas, constraints and
        // resolvers before concepts, so a declaration under one of
        // their names could never be referenced — every mention would
        // silently mean the built-in. Reject it here, at the one
        // chokepoint every declaration passes through.
        if let Some(kind) = builtin_kind(name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::ReservedName {
                    name: name.to_owned(),
                    kind,
                },
                range,
            ));
        }
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

    /// Record a branch-resolved attribute under the *reference*
    /// entity the document used, alongside its own entity. The two
    /// differ when the reference was an `id:<name>` URI whose
    /// published-name referent the resolver chased: parse-time
    /// lookups use the document's form, so the definition must be
    /// findable under that key too.
    pub(crate) fn record_attribute_reference(
        &self,
        reference: &Entity,
        attribute: AttributeDefinition,
    ) {
        self.in_doc_attributes_by_entity
            .lock()
            .insert(reference.to_string(), attribute.clone());
        self.record_attribute(None, attribute);
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

    /// Record a branch-resolved concept into the in-doc tables so
    /// the sync [`concept`](Self::concept) accessor finds it. Called
    /// by the graph's resolve phase for an external concept need.
    pub(crate) fn record_named_entity(&self, name: &str, entity: Entity) {
        self.named_entities.lock().insert(name.to_owned(), entity);
    }

    /// Sync attribute-by-name lookup — in-doc attributes only.
    pub(crate) fn attribute(&self, name: &str) -> Option<AttributeDefinition> {
        self.in_doc_attributes.lock().get(name).cloned()
    }

    /// Sync attribute-by-entity lookup — in-doc table only.
    pub(crate) fn attribute_by_entity(&self, entity: &Entity) -> Option<AttributeDefinition> {
        self.in_doc_attributes_by_entity
            .lock()
            .get(&entity.to_string())
            .cloned()
    }

    /// Record a branch-declared attribute under its selector id
    /// (`domain/name`), for claim-domain head fields.
    pub(crate) fn record_attribute_by_id(&self, id: &str, attribute: AttributeDefinition) {
        self.attributes_by_id
            .lock()
            .insert(id.to_owned(), attribute);
    }

    /// Sync attribute-by-selector-id lookup. In-doc declarations
    /// win over branch-resolved ones: a document that (re)declares
    /// `attribute!: … the: <id>` sees its own definition.
    pub(crate) fn attribute_by_id(&self, id: &str) -> Option<AttributeDefinition> {
        if let Some(found) = self
            .in_doc_attributes
            .lock()
            .values()
            .find(|def| def.descriptor.the().to_string() == id)
        {
            return Some(found.clone());
        }
        // Inline concept attributes register with no anchor name, so
        // they only live in the by-entity table — an id lookup must
        // still find them (a concept and a raw write of its attribute
        // in one document is the live-authoring shape).
        if let Some(found) = self
            .in_doc_attributes_by_entity
            .lock()
            .values()
            .find(|def| def.descriptor.the().to_string() == id)
        {
            return Some(found.clone());
        }
        self.attributes_by_id.lock().get(id).cloned()
    }

    /// Look up a bare symbol to an entity using only the populated
    /// table: in-doc declarations / variables (anchor heads), then
    /// in-doc attributes, then concepts, then branch-resolved named
    /// entities. This is the sync resolution order
    /// [`super::field::field_value_to_term`] uses; the graph's
    /// resolve phase fills the branch entries first.
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

    /// Sync lookup of an installed rule resolved for a retract.
    /// `Some(Some(rule))` is an installed rule; `Some(None)` is a
    /// confirmed-absent entity (retract of something not
    /// installed); `None` means the entity was never resolved.
    pub(crate) fn resolved_rule(&self, entity: &Entity) -> Option<Option<Rule>> {
        self.resolved_rules.lock().get(&entity.to_string()).cloned()
    }

    /// Record the rule resolved for a `rule!: ..: _` retract so the
    /// retract lowering reads it synchronously. `None` means
    /// nothing is installed at `entity`.
    pub(crate) fn record_resolved_rule(&self, entity: &Entity, rule: Option<Rule>) {
        self.resolved_rules.lock().insert(entity.to_string(), rule);
    }

    /// Sync lookup of a concept resolved off the branch for a field
    /// retraction. `Some(Some(def))` is the stored concept;
    /// `Some(None)` is a confirmed-absent entity; `None` means the
    /// entity was never resolved.
    pub(crate) fn resolved_concept(&self, entity: &Entity) -> Option<Option<ConceptDefinition>> {
        self.resolved_concepts
            .lock()
            .get(&entity.to_string())
            .cloned()
    }

    /// Record the concept resolved for a `concept!:` field retraction
    /// so the retract lowering reads its stored fields synchronously.
    /// `None` means no concept is stored at `entity`.
    pub(crate) fn record_resolved_concept(
        &self,
        entity: &Entity,
        concept: Option<ConceptDefinition>,
    ) {
        self.resolved_concepts
            .lock()
            .insert(entity.to_string(), concept);
    }
}
