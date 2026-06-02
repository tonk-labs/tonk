//! Dependency-graph resolution — the front of the analyzer.
//!
//! Replaces the old async `resolve` + `prefetch_references` walk
//! (and the `analyze_local` no-op-waker hack) with three phases:
//!
//! - [`push`] — walk the [`Syntax`] once, gathering every external
//!   reference into a [`Graph`] of typed [`Need`]s plus the in-doc
//!   declarations (anchors, variables, `attribute!` / `concept!`
//!   heads). Pure and synchronous; touches no env.
//! - [`Graph::resolve`] — satisfy the needs in dependency order
//!   against a [`Resolve`]r, filling the [`Scope`] tables the
//!   `expand` phase reads. In-doc references resolve from the
//!   tables `push` already filled; only genuinely external names
//!   reach the resolver. A [`LocalOnly`] resolver answers every
//!   external need with `None`, so the env-free `claim!` path is
//!   the same code with a different resolver — no fake `Waker`.
//!
//! The product is a fully-populated [`Scope`] plus the
//! [`DeclaredApplication`]s for `attribute!` / `concept!` heads,
//! handed to `expand` (the `build` phase) unchanged.
//!
//! See `plan/analyzer-graph.md` for the design.

use std::collections::HashMap;

use dialog_artifacts::Entity;
use dialog_common::ConditionalSync;
use tonk_notation::{Effectful, Expression, FieldValue, HeadName, Syntax};

use tonk_schema::concept::QueryEnv;
use tonk_schema::query_source::Source;
use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, NamedReference,
    ResolveError,
};
use tonk_schema::rule::{Rule, RuleResolveError};

use super::assertion::{body_digest, derive_head_intent};
use super::declaration::{
    DeclaredApplication, attribute_application, concept_application, parse_attribute_body,
    parse_concept_body,
};
use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::rule::{collect_rule_concepts, is_rule_retract_body, parse_rule_this_entity};
use super::scope::Scope;
use tonk_core::claim::ConceptDescriptor as DurableConceptDescriptor;
use tonk_core::meta::AnchorName;
use tonk_schema::prelude::EntityExt;
use tonk_schema::resolution::{AttributeDefinition as AttrDef, ConceptDefinition as ConceptDef};
use tonk_schema::transact::{ThisIntent, derive_this};

/// One external name the document references that may need a branch
/// lookup. In-doc names never become `Need`s — `push` resolves
/// them into the [`Scope`] directly — so a self-contained document
/// produces an empty need set and `resolve` touches the resolver
/// zero times.
#[derive(Debug, Clone)]
enum Need {
    /// A head concept name (`person …:`) or rule premise/head
    /// concept. Resolves to a [`ConceptDefinition`].
    Concept {
        name: String,
        range: lsp_types::Range,
    },
    /// A bare-symbol field value (`field: alice`). Resolves first
    /// as an attribute, then as a named entity — mirroring
    /// [`Scope::symbol`]'s fallback order.
    Symbol {
        name: String,
        range: lsp_types::Range,
    },
    /// An attribute reference inside a `concept!`'s `with:` map,
    /// by bare symbol / `?var`. Resolves to an [`AttributeDefinition`].
    Attribute {
        name: String,
        range: lsp_types::Range,
    },
    /// An attribute reference inside a `with:` map by URI.
    AttributeByEntity {
        entity: Entity,
        range: lsp_types::Range,
    },
    /// The installed rule a `rule!: ..: _` retract targets.
    Rule {
        entity: Entity,
        range: lsp_types::Range,
    },
}

/// A `concept!` / `attribute!` head whose body `push` deferred:
/// its descriptor entity depends on the attributes its `with:` map
/// references, so the body is parsed in `resolve` after those
/// attribute [`Need`]s are satisfied.
struct PendingDeclaration {
    index: usize,
    kind: DeclarationKind,
}

#[derive(Clone, Copy)]
enum DeclarationKind {
    Attribute,
    Concept,
    /// A `command!:` head — parsed exactly like `Concept` but
    /// always transient (a command *is* a transient concept).
    Command,
}

/// The dependency graph `push` builds: the external needs to
/// resolve and the declaration heads to parse once their
/// dependencies are in scope.
pub(crate) struct Graph {
    needs: Vec<Need>,
    declarations: Vec<PendingDeclaration>,
    /// Non-meta `&anchor` heads to pre-register in [`Scope`] after
    /// the head's concept resolves. Indexed back into
    /// [`Syntax::expressions`]. Deferred from `push` because
    /// `derive_this` needs the predicate's entity, which only the
    /// resolver knows.
    pending_anchors: Vec<usize>,
}

