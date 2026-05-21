//! Analyzer — turns a [`tonk_notation::Syntax`] tree into a
//! [`crate::transact::Analysis`] ready for evaluation against a
//! branch.
//!
//! See `analysis-spec.md` (sibling to this crate) for the full
//! design. Analysis runs in three phases:
//!
//! 1. **Derive.** Walk every head; populate `declarations`
//!    (anchor-form heads) and `variables` (variable-form heads)
//!    with content-derived entities. For `attribute!` /
//!    `concept!` heads the body is parsed in this phase to
//!    compute the descriptor's content-addressed entity, and the
//!    full `Application` is built up front.
//! 2. **Build the query.** For each query expression, build an
//!    [`Application`] with bare-symbol and analysis-time `?var`
//!    references substituted to constants.
//! 3. **Build the mutation plan.** For each mutation expression,
//!    emit cached meta `Application`s and, for non-meta heads,
//!    build a fresh `Application` with bare-symbol references
//!    substituted but `?var` references kept as variables — they
//!    bind at planning time against query results.
//!
//! Sub-modules:
//! - [`error`] — [`AnalyzeError`] enum
//! - [`resolver`] — [`Resolver`] trait, [`ResolvedAttribute`],
//!   [`ResolvedConcept`], [`NoopResolver`], [`ResolverError`]
//! - [`scope`] — in-document name index used during analysis
//! - [`declaration`] — `attribute!` / `concept!` body parsing +
//!   their `Application` builders
//! - [`query`] — `build_query_application`
//! - [`assertion`] — `build_assertion_application`,
//!   `derive_head_intent`
//! - [`field`] — field-value translation, scalar coercion,
//!   small utilities

// The analyzer's own consumers reference the `Resolver` trait by
// name — that trait is now deprecated in favour of
// `tonk_introspect::BranchIntrospection` (with a blanket impl
// supplying `Resolver`), but it stays as the analyzer's
// internal vocabulary. Silencing the warning crate-side keeps
// downstream consumers seeing the deprecation while not flooding
// our own build.
#![allow(deprecated)]

mod assertion;
mod declaration;
mod error;
mod field;
mod query;
mod resolver;
mod rule;
mod scan;
mod scope;

use std::collections::{HashMap, HashSet};

use dialog_common::ConditionalSync;
use tonk_notation::{Expression, HeadName, Syntax};

use crate::transact::{
    Analysis, Application, DomainApplication, MutationAnalysis, QueryAnalysis, Statement,
    ThisIntent,
};

pub use error::{
    AnalyzeDiagnostic, AnalyzeDiagnosticKind, AnalyzeError, AnalyzeErrorKind, DiagnosticSeverity,
};
pub use resolver::{NoopResolver, ResolvedAttribute, ResolvedConcept, Resolver, ResolverError};
pub use scan::scan_variables;

use crate::prelude::EntityExt;
use assertion::{body_digest, build_assertion_application, derive_head_intent};
use declaration::{
    DeclaredApplication, attribute_application, concept_application, parse_attribute_body,
    parse_concept_body,
};
use dialog_artifacts::Entity;
use field::collect_unbound_variables;
use query::build_query_application;
use scope::Scope;

/// Analyze a parsed [`Syntax`] tree.
///
/// `R: Resolver + ConditionalSync` works on both native and
/// wasm: [`ConditionalSync`] expands to `Send + Sync` on native
/// (so async-trait-generated futures stay `Send` for axum
/// handlers) and to nothing on wasm (single-threaded runtime,
/// `Resolver` itself is `?Send` there).
pub async fn analyze<R: Resolver + ConditionalSync>(
    syntax: &Syntax,
    resolver: &R,
) -> Result<Analysis, AnalyzeError> {
    if syntax.expressions.is_empty() {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::EmptyDocument,
            syntax.range,
        ));
    }

    let scope = Scope::new(resolver);

    // ---- Phase 1: derive declarations and variables ----
    //
    // For `attribute!` / `concept!` heads this also parses the
    // body so the descriptor's content-addressed entity is known
    // up front, then builds the `Application` on the spot. The
    // built application is stashed by source-expression index
    // so Phase 3 just emits it.
    let mut declared: HashMap<usize, DeclaredApplication> = HashMap::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        let (head, has_effect) = match expression {
            Expression::Query(q) => (&q.head, false),
            Expression::Assertion(a) => (&a.head, true),
            // Rule expressions are lifted in a separate analyzer
            // pass (Phase 3 notation surface). They contribute no
            // declarations to the query/mutation pipeline.
            Expression::Rule(_) => continue,
        };

        // Meta heads carry their entity in the descriptor, which
        // requires parsing the body. Do it here so the entity
        // can land in declarations/variables.
        if has_effect && let HeadName::Concept(name) = &head.name {
            match name.as_str() {
                "attribute" => {
                    let assertion = match expression {
                        Expression::Assertion(a) => a,
                        _ => continue,
                    };
                    let plan = parse_attribute_body(assertion)?;
                    let entity = plan.entity.clone();
                    let attribute = ResolvedAttribute {
                        entity: entity.clone(),
                        descriptor: plan.descriptor.clone(),
                    };
                    let (this, name) =
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref(), &scope)
                            .await?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    let anchor_range = assertion
                        .anchor
                        .as_ref()
                        .map(|a| a.range)
                        .unwrap_or(assertion.head.range);
                    let variable_range = assertion
                        .fields
                        .iter()
                        .find(|f| f.name == "this")
                        .map(|f| f.value_range)
                        .unwrap_or(assertion.head.range);
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone(), anchor_range)?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone(), variable_range)?;
                    }
                    scope.record_attribute(name.as_deref().or(variable.as_deref()), attribute);
                    let application = attribute_application(&plan.descriptor, &entity, name);
                    declared.insert(
                        index,
                        DeclaredApplication {
                            application,
                            inline_attributes: Vec::new(),
                        },
                    );
                    continue;
                }
                "concept" => {
                    // Concept body resolution may need the
                    // resolver — defer to Phase 3 and resolve
                    // here too. The body references attributes
                    // via bare-symbol / URIs that may live in
                    // either the in-doc map or the branch.
                    let assertion = match expression {
                        Expression::Assertion(a) => a,
                        _ => continue,
                    };
                    let plan = parse_concept_body(assertion, &scope).await?;
                    let entity = plan.entity.clone();
                    let concept = ResolvedConcept {
                        entity: entity.clone(),
                        descriptor: plan.descriptor.clone(),
                        transient: plan.transient,
                    };
                    let (this, name) =
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref(), &scope)
                            .await?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    let anchor_range = assertion
                        .anchor
                        .as_ref()
                        .map(|a| a.range)
                        .unwrap_or(assertion.head.range);
                    let variable_range = assertion
                        .fields
                        .iter()
                        .find(|f| f.name == "this")
                        .map(|f| f.value_range)
                        .unwrap_or(assertion.head.range);
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone(), anchor_range)?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone(), variable_range)?;
                    }
                    scope.record_concept(name.as_deref().or(variable.as_deref()), concept);
                    // Inline attrs declared inside the concept's
                    // `with:` map publish no name — the concept's
                    // local field key is not a global label for
                    // the attribute entity.
                    let inline_attributes: Vec<Application> = plan
                        .inline_attributes
                        .into_iter()
                        .map(|attr| attribute_application(&attr.descriptor, &attr.entity, None))
                        .collect();
                    let application =
                        concept_application(&plan.descriptor, &entity, name, plan.transient);
                    declared.insert(
                        index,
                        DeclaredApplication {
                            application,
                            inline_attributes,
                        },
                    );
                    continue;
                }
                _ => {}
            }
        }

        // Non-meta heads with `&anchor`: register the
        // declaration in `scope.declarations` so subsequent
        // expressions in the same document can resolve
        // `this: <anchor-name>` (or use the bare symbol in
        // field position) to the body-derived entity.
        //
        // The same registration happens again in Phase 3 via
        // `this_term_for_assertion`'s `Derived + name` branch
        // (it's the source of truth for `analysis.declarations`),
        // but Phase 3 runs after Phase 2's query build, which is
        // too late for in-doc symbol resolution. Pre-registering
        // here keeps doc-order semantics consistent. The two
        // computations agree on the entity because `body_digest`
        // is a pure function of the field literals.
        if let Expression::Assertion(a) = expression
            && let Some(anchor) = &a.anchor
        {
            let entity = Entity::of(&body_digest(&a.fields));
            scope.declare(&anchor.name, entity, anchor.range)?;
        }
    }

    // ---- Phase 2: build query Applications ----
    let mut analysis = Analysis {
        declarations: scope.declarations.lock().clone(),
        variables: scope.variables.lock().clone(),
        diagnostics: scan::scan_variables(syntax),
        ..Analysis::default()
    };

    let mut queries: Vec<Application> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for expression in &syntax.expressions {
        if let Expression::Query(q) = expression {
            let application = build_query_application(q, &scope, &analysis).await?;
            queries.push(application);
            labels.push(q.head.source.clone());
        }
    }
    if !queries.is_empty() {
        analysis.query = Some(QueryAnalysis {
            queries,
            labels,
            ..Default::default()
        });
    }

    // ---- Phase 3: build mutation Statements ----
    let mut statements: Vec<Statement> = Vec::new();
    let mut requires: HashSet<String> = HashSet::new();
    // Concept entities whose asserted facts are transient — the
    // evaluator routes these into the effects-fixpoint seed
    // bucket. Populated from each assertion's resolved concept.
    let mut transient: HashSet<Entity> = HashSet::new();
    // Indexes (within `statements`) of statements that came
    // from `attribute!` / `concept!` declarations. These are
    // excluded from auto-snapshot synthesis (Phase 4) because
    // their state lives in the schema branch, not user-facing
    // facts the editor wants to project.
    let mut declaration_statement_indexes: HashSet<usize> = HashSet::new();
    // Per-statement display label, populated from the assertion's
    // head source name. `None` for statements derived from a
    // declaration head (`attribute!` / `concept!`) — those are
    // skipped by the implicit-query synthesizer anyway.
    let mut statement_labels: Vec<Option<String>> = Vec::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        match expression {
            Expression::Query(_) => {}
            // Rule expressions are lifted by a separate pass and
            // contribute no statements to the query/mutation
            // pipeline today. Skipping here keeps the loop's
            // statement counter consistent with the query phase.
            Expression::Rule(_) => {}
            Expression::Assertion(a) => {
                if let Some(declaration) = declared.remove(&index) {
                    // `attribute!` / `concept!` head — Phase 1
                    // already built the application. Inline
                    // attribute definitions inside a `concept!`'s
                    // `with:` map are emitted as their own
                    // assertions *before* the concept itself, so
                    // the attribute facts are present on the
                    // branch (queryable via `attribute:`) by the
                    // time anything reads back.
                    for inline in declaration.inline_attributes {
                        collect_unbound_variables(&inline, &analysis, &mut requires);
                        declaration_statement_indexes.insert(statements.len());
                        statements.push(Statement::Assert(inline));
                        statement_labels.push(None);
                    }
                    collect_unbound_variables(&declaration.application, &analysis, &mut requires);
                    declaration_statement_indexes.insert(statements.len());
                    statements.push(Statement::Assert(declaration.application));
                    statement_labels.push(None);
                } else {
                    // An assertion expression can produce up to
                    // two statements: an assert side (explicit
                    // non-blank fields, plus naming) and a
                    // retract side (per-field `_` blanks plus
                    // any `..: _` rest expansion). Either or
                    // both can be empty; emit only what's
                    // present, retract first so the dissociate
                    // sees the prior state before the new
                    // assert lands.
                    let plan = build_assertion_application(a, &scope, &mut analysis).await?;
                    if let Some(retract_app) = plan.retract {
                        collect_unbound_variables(&retract_app, &analysis, &mut requires);
                        statements.push(Statement::Retract(retract_app));
                        statement_labels.push(Some(a.head.source.clone()));
                    }
                    if let Some(assert_app) = plan.assert {
                        collect_unbound_variables(&assert_app, &analysis, &mut requires);
                        // A transient-concept assertion: record the
                        // concept entity so the evaluator buckets
                        // its claims for the effects fixpoint.
                        if plan.transient
                            && let Application::Concept { query, .. } = &assert_app
                        {
                            transient.insert(query.predicate.this());
                        }
                        statements.push(Statement::Assert(assert_app));
                        statement_labels.push(Some(a.head.source.clone()));
                    }
                }
            }
        }
    }

    // requires must be subset of query bindings (analysis-time
    // variables already filtered out by `collect_unbound_variables`).
    if let Some(query) = &analysis.query {
        let bindings = query.bindings();
        for name in &requires {
            if !bindings.contains(name) {
                return Err(
                    AnalyzeErrorKind::UnboundMutationVariable { name: name.clone() }.into(),
                );
            }
        }
    } else if !requires.is_empty() {
        let name = requires.iter().next().cloned().unwrap_or_default();
        return Err(AnalyzeErrorKind::UnboundMutationVariable { name }.into());
    }

    analysis.mutate = MutationAnalysis {
        statements,
        requires,
        transient,
    };

    // ---- Phase 3b: lift rule!: expressions into Effects ----
    // Rules don't contribute to the query/mutation pipeline;
    // they install effects on the branch. Each lift resolves
    // the rule's head + premise concepts through the scope,
    // translates premise bindings into dialog Terms, and runs
    // dialog's planner to catch unbound-head-variable etc.
    for expression in &syntax.expressions {
        if let Expression::Rule(rule_expr) = expression {
            let effect = rule::lift_rule(rule_expr, &scope, &analysis).await?;
            analysis.effects.push(effect);
        }
    }

    // ---- Phase 4: synthesize implicit queries for touched entities ----
    // For every mutation statement that targets a known entity
    // and isn't already covered by a user-written query, mint an
    // auto-snapshot query so the editor's before/after view
    // surfaces the change. Skips meta-head declarations
    // (`attribute!` / `concept!`) — their state lives in the
    // schema branch, not in user-facing facts.
    synthesize_implicit_queries(
        &mut analysis,
        &statement_labels,
        &declaration_statement_indexes,
    );

    Ok(analysis)
}

