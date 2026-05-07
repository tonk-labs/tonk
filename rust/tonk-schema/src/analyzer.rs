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

mod assertion;
mod declaration;
mod error;
mod field;
mod query;
mod resolver;
mod scope;

use std::collections::{HashMap, HashSet};

use dialog_common::ConditionalSync;
use tonk_notation::{Expression, HeadName, Syntax};

use crate::transact::{
    Analysis, Application, MutationAnalysis, QueryAnalysis, Statement, ThisIntent,
};

pub use error::AnalyzeError;
pub use resolver::{NoopResolver, ResolvedAttribute, ResolvedConcept, Resolver, ResolverError};

use assertion::{build_assertion_application, derive_head_intent};
use declaration::{
    DeclaredApplication, attribute_application, concept_application, parse_attribute_body,
    parse_concept_body,
};
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
        return Err(AnalyzeError::EmptyDocument);
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
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref())?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
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
                    };
                    let (this, name) =
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref())?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
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
                    let application = concept_application(&plan.descriptor, &entity, name);
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

        // Non-meta heads: nothing to do in Phase 1.
        // Anchor and variable forms both produce body-content-
        // derived entities, which Phase 3 computes when it runs
        // `this_term_for_assertion`.
    }

    // ---- Phase 2: build query Applications ----
    let mut analysis = Analysis {
        declarations: scope.declarations.lock().clone(),
        variables: scope.variables.lock().clone(),
        ..Analysis::default()
    };

    let mut queries: Vec<Application> = Vec::new();
    for expression in &syntax.expressions {
        if let Expression::Query(q) = expression {
            let application = build_query_application(q, &scope, &analysis).await?;
            queries.push(application);
        }
    }
    if !queries.is_empty() {
        analysis.query = Some(QueryAnalysis { queries });
    }

    // ---- Phase 3: build mutation Statements ----
    let mut statements: Vec<Statement> = Vec::new();
    let mut requires: HashSet<String> = HashSet::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        match expression {
            Expression::Query(_) => {}
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
                        statements.push(Statement::Assert(inline));
                    }
                    collect_unbound_variables(&declaration.application, &analysis, &mut requires);
                    statements.push(Statement::Assert(declaration.application));
                } else {
                    let application = build_assertion_application(a, &scope, &mut analysis).await?;
                    collect_unbound_variables(&application, &analysis, &mut requires);
                    statements.push(Statement::Assert(application));
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
                return Err(AnalyzeError::UnboundMutationVariable { name: name.clone() });
            }
        }
    } else if !requires.is_empty() {
        let name = requires.iter().next().cloned().unwrap_or_default();
        return Err(AnalyzeError::UnboundMutationVariable { name });
    }

    analysis.mutate = MutationAnalysis {
        statements,
        requires,
    };

    Ok(analysis)
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
        assert!(matches!(err, AnalyzeError::EmptyDocument));
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
            matches!(&err, AnalyzeError::InvalidAttributeBody { reason } if reason.contains("description")),
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
            matches!(err, AnalyzeError::DuplicateName { .. }),
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
        assert!(matches!(err, AnalyzeError::NameShadowing { .. }));
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
        assert!(matches!(err, AnalyzeError::UnboundMutationVariable { .. }));
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
        assert_eq!(analysis.mutate.statements.len(), 1);
        // Stage 2.1: `..: _` is parsed but not yet wired to
        // produce `Statement::Retract`. The expression goes
        // through as `Statement::Assert` whose every field is
        // blank because the user supplied no values. Stage 2.8
        // turns this into a real retraction.
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept) — Stage 2.1 placeholder");
        };
        // `this` is bound; all other fields are blank.
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
        assert!(matches!(err, AnalyzeError::UnknownConcept { .. }));
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
        assert!(matches!(err, AnalyzeError::ClaimWithoutFields { .. }));
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
}