/// The product of [`Graph::resolve`] — the declaration
/// [`Application`]s `expand` emits, keyed by source-expression
/// index. The resolved [`Scope`] is filled in place.
pub(crate) struct Resolved {
    pub declared: HashMap<usize, DeclaredApplication>,
}

/// Resolve external references against a branch (or not at all).
///
/// The real impl ([`BranchResolver`]) calls into
/// [`tonk_schema::resolution`]; the [`LocalOnly`] impl returns
/// `None` for everything, which is how the compile-time `claim!`
/// macro runs with no system: every external need stays unresolved
/// and surfaces as an unknown-concept / unknown-bookmark error
/// during the declaration registration or `expand`.
pub(crate) trait Resolve {
    async fn concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError>;
    async fn attribute(&self, name: &str) -> Result<Option<AttributeDefinition>, ResolveError>;
    async fn attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError>;
    async fn named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError>;
    async fn rule(&self, entity: &Entity) -> Result<Option<Rule>, RuleResolveError>;
}

/// Resolver that answers every external need with `None`. Used by
/// the env-free `claim!` path: in-doc names resolve from the
/// scope; anything that would need the branch is reported missing.
pub(crate) struct LocalOnly;

impl Resolve for LocalOnly {
    async fn concept(&self, _: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        Ok(None)
    }
    async fn attribute(&self, _: &str) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(None)
    }
    async fn attribute_by_entity(
        &self,
        _: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(None)
    }
    async fn named_entity(&self, _: &str) -> Result<Option<Entity>, ResolveError> {
        Ok(None)
    }
    async fn rule(&self, _: &Entity) -> Result<Option<Rule>, RuleResolveError> {
        Ok(None)
    }
}

/// Resolver backed by a live branch / transaction `source` plus a
/// per-execution `env`. Wraps the [`tonk_schema::resolution`]
/// reference types.
pub(crate) struct BranchResolver<'a, 'e, Env> {
    source: Source<'a>,
    env: &'e Env,
}

impl<'a, 'e, Env> BranchResolver<'a, 'e, Env> {
    pub(crate) fn new(source: Source<'a>, env: &'e Env) -> Self {
        Self { source, env }
    }
}

impl<Env: QueryEnv> Resolve for BranchResolver<'_, '_, Env> {
    async fn concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        ConceptReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }
    async fn attribute(&self, name: &str) -> Result<Option<AttributeDefinition>, ResolveError> {
        AttributeReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }
    async fn attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        AttributeReference::from(entity.clone())
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }
    async fn named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        tonk_schema::concept::lookup_named_entity(name, self.source.clone(), self.env).await
    }
    async fn rule(&self, entity: &Entity) -> Result<Option<Rule>, RuleResolveError> {
        Rule::retracting(entity.clone())
            .resolve(&self.source, self.env)
            .await
    }
}

/// **push** — walk the document once, register in-doc declarations
/// into the [`Scope`], and gather every external reference into a
/// [`Graph`]. Concept / attribute heads are recorded as pending so
/// their bodies parse in `resolve` once their attribute
/// dependencies are satisfied.
pub(crate) fn push(syntax: &Syntax) -> Result<Graph, AnalyzeError> {
    let mut needs: Vec<Need> = Vec::new();
    let mut declarations: Vec<PendingDeclaration> = Vec::new();
    let mut pending_anchors: Vec<usize> = Vec::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        let (head, is_claim) = match expression {
            Expression::Query(q) => (&q.predicate, false),
            Expression::Claim(c) => (&c.inner.predicate, true),
        };

        // `attribute!` / `concept!` heads: defer the body parse to
        // resolve, but gather the attribute references the concept
        // body needs now so they resolve before the parse.
        if is_claim && let HeadName::Concept(name) = &head.name {
            match name.as_str() {
                "attribute" => {
                    declarations.push(PendingDeclaration {
                        index,
                        kind: DeclarationKind::Attribute,
                    });
                    continue;
                }
                "concept" => {
                    if let Expression::Claim(Effectful { inner: a, .. }) = expression {
                        collect_concept_with_needs(a, &mut needs);
                    }
                    declarations.push(PendingDeclaration {
                        index,
                        kind: DeclarationKind::Concept,
                    });
                    continue;
                }
                "command" => {
                    if let Expression::Claim(Effectful { inner: a, .. }) = expression {
                        collect_concept_with_needs(a, &mut needs);
                    }
                    declarations.push(PendingDeclaration {
                        index,
                        kind: DeclarationKind::Command,
                    });
                    continue;
                }
                _ => {}
            }
        }

        // Non-meta `&anchor` heads: defer to `resolve` so the
        // anchor is registered with the predicate-qualified
        // `derive_this(predicate, body)` rather than a payload-only
        // hash, keeping pre-registration in step with the mutation
        // pass's `this_term_for_assertion`.
        if let Expression::Claim(Effectful {
            anchor: Some(_),
            inner: _,
        }) = expression
        {
            pending_anchors.push(index);
        }

        // Head concept name (non-meta) — may be a branch concept.
        if let HeadName::Concept(name) = &head.name
            && !matches!(name.as_str(), "attribute" | "concept" | "command")
        {
            needs.push(Need::Concept {
                name: name.clone(),
                range: head.range,
            });
        }

        // Field-value bare symbols.
        let fields = match expression {
            Expression::Query(q) => &q.fields,
            Expression::Claim(c) => &c.inner.fields,
        };
        for field in fields {
            if let FieldValue::Symbol(name) = &field.value {
                needs.push(Need::Symbol {
                    name: name.clone(),
                    range: field.value_range,
                });
            }
        }

        // `rule!:` bodies reference concepts (head + premises) and,
        // on retract, the installed rule.
        if let Expression::Claim(c) = expression
            && matches!(&c.inner.predicate.name, HeadName::Concept(n) if n == "rule")
        {
            for (name, range) in collect_rule_concepts(&c.inner) {
                needs.push(Need::Concept { name, range });
            }
            if is_rule_retract_body(&c.inner)
                && let Some(entity) = parse_rule_this_entity(&c.inner)?
            {
                needs.push(Need::Rule {
                    entity,
                    range: c.inner.range,
                });
            }
        }
    }

    Ok(Graph {
        needs,
        declarations,
        pending_anchors,
    })
}