/// Add `Application` entries to `analysis.query` so that every
/// mutation-touched entity gets read back into the response, even
/// when the user wrote no explicit query for it.
///
/// Skipped entirely for `attribute!` / `concept!` declarations
/// (the meta heads — their state is in the schema branch, not
/// user-visible facts).
///
/// Entities that already appear as a `this:` constant in some
/// existing query are skipped: the user query will pick them up.
fn synthesize_implicit_queries(
    analysis: &mut Analysis,
    statement_labels: &[Option<String>],
    declaration_statement_indexes: &HashSet<usize>,
) {
    use dialog_artifacts::Value;
    use dialog_query::Term;

    if analysis.mutate.statements.is_empty() {
        return;
    }

    // Existing query coverage: which entities (by their string
    // form) does some existing query enumerate via a constant
    // `this:` term?
    let mut covered: HashSet<String> = HashSet::new();
    if let Some(query) = &analysis.query {
        for application in &query.queries {
            if let Some(Term::Constant(Value::Entity(e))) = application.parameters().get("this") {
                covered.insert(e.to_string());
            }
        }
    }

    let mut implicit: Vec<Application> = Vec::new();
    let mut implicit_labels: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (index, statement) in analysis.mutate.statements.iter().enumerate() {
        if declaration_statement_indexes.contains(&index) {
            continue;
        }
        let application = statement.application();
        // Two cases of "we know which entity to snapshot":
        //
        // 1. **Update existing thing** — `this:` is a constant
        //    URI (`did:key:…`, `id:foo`, or a bare symbol that
        //    resolved to a known entity). The before-snapshot
        //    shows prior state, the after-snapshot shows the
        //    update.
        //
        // 2. **Define new (potentially redundant) thing** —
        //    `this:` is `Derived` or an unbound `Variable`. In
        //    both cases `this_term_for_assertion` minted a
        //    body-content-derived entity, which is already
        //    substituted into `application.parameters()["this"]`
        //    as a constant. The before-snapshot shows nothing
        //    (the entity didn't exist), the after-snapshot
        //    shows what we just wrote — useful for catching
        //    redundant declarations and for confirming the
        //    expected entity URI.
        //
        // Variables bound by a preceding query stay as
        // `Term::Variable` in the terms — `as_constant_entity`
        // returns `None` for those, so they're correctly skipped
        // (the user's query already covers them).
        let Some(entity) = application
            .parameters()
            .get("this")
            .and_then(as_constant_entity)
        else {
            continue;
        };
        let entity_key = entity.to_string();
        if covered.contains(&entity_key) || !seen.insert(entity_key.clone()) {
            continue;
        }
        // Skip meta-head writes — `db:attribute` / `db:concept`
        // entities live behind URI heads we don't render in the
        // user-facing snapshot.
        if entity_key.starts_with("db:") {
            continue;
        }

        let snapshot = match application {
            Application::Concept { query, .. } => {
                // Build a fresh ConceptQuery: same predicate,
                // every field bound to a named variable so the
                // engine reads it back.
                let mut terms = dialog_query::Parameters::new();
                terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
                for (field_name, _) in query.predicate.with().iter() {
                    terms.insert(
                        field_name.into(),
                        Term::<dialog_query::Any>::var(field_name),
                    );
                }
                Application::Concept {
                    query: dialog_query::concept::query::ConceptQuery {
                        terms,
                        predicate: query.predicate.clone(),
                    },
                    this: ThisIntent::Uri(entity),
                    name: None,
                }
            }
            Application::Domain { application: d, .. } => {
                // Claim heads have no schema — re-read the same
                // attributes the user wrote, with values bound
                // to fresh variables.
                let mut parameters = dialog_query::Parameters::new();
                parameters.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
                for (field_name, _) in d.parameters.iter() {
                    if field_name == "this" {
                        continue;
                    }
                    parameters.insert(
                        field_name.clone(),
                        Term::<dialog_query::Any>::var(field_name),
                    );
                }
                Application::Domain {
                    application: DomainApplication {
                        domain: d.domain.clone(),
                        parameters,
                    },
                    this: ThisIntent::Uri(entity),
                    name: None,
                }
            }
        };
        implicit.push(snapshot);
        // Reuse the assertion's head name so the rendered
        // result block carries `person` (or whatever) instead
        // of the `?` fallback. `entity_key` (a URI) is the
        // worst-case fallback; the assertion's head should
        // always be present, but the `unwrap_or_else` keeps
        // the synthesizer from panicking if a future caller
        // forgets to populate `statement_labels`.
        let label = statement_labels
            .get(index)
            .and_then(|l| l.clone())
            .unwrap_or(entity_key);
        implicit_labels.push(label);
    }

    if implicit.is_empty() {
        return;
    }
    // Snapshot queries land in `synthesized`, NOT `queries` — a
    // snapshot of a fresh assert target reads a not-yet-existing
    // entity and returns zero rows; joining it into `queries`
    // would zero the join that feeds mutation planning. The
    // renderer reads both; the evaluator's planning path reads
    // only `queries`.
    let mut query = analysis.query.clone().unwrap_or_default();
    query.synthesized.extend(implicit);
    query.synthesized_labels.extend(implicit_labels);
    analysis.query = Some(query);
}

/// Pull a concrete `Entity` out of a `Term::Constant(Value::Entity(_))`,
/// or `None` for any other term shape (variable, blank, non-entity
/// constant). Used to detect when an `Application`'s `this:` slot
/// has been resolved to a known entity at analysis time.
fn as_constant_entity(term: &dialog_query::Term<dialog_query::Any>) -> Option<Entity> {
    use dialog_artifacts::Value;
    use dialog_query::Term;
    match term {
        Term::Constant(Value::Entity(e)) => Some(e.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dialog_artifacts::{Entity, Value};
    use dialog_query::{ConceptDescriptor, Term};
    use tonk_notation::parse;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// `parse` returns `Parsed`; tests want the `Syntax` and panic on diagnostics.
    fn must_parse(src: &str) -> Syntax {
        let parsed = parse(src);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:#?}",
            parsed.diagnostics
        );
        parsed
            .syntax
            .expect("parser produces a Syntax for non-empty input")
    }

    /// Resolver that hands back a fixed `(name → ConceptDescriptor)`
    /// and errors on attribute lookups.
    struct FixedConcept {
        name: String,
        descriptor: ConceptDescriptor,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Resolver for FixedConcept {
        async fn resolve_concept(
            &self,
            name: &str,
        ) -> Result<Option<ResolvedConcept>, ResolverError> {
            if name == self.name {
                Ok(Some(ResolvedConcept {
                    entity: self.descriptor.this(),
                    descriptor: self.descriptor.clone(),
                    transient: false,
                }))
            } else {
                Ok(None)
            }
        }
        async fn resolve_attribute(
            &self,
            _name: &str,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(None)
        }
        async fn resolve_attribute_by_entity(
            &self,
            _entity: &Entity,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(None)
        }
        async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolverError> {
            Ok(None)
        }
    }

    fn fixed_concept(name: &str, fields: &[(&str, &str)]) -> FixedConcept {
        let mut with = serde_json::Map::new();
        for (field, the) in fields {
            with.insert(
                (*field).into(),
                serde_json::json!({ "the": the, "as": "Text", "cardinality": "one" }),
            );
        }
        let descriptor: ConceptDescriptor =
            serde_json::from_value(serde_json::json!({ "with": with })).unwrap();
        FixedConcept {
            name: name.into(),
            descriptor,
        }
    }

    #[dialog_common::test]
    async fn it_rejects_empty_document() {
        let syntax = Syntax {
            expressions: Vec::new(),
            range: lsp_types::Range::default(),
        };
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err.kind, AnalyzeErrorKind::EmptyDocument));
    }

    /// `attribute!: &foo` declares a content-derived attribute
    /// entity in `declarations`, an Assert statement, no query,
    /// no requires.
    #[dialog_common::test]
    async fn it_declares_a_single_attribute_assertion() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.variables.is_empty());
        assert!(analysis.query.is_none());
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(analysis.mutate.requires.is_empty());
        let Statement::Assert(Application::Concept { .. }) = &analysis.mutate.statements[0] else {
            panic!("expected Assert(Concept)");
        };
    }

    /// Three-expression doc: two `attribute!` + one `concept!`
    /// referencing them via bare symbols. Concept body resolution
    /// must hit the in-doc index, not the (Noop) outer resolver.
    #[dialog_common::test]
    async fn it_resolves_attributes_referenced_by_a_concept_in_the_same_doc() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
attribute!: &person-age
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one
  description: "Person's age"
concept!: &person
  description: "Person concept"
  with:
    name: person-name
    age:  person-age
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.declarations.contains_key("person-age"));
        assert!(analysis.declarations.contains_key("person"));
        assert!(analysis.query.is_none());
        // 3 statements — 2 attribute + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
    }

    /// `concept!` `with:` accepts inline attribute definitions
    /// alongside bare-symbol references. Each inline definition
    /// becomes its own `Statement::Assert` so the attribute
    /// surfaces in `attribute:` queries; the inline attrs are
    /// anonymous (no `dialog.meta/name` claim, since the field
    /// key is the concept's local name, not a global label).
    #[dialog_common::test]
    async fn it_emits_inline_attribute_definitions_from_concept_with() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "A person"
  with:
    name:
      description: "Name of the person"
      the:         xyz.tonk.person/name
      as:          Text
      cardinality: one
    age:
      description: "Age of the person"
      the:         xyz.tonk.person/age
      as:          UnsignedInteger
      cardinality: one
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // 3 statements: 2 inline attrs + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
        // First two are anonymous attributes (no published name).
        for i in 0..2 {
            let Statement::Assert(Application::Concept { name, .. }) =
                &analysis.mutate.statements[i]
            else {
                panic!("expected Assert(Concept) for inline attr {i}");
            };
            assert!(name.is_none(), "inline attr should have no published name");
        }
        // Third is the concept itself, anchored as `&person`.
        let Statement::Assert(Application::Concept { name, .. }) = &analysis.mutate.statements[2]
        else {
            panic!("expected Assert(Concept) for concept");
        };
        assert_eq!(name.as_deref(), Some("person"));
    }

    /// A bare symbol in field-value position resolves through the
    /// in-doc concept table — referencing `person` should find a
    /// `concept!: &person` defined earlier in the same document,
    /// not error as if `person` were an unknown attribute.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_to_in_doc_concept() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "A person"
  with:
    name:
      description: "Name of the person"
      the:         xyz.tonk.person/name
      as:          Text
      cardinality: one
concept!: &view
  description: "A view"
  with:
    source:
      description: "Source concept"
      the:         dialog.view/source
      as:          Entity
      cardinality: one