/// Gather the attribute references a `concept!`'s `with:` map makes
/// by bare symbol / `?var` / URI, as [`Need`]s. Inline attribute
/// definitions (a nested mapping) declare their own entity and
/// need no resolution, so they're skipped here.
fn collect_concept_with_needs(assertion: &tonk_notation::Application, needs: &mut Vec<Need>) {
    for field in &assertion.fields {
        if field.name != "with" {
            continue;
        }
        let FieldValue::Nested(inner) = &field.value else {
            continue;
        };
        for sub in inner {
            match &sub.value {
                FieldValue::Nested(_) => {} // inline definition, no need
                FieldValue::Symbol(name) | FieldValue::Variable(name) => {
                    needs.push(Need::Attribute {
                        name: name.clone(),
                        range: sub.value_range,
                    });
                }
                FieldValue::Uri(uri) => {
                    if let Ok(entity) = uri.parse::<Entity>() {
                        needs.push(Need::AttributeByEntity {
                            entity,
                            range: sub.value_range,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

impl Graph {
    /// **resolve** — satisfy the gathered needs against `resolver`,
    /// filling the `scope` tables, then parse the deferred
    /// declaration heads (their attribute dependencies are in scope
    /// by now). Returns the declaration [`Application`]s keyed by
    /// source-expression index for `expand` to emit.
    ///
    /// Order matters: attribute needs resolve first (concept bodies
    /// depend on them), then the remaining needs, then declaration
    /// bodies parse and register.
    pub(crate) async fn resolve<R: Resolve + ConditionalSync>(
        self,
        syntax: &Syntax,
        scope: &Scope,
        resolver: &R,
    ) -> Result<Resolved, AnalyzeError> {
        // Pass 1 — attribute needs (concept `with:` dependencies).
        for need in &self.needs {
            match need {
                Need::Attribute { name, range } => {
                    if scope.attribute(name).is_some() {
                        continue;
                    }
                    let found = resolver.attribute(name).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::ResolverFailed {
                                context: format!("attribute {name:?}"),
                                reason: e.to_string(),
                            },
                            *range,
                        )
                    })?;
                    if let Some(def) = found {
                        scope.record_attribute(Some(name), def);
                    }
                }
                Need::AttributeByEntity { entity, range } => {
                    if scope.attribute_by_entity(entity).is_some() {
                        continue;
                    }
                    let found = resolver.attribute_by_entity(entity).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::ResolverFailed {
                                context: format!("attribute entity {entity}"),
                                reason: e.to_string(),
                            },
                            *range,
                        )
                    })?;
                    if let Some(def) = found {
                        scope.record_attribute(None, def);
                    }
                }
                _ => {}
            }
        }

        // Pass 2 — declaration bodies. Now that attribute needs are
        // satisfied, concept bodies parse synchronously against the
        // scope. Registering each declaration's entity also makes
        // it visible to later concept refs / symbols.
        let mut declared: HashMap<usize, DeclaredApplication> = HashMap::new();
        for pending in &self.declarations {
            let expression = &syntax.expressions[pending.index];
            let Expression::Claim(Effectful { anchor, inner: a }) = expression else {
                continue;
            };
            let anchor = anchor.as_ref();
            match pending.kind {
                DeclarationKind::Attribute => {
                    let plan = parse_attribute_body(a)?;
                    let entity = plan.entity.clone();
                    let attribute = AttrDef {
                        entity: entity.clone(),
                        descriptor: plan.descriptor.clone(),
                    };
                    let (this, name) = derive_head_intent(&a.fields, anchor, scope)?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    let anchor_range = anchor.map(|x| x.range).unwrap_or(a.predicate.range);
                    let variable_range = a
                        .fields
                        .iter()
                        .find(|f| f.name == "this")
                        .map(|f| f.value_range)
                        .unwrap_or(a.predicate.range);
                    if let Some(name) = &name {
                        scope.declare(name.as_str(), entity.clone(), anchor_range)?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone(), variable_range)?;
                    }
                    scope.record_attribute(
                        name.as_ref()
                            .map(AnchorName::as_str)
                            .or(variable.as_deref()),
                        attribute,
                    );
                    let application = attribute_application(&plan.descriptor, &entity, name);
                    declared.insert(
                        pending.index,
                        DeclaredApplication {
                            application,
                            inline_attributes: Vec::new(),
                        },
                    );
                }
                kind @ (DeclarationKind::Concept | DeclarationKind::Command) => {
                    let plan = parse_concept_body(a, scope)?;
                    let entity = plan.entity.clone();
                    // `command!:` is transient by definition; a
                    // `concept!:` reads its `transient:` tag. An
                    // explicit `transient:` inside a `command!:`
                    // body is redundant and silently ignored.
                    let transient = matches!(kind, DeclarationKind::Command) || plan.transient;
                    let descriptor = if transient {
                        DurableConceptDescriptor::Transient(plan.descriptor.clone())
                    } else {
                        DurableConceptDescriptor::Durable(plan.descriptor.clone())
                    };
                    let concept = ConceptDef {
                        entity: entity.clone(),
                        descriptor,
                    };
                    let (this, name) = derive_head_intent(&a.fields, anchor, scope)?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    let anchor_range = anchor.map(|x| x.range).unwrap_or(a.predicate.range);
                    let variable_range = a
                        .fields
                        .iter()
                        .find(|f| f.name == "this")
                        .map(|f| f.value_range)
                        .unwrap_or(a.predicate.range);
                    if let Some(name) = &name {
                        scope.declare(name.as_str(), entity.clone(), anchor_range)?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone(), variable_range)?;
                    }
                    scope.record_concept(
                        name.as_ref()
                            .map(AnchorName::as_str)
                            .or(variable.as_deref()),
                        concept,
                    );
                    let inline_attributes = plan
                        .inline_attributes
                        .into_iter()
                        .map(|attr| attribute_application(&attr.descriptor, &attr.entity, None))
                        .collect();
                    let application =
                        concept_application(&plan.descriptor, &entity, name, transient);
                    declared.insert(
                        pending.index,
                        DeclaredApplication {
                            application,
                            inline_attributes,
                        },
                    );
                }
            }
        }

        // Pass 3 — remaining needs (concepts, symbols, rules). These
        // can see every in-doc declaration registered above.
        for need in &self.needs {
            match need {
                Need::Concept { name, range } => {
                    if scope.concept(name).is_some() {
                        continue;
                    }
                    let found = resolver.concept(name).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::ResolverFailed {
                                context: format!("concept {name:?}"),
                                reason: e.to_string(),
                            },
                            *range,
                        )
                    })?;
                    if let Some(def) = found {
                        scope.record_concept(Some(name), def);
                    }
                }
                Need::Symbol { name, range } => {
                    if scope.symbol(name).is_some() {
                        continue;
                    }
                    // Symbols resolve as an attribute first, then as
                    // a named entity — `Scope::symbol`'s order.
                    let attr = resolver.attribute(name).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::ResolverFailed {
                                context: format!("symbol {name}"),
                                reason: e.to_string(),
                            },
                            *range,
                        )
                    })?;
                    if let Some(def) = attr {
                        scope.record_attribute(Some(name), def);
                        continue;
                    }
                    let named = resolver.named_entity(name).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::ResolverFailed {
                                context: format!("symbol {name}"),
                                reason: e.to_string(),
                            },
                            *range,
                        )
                    })?;
                    if let Some(entity) = named {
                        scope.record_named_entity(name, entity);
                    }
                }
                Need::Rule { entity, range } => {
                    if scope.resolved_rule(entity).is_some() {
                        continue;
                    }
                    let rule = resolver.rule(entity).await.map_err(|e| {
                        AnalyzeError::at(
                            AnalyzeErrorKind::RuleCompileFailed {
                                reason: format!("rule retract resolve failed at {entity}: {e}"),
                            },
                            *range,
                        )
                    })?;
                    scope.record_resolved_rule(entity, rule);
                }
                Need::Attribute { .. } | Need::AttributeByEntity { .. } => {}
            }
        }

        // Pass 4 — non-meta `&anchor` heads. With the head concept
        // (or claim domain) in scope, derive the anchor's entity
        // using the same `derive_this(predicate, body)` recipe the
        // mutation pass and the wire path use, so all three
        // converge on the same URI for the same `(predicate, body)`.
        for index in &self.pending_anchors {
            let Expression::Claim(Effectful {
                anchor: Some(anchor),
                inner: a,
            }) = &syntax.expressions[*index]
            else {
                continue;
            };
            let predicate_entity = match &a.predicate.name {
                HeadName::Concept(name) => match scope.concept(name) {
                    // Carry the concept's resolved entity (pinned or
                    // derived), not a recomputed descriptor hash, so a
                    // pinned concept's anchored instances derive from
                    // its pinned URI.
                    Some(def) => def.entity.clone(),
                    None => continue,
                },
                HeadName::Claim(domain) => Entity::of(domain),
                HeadName::Uri(_) => continue,
            };
            let entity = derive_this(&predicate_entity, &body_digest(&a.fields, scope)?);
            scope.declare(&anchor.name, entity, anchor.range)?;
        }

        Ok(Resolved { declared })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tonk_notation::parse;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A resolver that counts how many external lookups it served,
    /// answering each with `None`. `AtomicUsize` keeps it `Sync` so
    /// it satisfies `Graph::resolve`'s `ConditionalSync` bound on
    /// native. Used to assert a self-contained document drives zero
    /// branch lookups.
    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
    }

    impl Counting {
        fn bump(&self) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        fn count(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl Resolve for Counting {
        async fn concept(&self, _: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
            self.bump();
            Ok(None)
        }
        async fn attribute(&self, _: &str) -> Result<Option<AttributeDefinition>, ResolveError> {
            self.bump();
            Ok(None)
        }
        async fn attribute_by_entity(
            &self,
            _: &Entity,
        ) -> Result<Option<AttributeDefinition>, ResolveError> {
            self.bump();
            Ok(None)
        }
        async fn named_entity(&self, _: &str) -> Result<Option<Entity>, ResolveError> {
            self.bump();
            Ok(None)
        }
        async fn rule(&self, _: &Entity) -> Result<Option<Rule>, RuleResolveError> {
            self.bump();
            Ok(None)
        }
    }

    fn must_parse(src: &str) -> Syntax {
        let parsed = parse(src);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:#?}",
            parsed.diagnostics,
        );
        parsed
            .syntax
            .expect("parser produces a Syntax for non-empty input")
    }

    /// Resolving a fully self-contained document — a concept whose
    /// `with:` map defines its attribute inline, plus an instance of
    /// it — touches the resolver zero times. Every reference lands
    /// in the scope during push / the local drain.
    #[dialog_common::test]
    async fn it_resolves_a_self_contained_document_without_touching_the_resolver() {
        let syntax = must_parse(
            "concept!: &point\n  description: \"a point\"\n  with:\n    \
             x:\n      description: \"x coord\"\n      the: demo/x\n      \
             cardinality: one\n      as: signed-integer\npoint!: &origin\n  x: 0\n",
        );
        let scope = Scope::new();
        let graph = push(&syntax).expect("push");
        let resolver = Counting::default();
        graph
            .resolve(&syntax, &scope, &resolver)
            .await
            .expect("resolve");
        assert_eq!(
            resolver.count(),
            0,
            "a self-contained document must not hit the resolver",
        );
    }

    /// An external concept reference — one with no in-doc
    /// definition — reaches the resolver. Mirrors the live path
    /// where the resolver would hit the branch.
    #[dialog_common::test]
    async fn it_routes_an_external_concept_reference_to_the_resolver() {
        let syntax = must_parse("person:\n  name: ?n\n");
        let scope = Scope::new();
        let graph = push(&syntax).expect("push");
        let resolver = Counting::default();
        let _ = graph.resolve(&syntax, &scope, &resolver).await;
        assert!(
            resolver.count() >= 1,
            "an external concept reference must reach the resolver",
        );
    }
}