view!: &title
  source: person
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // The view's `source` term should be a constant entity
        // pointing at the `person` concept's entity, not a
        // variable.
        let person_entity = analysis
            .declarations
            .get("person")
            .expect("person was declared");
        let view_assert = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = view_assert else {
            panic!("expected view assertion");
        };
        match query.terms.get("source") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert_eq!(e, person_entity, "source should resolve to person's entity");
            }
            other => panic!("source should be a constant entity, got {other:?}"),
        }
    }

    /// Attribute definitions without `description` are rejected.
    /// Both `attribute!: &foo` form and inline-in-`concept!` form.
    #[dialog_common::test]
    async fn it_rejects_attribute_definition_without_description() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/foo
  as:          Text
  cardinality: one
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("description")),
            "expected InvalidAttributeBody about description, got {err:?}"
        );
    }

    /// Variable-form `attribute!:` with `this: ?foo` (no anchor)
    /// lands in `variables`, not `declarations`, and does NOT
    /// emit a `dialog.meta/name` claim (the name is doc-scoped
    /// only).
    #[dialog_common::test]
    async fn it_keeps_variable_form_attribute_doc_scoped() {
        let syntax = must_parse(
            r#"
attribute!:
  this:        ?person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // Variable-form `this:` (no anchor) registers the name
        // in `variables`, not `declarations`. The meta-pass
        // resolves the variable to the body-derived attribute
        // entity and stores it for cross-expression sharing.
        assert!(analysis.declarations.is_empty());
        assert!(analysis.variables.contains_key("person-name"));
        let Statement::Assert(Application::Concept { query, name, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        // Variable-form attributes publish no name (no anchor),
        // so neither `name` nor any predicate `name` term is set.
        assert!(query.terms.get("name").is_none());
        assert!(name.is_none());
    }

    /// `attribute!: &foo` (anchored): the head's `name` slot
    /// records the published name. The planner emits the
    /// `dialog.meta/name` claim on `id:<name>`, not as a
    /// parameter on the predicate.
    #[dialog_common::test]
    async fn it_records_published_name_for_anchored_attribute() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, name, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(query.terms.get("name").is_none());
        assert_eq!(name.as_deref(), Some("person-name"));
    }

    /// Two meta heads of different kinds (`attribute!` and
    /// `concept!`) that share an anchor name → `DuplicateName`.
    /// Phase 1 sees both heads declare `a` and the second
    /// `declare` returns `Some(prior_entity)`, triggering the
    /// error. Only meta heads register declarations in Phase 1
    /// — non-meta heads defer their entity to Phase 3 and so
    /// are not checked for name collisions today.
    #[dialog_common::test]
    async fn it_rejects_duplicate_meta_bookmark_anchors() {
        // The concept's `with: { x: a }` references the `a`
        // attribute defined just above, so concept-body
        // resolution succeeds and Phase 1 reaches the second
        // `declare("a", …)` call which finds the prior entry.
        let syntax = must_parse(
            r#"
attribute!: &a
  the:         x.y/a
  as:          Text
  cardinality: one
  description: "A"
concept!: &a
  with:
    x: a
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::DuplicateName { .. }),
            "expected DuplicateName, got {err:?}"
        );
    }

    /// An anchor and a `this: ?var` that share a name →
    /// `NameShadowing`.
    #[dialog_common::test]
    async fn it_rejects_anchor_and_this_variable_with_same_name() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/a
  as:          Text
  cardinality: one
  description: "A"
attribute!:
  this:        ?foo
  the:         x.y/b
  as:          Text
  cardinality: one
  description: "B"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err.kind, AnalyzeErrorKind::NameShadowing { .. }));
    }

    /// Pure-query document: `Analysis::query` is `Some`, no
    /// statements, no requires.
    #[dialog_common::test]
    async fn it_analyzes_a_pure_query_document() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(analysis.query.is_some());
        assert!(analysis.mutate.statements.is_empty());
        assert!(analysis.mutate.requires.is_empty());
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        assert!(q.bindings().contains("alice"));
    }

    /// Query + assertion joined by `?alice`: the mutation
    /// records `alice` in `requires`, and the analyzer does
    /// not error (the query binds it).
    #[dialog_common::test]
    async fn it_joins_query_and_assertion_via_this_variable() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
person!:
  this: ?alice
  name: "Renamed"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(analysis.query.is_some());
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(analysis.mutate.requires.contains("alice"));
    }

    /// Mutation references `?bogus` that no source binds →
    /// `UnboundMutationVariable`.
    #[dialog_common::test]
    async fn it_rejects_unbound_mutation_variable() {
        let syntax = must_parse(
            r#"
person!:
  this: ?ghost
  name: ?nope
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err.kind,
            AnalyzeErrorKind::UnboundMutationVariable { .. }
        ));
    }

    /// Concept retraction via `..: _`: blank terms for every
    /// field, the `this` term carries the URI-resolved entity.
    #[dialog_common::test]
    async fn it_blanks_every_field_for_concept_retraction() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // Stage 2.7: `..: _` produces a `Statement::Retract`
        // since every field is blank and no `&anchor` publishes
        // a name on the assert side.
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Retract(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Retract(Concept) for `..: _` body");
        };
        // `this` is the URI-resolved entity; every with: field
        // is blank (the planner walks the branch to find values
        // to dissociate).
        assert!(matches!(
            q.terms.get("this"),
            Some(Term::Constant(Value::Entity(_)))
        ));
        for field in ["name", "age"] {
            assert!(
                matches!(q.terms.get(field), Some(Term::Variable { name: None, .. })),
                "field {field:?} should be blank"
            );
        }
    }

    /// Unknown concept in head position → `UnknownConcept`.
    #[dialog_common::test]
    async fn it_errors_on_unknown_concept() {
        let syntax = must_parse(
            r#"
nope:
  field: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err.kind, AnalyzeErrorKind::UnknownConcept { .. }));
    }

    /// Built-in `attribute:` resolves without a branch resolver
    /// — the registry is consulted before the inner resolver, so
    /// the LSP (which uses [`NoopResolver`]) gets autocomplete /
    /// diagnostics for built-ins for free.
    #[dialog_common::test]
    async fn it_resolves_builtin_attribute_under_noop_resolver() {
        let syntax = must_parse(
            r#"
attribute:
  this: ?a
  description: ?d
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        // The four anonymous-attribute fields — id/type/
        // cardinality/description — must all be present in the
        // unified term map. `name` is intentionally not in the
        // built-in `attribute:` view (only anchor-form attrs
        // carry a `dialog.meta/name` claim).
        for field in ["id", "type", "cardinality", "description"] {
            assert!(query.terms.contains(field), "missing {field}");
        }
    }

    /// Built-in `branch:` (and other Rust-defined repository
    /// concepts) resolve through the registry, not the resolver,
    /// even though they have no branch-side `concept!`
    /// definition.
    #[dialog_common::test]
    async fn it_resolves_builtin_branch_under_noop_resolver() {
        let syntax = must_parse(
            r#"
branch:
  this: ?b
  name: ?name
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        assert!(query.terms.contains("name"));
        assert!(query.terms.contains("origin"));
    }

    /// Built-in `attribute:` empty-body query surfaces every
    /// well-known field as a defaulted variable. Documents the
    /// "fields you didn't mention default to ?field" behavior
    /// applied to a built-in.
    #[dialog_common::test]
    async fn it_defaults_every_attribute_field_to_a_variable_on_empty_body() {
        let syntax = must_parse("attribute:\n");
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        for field in ["id", "type", "cardinality", "description"] {
            assert!(query.terms.contains(field), "missing {field}");
        }
    }

    /// Claim head with no fields → `ClaimWithoutFields`.
    #[dialog_common::test]
    async fn it_errors_on_claim_without_fields() {
        let syntax = must_parse("xyz.tonk:\n");
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(
            err.kind,
            AnalyzeErrorKind::ClaimWithoutFields { .. }
        ));
    }

    /// Claim heads build a synthesized predicate with one
    /// `<domain>/<field>` attribute per parameter.
    #[dialog_common::test]
    async fn it_synthesizes_descriptor_for_claim_head() {
        let syntax = must_parse(
            r#"
xyz.tonk:
  role: ?role
  contact: "alice"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let crate::transact::Application::Domain { application: d, .. } = &q.queries[0] else {
            panic!("expected Domain application");
        };
        assert_eq!(d.domain, "xyz.tonk");
        assert!(d.parameters.contains("role"));
        assert!(d.parameters.contains("contact"));
    }

    /// Anonymous-head queries (`person:`) need `this` as a
    /// named variable, not a blank — otherwise dialog's engine
    /// fails with `UnboundVariable { variable_name: "this" }`
    /// at evaluation. Regression for that crash.
    #[dialog_common::test]
    async fn it_binds_this_as_variable_for_anonymous_query_head() {
        let syntax = must_parse(
            r#"
person:
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        assert!(matches!(
            query.terms.get("this"),
            Some(Term::Variable { name: Some(_), .. })
        ));
    }

    /// `name` is a built-in concept resolvable without a branch
    /// — same as `attribute` and `concept`. Backed by the single
    /// `dialog.meta/name` attribute (cardinality one).
    #[dialog_common::test]
    async fn it_resolves_builtin_name_concept() {
        let syntax = must_parse(
            r#"
name:
  this: ?n
  entity: ?e
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        // `entity` field comes from the `Name` concept's `with:`.
        assert!(query.terms.contains("entity"));
    }

    /// The concept-of-concept built-in resolves to `db:concept`
    /// — *not* the legacy `concept:concept`.
    #[dialog_common::test]
    async fn it_uses_db_concept_uri_for_concept_marker() {
        // Asserting any concept emits a marker claim
        // `(this, dialog.meta/concept, db:concept)`. We can read
        // it back by inspecting the `concept` term of the
        // emitted concept-head application.
        let syntax = must_parse(
            r#"
concept!: &foo
  description: "x"
  with:
    bar:
      description: "y"
      the: x.y/bar
      as: Text
      cardinality: one
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // Last statement is the concept itself — its `concept`
        // term should be the marker entity.
        let last = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = last else {
            panic!("expected Concept application for concept");
        };
        match query.terms.get("concept") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert_eq!(
                    e.to_string(),
                    "db:concept",
                    "concept marker should be db:concept"
                );
            }
            other => panic!("expected concept term to be db:concept entity, got {other:?}"),
        }
    }

    /// The anchor publication (`person!: &alice`) is recorded
    /// as `name: Some("alice")` on the `Application`. The `this`
    /// intent stays `Derived` since the body produces the entity.
    #[dialog_common::test]
    async fn it_records_anchor_publication_on_application() {
        let syntax = must_parse(
            r#"
person!: &alice
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Assert(Application::Concept { name, this, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_deref(), Some("alice"));
        assert!(matches!(this, ThisIntent::Derived));
    }

    // ----------------------------------------------------------- //
    // (this × name) coverage matrix                               //
    // ----------------------------------------------------------- //

    /// `&anchor` + `this: did:key:…` together — publish a name
    /// pointing at an existing entity, no body-derivation. The
    /// canonical Stage 2.3 case the orthogonal `(this, name)`
    /// fields exist to express.
    #[dialog_common::test]
    async fn it_combines_anchor_with_this_uri_to_publish_name_for_existing_entity() {
        let syntax = must_parse(
            r#"
person!: &alice
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { name, this, query }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_deref(), Some("alice"));
        match this {
            ThisIntent::Uri(e) => assert!(e.to_string().starts_with("did:key:")),
            other => panic!("expected ThisIntent::Uri, got {other:?}"),
        }
        // `terms["this"]` carries the URI entity verbatim, not a
        // body-derived hash.
        match query.terms.get("this") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert!(e.to_string().starts_with("did:key:"));
            }
            other => panic!("expected this term to be the URI entity, got {other:?}"),
        }
    }

    /// `&anchor` + `this: ?var` — anchor publishes a name
    /// pointing at the variable's resolved entity. The `this`
    /// intent stays `Variable(name)`; the planner's
    /// substitute-then-emit path handles the rest.
    #[dialog_common::test]
    async fn it_combines_anchor_with_this_variable() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
person!: &latest-alice
  this: ?alice
  name: "Renamed"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // The mutation expression is the second statement.
        let Statement::Assert(Application::Concept { name, this, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_deref(), Some("latest-alice"));
        assert!(matches!(this, ThisIntent::Variable(s) if s == "alice"));
        assert!(analysis.mutate.requires.contains("alice"));
    }

    /// `this: id:foo` (Uri form, `id:` scheme). Same shape as
    /// the `did:key:` case — analyzer treats every URI as direct.
    #[dialog_common::test]
    async fn it_accepts_id_uri_in_this() {
        let syntax = must_parse(
            r#"
person!:
  this: id:alice
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { this, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        match this {
            ThisIntent::Uri(e) => assert_eq!(e.to_string(), "id:alice"),
            other => panic!("expected ThisIntent::Uri(id:alice), got {other:?}"),
        }
    }

    /// `this: db:concept` is rejected — `db:` is reserved for
    /// system-published built-ins; user assertions cannot
    /// modify what lives there. Stage 2.4.
    #[dialog_common::test]
    async fn it_rejects_assertion_targeting_db_uri() {
        let syntax = must_parse(
            r#"
person!:
  this: db:concept
  name: "x"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::ProtectedUri { entity, scheme } if entity == "db:concept" && scheme == "db"),
            "expected ProtectedUri{{entity:\"db:concept\", scheme:\"db\"}}, got {err:?}"
        );
    }

    /// Same protection fires when `db:` arrives via a resolved
    /// bare symbol — the gate runs on the resolved
    /// `ThisIntent::Uri`, not on the source-form text.
    #[dialog_common::test]
    async fn it_rejects_assertion_when_resolved_symbol_targets_db_uri() {
        struct DbResolver;
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        impl Resolver for DbResolver {
            async fn resolve_concept(
                &self,
                name: &str,
            ) -> Result<Option<ResolvedConcept>, ResolverError> {
                if name == "person" {
                    let descriptor: ConceptDescriptor =
                        serde_json::from_value(serde_json::json!({
                            "with": { "name": { "the": "x.y/name", "as": "Text", "cardinality": "one" } }
                        }))
                        .unwrap();
                    Ok(Some(ResolvedConcept {
                        entity: descriptor.this(),
                        descriptor,
                        transient: false,
                    }))
                } else {
                    Ok(None)
                }
            }
            async fn resolve_attribute(
                &self,
                _name: &str,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_attribute_by_entity(
                &self,
                _entity: &Entity,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_named_entity(
                &self,
                name: &str,
            ) -> Result<Option<Entity>, ResolverError> {
                if name == "evil" {
                    Ok(Some("db:concept".parse().unwrap()))
                } else {
                    Ok(None)
                }
            }
        }
        let syntax = must_parse(
            r#"
person!:
  this: evil
  name: "x"
"#,
        );
        let err = analyze(&syntax, &DbResolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::ProtectedUri { entity, .. } if entity == "db:concept"),
            "expected ProtectedUri after resolving `evil` to db:concept, got {err:?}"
        );
    }

    /// Querying `db:` URIs is fine — only assertions are
    /// protected.
    #[dialog_common::test]
    async fn it_allows_querying_with_db_uri_in_this() {
        let syntax = must_parse(
            r#"
attribute:
  this: db:attribute
  description: ?d
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.query.is_some());
    }

    /// `this: alice` (bare symbol) resolves through the name
    /// table — Stage 2.5. A bare symbol that doesn't match any
    /// in-doc declaration or branch name surfaces
    /// `UnknownBookmark` with `field: "this"`.
    #[dialog_common::test]
    async fn it_rejects_unresolvable_bare_symbol_in_this() {
        let syntax = must_parse(
            r#"
person!:
  this: ghost
  name: "x"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::UnknownBookmark { field, bookmark } if field == "this" && bookmark == "ghost"),
            "expected UnknownBookmark on `this` with bookmark=\"ghost\", got {err:?}"
        );
    }

    /// `this: alice` resolves through the in-doc anchor table
    /// — `&alice` declared by an earlier expression in the same
    /// document means a later `this: alice` lands on that
    /// entity.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_in_this_via_in_doc_anchor() {
        let syntax = must_parse(
            r#"
person!: &alice
  name: "Alice"
person!:
  this: alice
  name: "Renamed"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // First expression declares `&alice`. Second uses
        // `this: alice` — should resolve to the same body-derived
        // entity that first expression registered.
        let alice_entity = analysis
            .declarations
            .get("alice")
            .expect("alice should be declared")
            .clone();
        let Statement::Assert(Application::Concept { this, query, .. }) =
            &analysis.mutate.statements[1]
        else {
            panic!("expected Assert(Concept) for second expression");
        };
        match this {
            ThisIntent::Uri(e) => assert_eq!(e, &alice_entity),
            other => panic!("expected ThisIntent::Uri(<alice>), got {other:?}"),
        }
        // The resolved entity also flows into `terms["this"]`.
        match query.terms.get("this") {
            Some(Term::Constant(Value::Entity(e))) => assert_eq!(e, &alice_entity),
            other => panic!("expected this term to be alice's entity, got {other:?}"),
        }
    }

    /// `this: alice` resolves through the branch's name index
    /// when no in-doc declaration matches. Uses a custom
    /// resolver whose `resolve_named_entity` returns a fixed
    /// entity for `"alice"`.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_in_this_via_branch_name_index() {
        struct NameResolver {
            concept_name: String,
            concept: ConceptDescriptor,
            named_entity: Entity,
        }
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        impl Resolver for NameResolver {
            async fn resolve_concept(
                &self,
                name: &str,
            ) -> Result<Option<ResolvedConcept>, ResolverError> {
                if name == self.concept_name {
                    Ok(Some(ResolvedConcept {
                        entity: self.concept.this(),
                        descriptor: self.concept.clone(),
                        transient: false,
                    }))
                } else {
                    Ok(None)
                }
            }
            async fn resolve_attribute(
                &self,
                _name: &str,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_attribute_by_entity(
                &self,
                _entity: &Entity,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_named_entity(
                &self,
                name: &str,
            ) -> Result<Option<Entity>, ResolverError> {
                if name == "alice" {
                    Ok(Some(self.named_entity.clone()))
                } else {
                    Ok(None)
                }
            }
        }
        let person_descriptor: ConceptDescriptor = serde_json::from_value(serde_json::json!({
            "with": {
                "name": { "the": "io.gozala.person/name", "as": "Text", "cardinality": "one" },
            }
        }))
        .unwrap();
        let alice_entity: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv"
            .parse()
            .unwrap();
        let resolver = NameResolver {
            concept_name: "person".into(),
            concept: person_descriptor,
            named_entity: alice_entity.clone(),
        };
        let syntax = must_parse(
            r#"
person!:
  this: alice
  name: "Renamed"
"#,
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { this, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        match this {
            ThisIntent::Uri(e) => assert_eq!(e, &alice_entity),
            other => panic!("expected ThisIntent::Uri(alice), got {other:?}"),
        }
    }

    /// `this: foo` works in queries too — same lookup as
    /// assertions, surfaces a constant entity in `terms["this"]`.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_in_this_for_queries() {
        let syntax = must_parse(
            r#"
person!: &alice
  name: "Alice"
person:
  this: alice
  name: ?n
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let alice_entity = analysis
            .declarations
            .get("alice")
            .expect("alice declared")
            .clone();
        let q = analysis.query.as_ref().expect("query present");
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept query");
        };
        match query.terms.get("this") {
            Some(Term::Constant(Value::Entity(e))) => assert_eq!(e, &alice_entity),
            other => panic!("expected this to resolve to alice, got {other:?}"),
        }
    }

    /// `this: my-concept` resolves to a `concept!: &my-concept`
    /// declared earlier in the same document. Symmetric with the
    /// in-doc anchor case for non-meta heads, but routes through
    /// `scope.declarations` after `concept!` registers it.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_in_this_via_in_doc_concept_anchor() {
        let syntax = must_parse(
            r#"
concept!: &my-thing
  description: "x"
  with:
    label:
      description: "the label"
      the:         x.y/label
      as:          Text
      cardinality: one
my-thing!:
  this: my-thing
  label: "hi"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let concept_entity = analysis
            .declarations
            .get("my-thing")
            .expect("my-thing declared")
            .clone();
        // Last statement is the `my-thing!` instance; its `this`
        // should be the resolved concept entity.
        let last = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { this, .. }) = last else {
            panic!("expected Assert(Concept) for instance");
        };
        match this {
            ThisIntent::Uri(e) => assert_eq!(e, &concept_entity),
            other => panic!("expected ThisIntent::Uri(my-thing), got {other:?}"),
        }
    }

    /// Non-meta `&anchor` and meta `&same-name` collide →
    /// `DuplicateName`. Tests Phase 1's pre-registration of
    /// non-meta anchors.
    #[dialog_common::test]
    async fn it_rejects_collision_between_meta_and_non_meta_anchors() {
        let syntax = must_parse(
            r#"
person!: &foo
  name: "Alice"
attribute!: &foo
  the:         x.y/foo
  as:          Text
  cardinality: one
  description: "F"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::DuplicateName { .. }),
            "expected DuplicateName, got {err:?}"
        );
    }

    /// `this: 42` (literal) is rejected — `this:` accepts only
    /// `?var` / URI / bare symbol per the guide.
    #[dialog_common::test]
    async fn it_rejects_literal_in_this() {
        let syntax = must_parse(
            r#"
person!:
  this: 42
  name: "x"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err.kind,
            AnalyzeErrorKind::UnsupportedFieldValue { .. }
        ));
    }

    /// `this: { entropy: ... }` (mapping form for explicit
    /// content-derivation salt) — the parser produces it but the
    /// analyzer doesn't yet handle it. Pin the current behavior.
    #[dialog_common::test]
    async fn it_rejects_nested_mapping_in_this_pending_implementation() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
  this:
    entropy: "salt"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err.kind,
            AnalyzeErrorKind::UnsupportedFieldValue { .. }
        ));
    }

    // ----------------------------------------------------------- //
    // Anchor declarations land in `analysis.declarations`         //
    // ----------------------------------------------------------- //

    /// `&alice` on a non-meta head registers `alice → entity`
    /// in `analysis.declarations` so later expressions and the
    /// editor can reach the published entity by name.
    #[dialog_common::test]
    async fn it_records_non_meta_anchor_in_declarations() {
        let syntax = must_parse(
            r#"
person!: &alice
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(
            analysis.declarations.contains_key("alice"),
            "expected `alice` in declarations: {:?}",
            analysis.declarations
        );
    }

    // ----------------------------------------------------------- //
    // Field-value scalar types                                    //
    // ----------------------------------------------------------- //

    /// Integer field values flow through as `Value::SignedInt`.
    #[dialog_common::test]
    async fn it_carries_integer_field_values_through_to_terms() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
  age: 28
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(matches!(
            query.terms.get("age"),
            Some(Term::Constant(Value::SignedInt(28)))
        ));
    }

    /// Boolean field values flow through as `Value::Boolean`.
    #[dialog_common::test]
    async fn it_carries_boolean_field_values_through_to_terms() {
        let syntax = must_parse(
            r#"
thing!:
  active: true
"#,
        );
        let resolver = fixed_concept("thing", &[("active", "x.y/active")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(matches!(
            query.terms.get("active"),
            Some(Term::Constant(Value::Boolean(true)))
        ));
    }

    /// Float field values flow through as `Value::Float`.
    #[dialog_common::test]
    async fn it_carries_float_field_values_through_to_terms() {
        let syntax = must_parse(
            r#"
thing!:
  weight: 1.5
"#,
        );
        let resolver = fixed_concept("thing", &[("weight", "x.y/weight")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        match query.terms.get("weight") {
            Some(Term::Constant(Value::Float(f))) => {
                assert!((f - 1.5).abs() < f64::EPSILON);
            }
            other => panic!("expected Float(1.5), got {other:?}"),
        }
    }

    /// `null` field value is rejected — the analyzer's
    /// `scalar_to_value` has no mapping for null.
    #[dialog_common::test]
    async fn it_rejects_null_field_value() {
        let syntax = must_parse(
            r#"
thing!:
  weight: null
"#,
        );
        let resolver = fixed_concept("thing", &[("weight", "x.y/weight")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err.kind,
            AnalyzeErrorKind::UnsupportedFieldValue { .. }
        ));
    }

    // ----------------------------------------------------------- //
    // Field-value resolution / variable substitution              //
    // ----------------------------------------------------------- //

    /// A `?var` whose value was derived in Phase 1 (e.g. by an
    /// earlier `attribute! ?var:` head) substitutes through to a
    /// `Term::Constant`, not a `Term::Variable`.
    #[dialog_common::test]
    async fn it_substitutes_known_variables_in_field_position() {
        let syntax = must_parse(
            r#"
attribute!:
  this: ?person-name
  the:         x.y/name
  as:          Text
  cardinality: one
  description: "x"
concept!:
  this: ?person
  description: "p"
  with:
    name: ?person-name
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // The concept assertion is the second statement.
        let Statement::Assert(Application::Concept { query, .. }) = &analysis.mutate.statements[1]
        else {
            panic!("expected Assert(Concept) for concept");
        };
        // `with.name` should be a constant (the attribute's
        // entity), not a variable.
        match query.terms.get("with.name") {
            Some(Term::Constant(Value::Entity(_))) => {}
            other => panic!("with.name should be Constant(Entity), got {other:?}"),
        }
    }

    /// A bare symbol that doesn't resolve anywhere surfaces
    /// `UnknownBookmark` (with the symbol's name in `bookmark`).
    #[dialog_common::test]
    async fn it_rejects_unresolvable_bare_symbol() {
        let syntax = must_parse(
            r#"
person!:
  name: ghost
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::UnknownBookmark { bookmark, .. } if bookmark == "ghost"),
            "expected UnknownBookmark{{bookmark:\"ghost\"}}, got {err:?}"
        );
    }

    // ----------------------------------------------------------- //
    // Reserved meta-fields                                        //
    // ----------------------------------------------------------- //

    /// `..:` inside a body isn't surfaced as an `UnknownField`.
    /// Stage 2.7 will give it real semantics; today it's a
    /// silently-tolerated meta-key.
    #[dialog_common::test]
    async fn it_does_not_treat_dotdot_as_unknown_field() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  ..: _
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        // Should not error with `UnknownField`.
        analyze(&syntax, &resolver).await.unwrap();
    }

    // ----------------------------------------------------------- //
    // Error paths                                                 //
    // ----------------------------------------------------------- //

    /// Empty assertion body (no fields, no `this:`, no anchor)
    /// → `AssertionWithoutFields`.
    #[dialog_common::test]
    async fn it_errors_on_assertion_without_fields() {
        let syntax = must_parse("person!:\n");
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::AssertionWithoutFields { .. }),
            "expected AssertionWithoutFields, got {err:?}"
        );
    }

    /// `concept!` body with no `with:` field → `InvalidConceptBody`.
    #[dialog_common::test]
    async fn it_errors_on_concept_body_missing_with_field() {
        let syntax = must_parse(
            r#"
concept!: &foo
  description: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidConceptBody { .. }),
            "expected InvalidConceptBody, got {err:?}"
        );
    }

    /// User-supplied field that isn't in the concept's `with:`
    /// map → `UnknownField`.
    #[dialog_common::test]
    async fn it_errors_on_unknown_field_in_assertion_body() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
  bogus: "x"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::UnknownField { field, .. } if field == "bogus"),
            "expected UnknownField{{field:\"bogus\"}}, got {err:?}"
        );
    }

    /// Claim head with a malformed field name → `InvalidClaimAttribute`.
    /// `field/with/slashes` doesn't parse as a `the:` URI.
    #[dialog_common::test]
    async fn it_errors_on_invalid_claim_attribute() {
        let syntax = must_parse(
            r#"
xyz.tonk:
  has/slash: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidClaimAttribute { .. }),
            "expected InvalidClaimAttribute, got {err:?}"
        );
    }

    /// A failing resolver propagates as `ResolverFailed`.
    #[dialog_common::test]
    async fn it_surfaces_resolver_failures() {
        struct FailingResolver;
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        impl Resolver for FailingResolver {
            async fn resolve_concept(
                &self,
                _name: &str,
            ) -> Result<Option<ResolvedConcept>, ResolverError> {
                Err(ResolverError::new("simulated I/O failure"))
            }
            async fn resolve_attribute(
                &self,
                _name: &str,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_attribute_by_entity(
                &self,
                _entity: &Entity,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
            async fn resolve_named_entity(
                &self,
                _name: &str,
            ) -> Result<Option<Entity>, ResolverError> {
                Ok(None)
            }
        }
        let syntax = must_parse(
            r#"
person:
  name: "Alice"
"#,
        );
        let err = analyze(&syntax, &FailingResolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::ResolverFailed { .. }),
            "expected ResolverFailed, got {err:?}"
        );
    }

    // ----------------------------------------------------------- //
    // Built-in concept registry — remaining entries               //
    // ----------------------------------------------------------- //

    /// Built-in `concept:` resolves through the registry without
    /// a branch. Returns the concept-of-concept descriptor whose
    /// entity is `db:concept`.
    #[dialog_common::test]
    async fn it_resolves_builtin_concept_under_noop_resolver() {
        let syntax = must_parse(
            r#"
concept:
  this: ?c
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_replica_under_noop_resolver() {
        let syntax = must_parse(
            r#"
replica:
  this: ?r
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_remote_under_noop_resolver() {
        let syntax = must_parse(
            r#"
remote:
  this: ?r
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_tracking_branch_under_noop_resolver() {
        let syntax = must_parse(
            r#"
tracking-branch:
  this: ?t
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.query.is_some());
    }

    // ----------------------------------------------------------- //
    // Statement order: inline attrs before their concept          //
    // ----------------------------------------------------------- //

    /// Inline attribute definitions inside a `concept!`'s
    /// `with:` emit *before* the concept itself, so the
    /// attribute facts are present on the branch by the time
    /// anything reads back. Order matters: an off-by-one or
    /// reorder would silently break runtime behavior.
    #[dialog_common::test]
    async fn it_orders_inline_attrs_strictly_before_concept() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "p"
  with:
    name:
      description: "Name"
      the:         x.y/name
      as:          Text
      cardinality: one
    age:
      description: "Age"
      the:         x.y/age
      as:          UnsignedInteger
      cardinality: one
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // 2 inline attrs + 1 concept = 3 statements.
        assert_eq!(analysis.mutate.statements.len(), 3);

        // The concept itself carries an anchor name `person`;
        // inline attrs publish no name. Order: attr, attr, concept.
        for (i, stmt) in analysis.mutate.statements.iter().enumerate() {
            let Statement::Assert(Application::Concept { name, .. }) = stmt else {
                panic!("statement {i} should be Assert(Concept)");
            };
            if i < 2 {
                assert!(
                    name.is_none(),
                    "statement {i} (inline attr) should publish no name"
                );
            } else {
                assert_eq!(name.as_deref(), Some("person"), "concept itself anchored");
            }
        }
    }

    // ----------------------------------------------------------- //
    // Stage 2.6 — type-name normalization                         //
    // ----------------------------------------------------------- //

    /// User-facing kebab-case-lowercase type names (`text`,
    /// `unsigned-integer`, …) are accepted and normalized to
    /// dialog's PascalCase serde form. The analyzer hides the
    /// translation from authors.
    #[dialog_common::test]
    async fn it_accepts_lowercase_type_names_in_attribute_body() {
        let syntax = must_parse(
            r#"
attribute!: &age
  the:         x.y/age
  as:          unsigned-integer
  cardinality: one
  description: "Person's age"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("age"));
    }

    /// Each guide-listed type name is accepted in `as:`.
    #[dialog_common::test]
    async fn it_accepts_every_lowercase_type_name() {
        for ty in &[
            "text",
            "unsigned-integer",
            "signed-integer",
            "float",
            "boolean",
            "entity",
            "bytes",
        ] {
            let src = format!(
                "attribute!: &foo\n  the:         x.y/foo\n  as:          {ty}\n  cardinality: one\n  description: \"x\"\n"
            );
            let syntax = must_parse(&src);
            analyze(&syntax, &NoopResolver)
                .await
                .unwrap_or_else(|e| panic!("type {ty:?} should be accepted: {e:?}"));
        }
    }

    /// Legacy PascalCase type names still work — schemas authored
    /// before the guide rewrite pass through unchanged.
    #[dialog_common::test]
    async fn it_still_accepts_pascal_case_type_names_for_back_compat() {
        let syntax = must_parse(
            r#"
attribute!: &age
  the:         x.y/age
  as:          UnsignedInteger
  cardinality: one
  description: "x"
"#,
        );
        analyze(&syntax, &NoopResolver).await.unwrap();
    }

    /// An unknown type name surfaces a guiding error listing the
    /// accepted forms.
    #[dialog_common::test]
    async fn it_rejects_unknown_type_name_with_guidance() {
        let syntax = must_parse(
            r#"
attribute!: &age
  the:         x.y/age
  as:          quaternion
  cardinality: one
  description: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("quaternion")
                    && reason.contains("text")
            ),
            "expected InvalidAttributeBody listing accepted types, got {err:?}"
        );
    }

    /// Cardinality also has a normalization layer — only `one`
    /// and `many` are accepted (PascalCase forms also work for
    /// back-compat).
    #[dialog_common::test]
    async fn it_rejects_unknown_cardinality_name() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/foo
  as:          text
  cardinality: maybe
  description: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("maybe")
            ),
            "expected InvalidAttributeBody listing accepted cardinality, got {err:?}"
        );
    }

    // ----------------------------------------------------------- //
    // Stage 2.7 / 2.8 — retraction emission                       //
    // ----------------------------------------------------------- //

    /// `field: _` (per-field blank) routes the affected field
    /// to the retract side. The other fields the user wrote
    /// continue to assert. One expression → two statements.
    #[dialog_common::test]
    async fn it_splits_field_blank_into_assert_and_retract() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  name: "Alice"
  age:  _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // Two statements: retract first (drop age), then assert
        // (set name).
        assert_eq!(analysis.mutate.statements.len(), 2);
        assert!(matches!(
            &analysis.mutate.statements[0],
            Statement::Retract(_)
        ));
        assert!(matches!(
            &analysis.mutate.statements[1],
            Statement::Assert(_)
        ));
    }

    /// `..: _` plus an explicit field — the explicit field
    /// asserts; every other `with:` attribute retracts.
    #[dialog_common::test]
    async fn it_splits_dotdot_rest_marker_into_assert_and_retract() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  name: "Alice"
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // Two statements — retract (drop age, the unmentioned
        // field), assert (set name).
        assert_eq!(analysis.mutate.statements.len(), 2);
        let Statement::Retract(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Retract first");
        };
        // Retract side: name is blank (not retracted because
        // user set it), age is blank (slated for retraction).
        // The planner walks the branch to find concrete values.
        assert!(matches!(
            q.terms.get("age"),
            Some(Term::Variable { name: None, .. })
        ));
    }

    /// Pure-retract body (no explicit asserted fields) → only
    /// `Statement::Retract`. No `&anchor`, so no name to
    /// publish either.
    #[dialog_common::test]
    async fn it_emits_only_retract_for_pure_retraction_body() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(matches!(
            &analysis.mutate.statements[0],
            Statement::Retract(_)
        ));
    }

    /// Pure-assert body (no blanks, no `..: _`) → only
    /// `Statement::Assert`. Confirms the new logic doesn't
    /// regress the common case.
    #[dialog_common::test]
    async fn it_emits_only_assert_for_pure_assertion_body() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(matches!(
            &analysis.mutate.statements[0],
            Statement::Assert(_)
        ));
    }

    // ----------------------------------------------------------- //
    // Description-field shape acceptance                          //
    // ----------------------------------------------------------- //

    /// Bare unquoted multi-word description on `attribute!` —
    /// the parser classifies it as a string literal (uppercase
    /// + space outside the symbol charset), and the analyzer
    /// accepts it without quotes.
    #[dialog_common::test]
    async fn it_accepts_bare_multi_word_description_on_attribute() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name
"#,
        );
        analyze(&syntax, &NoopResolver).await.unwrap();
    }

    /// Bare unquoted multi-word description on `concept!`.
    #[dialog_common::test]
    async fn it_accepts_bare_multi_word_description_on_concept() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: A person
  with:
    name:
      description: Name of the person
      the:         x.y/name
      as:          text
      cardinality: one
"#,
        );
        analyze(&syntax, &NoopResolver).await.unwrap();
    }

    /// A single bare lowercase token in `description:` (`recipe`)
    /// is parsed as a `Symbol` — the analyzer rejects it to push
    /// authors toward writing a prose description rather than
    /// repeating the concept's name. Quote it (`description:
    /// "recipe"`) to override.
    #[dialog_common::test]
    async fn it_rejects_bare_symbol_description_to_encourage_prose() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/foo
  as:          text
  cardinality: one
  description: recipe
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("recipe") && reason.contains("symbol")
            ),
            "expected guidance about bare symbol in `description:`, got {err:?}"
        );
    }

    /// Same rule on `concept!`'s `description:` field — Symbol
    /// rejected, prose required.
    #[dialog_common::test]
    async fn it_rejects_bare_symbol_description_on_concept() {
        let syntax = must_parse(
            r#"
concept!: &thing
  description: recipe
  with:
    title:
      description: A short title
      the:         x.y/title
      as:          text
      cardinality: one
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("recipe")
            ),
            "expected guidance about bare symbol in `description:`, got {err:?}"
        );
    }

    /// Quoted single-word description is fine — quotes signal
    /// the author meant the literal string.
    #[dialog_common::test]
    async fn it_accepts_quoted_single_word_description() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/foo
  as:          text
  cardinality: one
  description: "recipe"
"#,
        );
        analyze(&syntax, &NoopResolver).await.unwrap();
    }

    // ----------------------------------------------------------- //
    // IncompleteAssertion check                                   //
    // ----------------------------------------------------------- //

    /// `person!:\n  this: ?alice\n  age: 29` with no preceding
    /// query that binds `?alice` is the gotcha case — the user
    /// almost certainly meant to update Alice but the analyzer
    /// has no way to know that. Mints a fresh entity with only
    /// `age` set, which is meaningless. Caught with
    /// `IncompleteAssertion`.
    #[dialog_common::test]
    async fn it_rejects_partial_assertion_with_unbound_this_variable() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::IncompleteAssertion { concept, set, missing, selector_form }
                    if concept == "person"
                       && set == &vec!["age".to_string()]
                       && missing == &vec!["name".to_string()]
                       && selector_form.contains("?alice")
            ),
            "expected IncompleteAssertion for `?alice` + age-only body, got {err:?}"
        );
    }

    /// Same body shape, but a preceding query binds `?alice`.
    /// Now the partial assertion is intentional (update an
    /// existing entity), so the check is suppressed.
    #[dialog_common::test]
    async fn it_allows_partial_assertion_when_query_binds_this() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
person!:
  this: ?alice
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        analyze(&syntax, &resolver).await.unwrap();
    }

    /// Omitted `this:` (anonymous body-derive) plus an
    /// incomplete body — the same "ghost entity" mistake but
    /// without the variable. Caught with the same error.
    #[dialog_common::test]
    async fn it_rejects_partial_assertion_with_omitted_this() {
        let syntax = must_parse(
            r#"
person!:
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::IncompleteAssertion { selector_form, .. }
                    if selector_form.contains("omitted")
            ),
            "expected IncompleteAssertion for omitted `this:` + partial body, got {err:?}"
        );
    }

    /// Anchor doesn't rescue an incomplete assertion. The
    /// anchor publishes a name *for* a body-derived entity, but
    /// the body still has the ghost-entity problem.
    #[dialog_common::test]
    async fn it_rejects_partial_assertion_even_with_anchor() {
        let syntax = must_parse(
            r#"
person!: &alice
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::IncompleteAssertion { .. }),
            "anchor should not bypass the check, got {err:?}"
        );
    }

    /// `..: _` is the explicit opt-in for "I know this is
    /// partial." Accepted.
    #[dialog_common::test]
    async fn it_allows_partial_assertion_with_rest_marker() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  age: 29
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        analyze(&syntax, &resolver).await.unwrap();
    }

    /// Setting every `with:` field is intentional — pass.
    #[dialog_common::test]
    async fn it_allows_full_assertion_with_unbound_this() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  name: "Alice"
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        analyze(&syntax, &resolver).await.unwrap();
    }

    /// `this: did:key:…` (URI form) is always assumed
    /// intentional — the user wrote a concrete entity URI, so
    /// the partial-body check doesn't fire.
    #[dialog_common::test]
    async fn it_allows_partial_assertion_when_this_is_uri() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        analyze(&syntax, &resolver).await.unwrap();
    }

    /// Per-field `_` retraction on a body that *only* sets `_`
    /// blanks — same as omitting fields entirely. The user is
    /// trying to drop a field on a fresh entity, which is
    /// nonsensical. Caught.
    #[dialog_common::test]
    async fn it_rejects_field_retraction_on_unbound_entity() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  age: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::IncompleteAssertion { .. }),
            "expected IncompleteAssertion for `age: _` on unbound entity, got {err:?}"
        );
    }

    /// AnalyzeError carries a stable code per kind that an editor
    /// can match against without parsing the human message.
    #[dialog_common::test]
    async fn it_exposes_stable_codes_on_errors() {
        let syntax = must_parse("person!:\n");
        let resolver = NoopResolver;
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert_eq!(err.code(), "E_ASSERTION_WITHOUT_FIELDS");
    }

    /// AnalyzeError carries a source range for errors with a
    /// clear surface-syntax origin. This lets the LSP attach the
    /// diagnostic to the offending span instead of the whole
    /// document.
    #[dialog_common::test]
    async fn it_attaches_source_ranges_to_errors() {
        // `person!:` (empty body) — the range should land on the
        // head, which sits on the second line of the document.
        let syntax = must_parse("\nperson!:\n");
        let resolver = NoopResolver;
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        let range = err
            .range
            .expect("AssertionWithoutFields should carry a range");
        assert_eq!(
            range.start.line, 1,
            "expected the head's line, got {range:?}"
        );
    }

    /// Unknown-field errors point at the offending field name,
    /// not the head — squiggle on `bogus`, not on `person!`.
    #[dialog_common::test]
    async fn it_attaches_field_range_to_unknown_field() {
        let syntax = must_parse(
            r#"
person:
  bogus: ?value
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert_eq!(err.code(), "E_UNKNOWN_FIELD");
        let range = err.range.expect("UnknownField should carry a range");
        // `bogus:` is on line 2 (0-indexed) of the doc — the
        // head is on line 1.
        assert_eq!(range.start.line, 2, "expected `bogus:` line, got {range:?}");
    }

    /// Category 2 from the user's classification: assertion
    /// updates an existing entity (URI). The analyzer
    /// synthesizes an implicit query for that URI so the editor
    /// can render before/after — even when no explicit query
    /// was written.
    #[dialog_common::test]
    async fn it_synthesizes_implicit_query_for_uri_targeted_assertion() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: 30
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let synthesized = analysis
            .query
            .as_ref()
            .expect("auto-snapshot should populate analysis.query")
            .synthesized
            .as_slice();
        assert_eq!(
            synthesized.len(),
            1,
            "expected one synthesized snapshot for the touched URI"
        );
    }

    /// Category 1 from the user's classification: assertion
    /// defines a new (potentially redundant) thing. The
    /// body-derived entity is a known constant after Phase 3,
    /// so the auto-snapshot also fires — useful for surfacing
    /// "you re-asserted the same fields" in the after-view.
    #[dialog_common::test]
    async fn it_synthesizes_implicit_query_for_body_derived_assertion() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(
            analysis
                .query
                .as_ref()
                .map(|q| q.synthesized.len())
                .unwrap_or(0),
            1,
            "expected one synthesized snapshot for the body-derived entity"
        );
    }

    /// When the user wrote a query covering the touched URI,
    /// the synthesizer skips it — no duplicate snapshot.
    #[dialog_common::test]
    async fn it_skips_implicit_query_when_user_wrote_one() {
        let syntax = must_parse(
            r#"
person:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv

person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: 30
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        // Only the user's explicit query — no synthesized
        // duplicate for the already-covered URI.
        let query = analysis.query.as_ref().unwrap();
        assert_eq!(query.queries.len(), 1);
        assert!(
            query.synthesized.is_empty(),
            "user query covers the URI — synthesis must skip it"
        );
    }

    /// Pure-query documents skip the synthesis entirely —
    /// nothing to snapshot if nothing's being written.
    #[dialog_common::test]
    async fn it_skips_synthesis_for_pure_query_documents() {
        let syntax = must_parse(
            r#"
person:
  name: ?n
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(analysis.mutate.statements.is_empty());
        assert_eq!(
            analysis
                .query
                .as_ref()
                .map(|q| q.queries.len())
                .unwrap_or(0),
            1,
            "only the user's query — synthesis must be a no-op"
        );
    }

    /// Category 1 sub-case: the user wrote `this: ?alice` but
    /// `?alice` is unbound. `this_term_for_assertion` mints a
    /// body-derived entity for it and registers ?alice → entity,
    /// so the implicit query targets that entity. This is the
    /// "we know what we're about to write, so we can show what's
    /// there before we write it" case.
    #[dialog_common::test]
    async fn it_synthesizes_implicit_query_for_unbound_this_variable() {
        let syntax = must_parse(
            r#"
person!:
  this: ?alice
  name: "Alice"
  age: 29
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(
            analysis
                .query
                .as_ref()
                .map(|q| q.synthesized.len())
                .unwrap_or(0),
            1,
            "unbound `?alice` should still seed a synthesized snapshot for the body-derived entity"
        );
    }

    /// Meta-head declarations (`attribute!` / `concept!`) don't
    /// get auto-snapshots. Their state lives in the schema
    /// branch; user-facing snapshots would surface schema noise.
    #[dialog_common::test]
    async fn it_does_not_synthesize_implicit_queries_for_meta_heads() {
        let syntax = must_parse(
            r#"
attribute!:
  the: io.gozala.person/nickname
  as: text
  cardinality: one
  description: "Optional short name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // The declaration produces a Statement::Assert but
        // synthesizer skips it — no implicit query.
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(
            analysis.query.is_none(),
            "meta-head declarations must not seed an auto-snapshot, got {:?}",
            analysis.query
        );
    }
}
