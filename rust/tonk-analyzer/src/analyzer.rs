//! Analyzer — turns a [`tonk_notation::Syntax`] tree into an
//! [`Analysis<Syntax>`][crate::analysis::Analysis] tree ready for
//! evaluation against a branch.
//!
//! See `analysis-spec.md` (sibling to this crate) and
//! `plan/runtime.md` for the full design. Analysis runs in two
//! named sub-phases — [`resolve`] then [`expand`]:
//!
//! - **resolve** — walk the document, bind every concept /
//!   attribute reference through the [`Scope`]'s resolution
//!   chain (which calls into [`tonk_schema::resolution`] with
//!   the per-execution `env`), record content-derived entities
//!   into `declarations` (anchor-form heads) and `variables`
//!   (variable-form heads), and scan for diagnostics. For
//!   `attribute!` / `concept!` heads the body is parsed here so
//!   the descriptor's content-addressed entity is known up
//!   front. Output keeps the source shape.
//! - **expand** — lower notation sugar into kernel-shaped
//!   claims: a domain predicate becomes an anonymous concept, an
//!   `&anchor` pairs with a built-in `Name` assert, an omitted
//!   `this:` is injected as `id:<body-digest>`. This builds the
//!   query [`Application`]s, the mutation [`Statement`]s, the
//!   `rule!:`-to-[`Statement::InstallEffect`] lift, and the
//!   implicit snapshot queries.
//!
//! Sub-modules:
//! - [`error`] — [`AnalyzeError`] enum
//! - [`scope`] — in-document name index used during analysis,
//!   plus the `resolve_*` helpers that fall through to the
//!   live source / env pair
//! - [`declaration`] — `attribute!` / `concept!` body parsing +
//!   their `Application` builders
//! - [`query`] — `build_query_application`
//! - [`assertion`] — `build_assertion_application`,
//!   `derive_head_intent`
//! - [`field`] — field-value translation, scalar coercion,
//!   small utilities

mod assertion;
mod constraint;
mod declaration;
mod error;
mod field;
mod formula;
mod graph;
mod query;
mod resolver_registry;
mod rule;
mod scan;
mod scope;

use std::collections::{HashMap, HashSet};

use dialog_common::ConditionalSync;
use tonk_notation::{Effectful, Expression, HeadName, Syntax};

use tonk_schema::transact::{Application, DomainApplication, Statement, ThisIntent};

use crate::analysis::{
    Analysis as Tree, AssertionAnalysis, AssertionNode, DocumentAnalysis, ExpressionAnalysis,
    Predicate, QueryNode, QueryNodeAnalysis,
};
use tonk_core::claim::ConceptDescriptor;

pub use constraint::{ConstraintCompletion, constraint_completions, notation_form};
pub use error::{
    AnalyzeDiagnostic, AnalyzeDiagnosticKind, AnalyzeError, AnalyzeErrorKind, DiagnosticSeverity,
};
pub use formula::{FormulaCompletion, formula_completions};
pub use resolver_registry::{
    ResolverCompletion, ResolverOperand, resolver_completions, resolver_operands,
};
pub use rule::builtin_kind;
pub use scan::scan_variables;

use tonk_schema::concept::QueryEnv;
use tonk_schema::query_source::Source;

use assertion::build_assertion_application;
use dialog_artifacts::Entity;
use field::collect_unbound_variables;
use query::build_query_application;
use scope::Scope;

/// The analyzer's per-pass working state — the scratch `expand`
/// reads and mutates as it walks the document.
///
/// Not the analyzer's product (that is the [`Analysis<Syntax>`][Tree]
/// tree); this is internal accumulator state. `resolve` seeds
/// `declarations` / `variables`; `expand`'s query pass fills
/// `queries`, and its mutation pass reads all three.
#[derive(Debug, Default)]
pub(crate) struct Working {
    /// `name` → entity. Anchor-form heads. Seeded by `resolve`.
    pub declarations: HashMap<String, Entity>,
    /// `?foo` → entity. Variable-form heads with content-derived
    /// entities. Seeded by `resolve`.
    pub variables: HashMap<String, Entity>,
    /// The user-written query [`Application`]s built by `expand`'s
    /// query pass, in document order. Read by the mutation pass
    /// to decide which `?var` references a preceding query binds.
    pub queries: Vec<Application>,
}

impl Working {
    /// User-named variable slots the document's queries bind at
    /// evaluation time; auto-generated `__N` names are excluded.
    pub fn query_bindings(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for application in &self.queries {
            for name in application.bindings() {
                if !name.starts_with("__") {
                    out.insert(name);
                }
            }
        }
        out
    }
}

/// Stage analysis of a parsed [`Syntax`] tree against the given
/// [`Source`]. Returns a chain handle; call `.perform(env)` to
/// run the two sub-phases and yield the [`Analysis<Syntax>`][Tree].
///
/// Source goes in at construction (a borrowed branch / txn handle
/// that outlives the call); env is supplied per execution, the
/// dialog idiom. The chain handle holds nothing else.
pub fn analyze<'s, 'a>(syntax: &'s Syntax, source: Source<'a>) -> Analyze<'s, 'a> {
    Analyze { syntax, source }
}

/// Analyze a self-contained document with no running system —
/// every reference must resolve against the document's own
/// `concept!` / `attribute!` / `&anchor` definitions (plus
/// builtins). A reference that would need a branch lookup surfaces
/// as an unknown-concept / unknown-bookmark error rather than
/// being resolved.
///
/// Synchronous: the resolution plumbing is run with no `env`, so
/// it never awaits a branch query. Used by the compile-time
/// `claim!` macro, which has no running system to resolve against.
pub fn analyze_local(syntax: &Syntax) -> Result<Tree<Syntax>, AnalyzeError> {
    if syntax.expressions.is_empty() {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::EmptyDocument,
            syntax.range,
        ));
    }

    // push → resolve(LocalOnly) → build. The graph's resolve phase
    // is genuinely synchronous when the resolver does no IO:
    // `LocalOnly` answers every external need with `None` without
    // awaiting, so the future is `Ready` on first poll. Drive it
    // with a no-op waker rather than pull in an executor — but
    // unlike the old path this is a property of `LocalOnly`, not a
    // fragile assumption about every prefetch short-circuiting.
    let scope = Scope::new();
    let graph = graph::push(syntax)?;
    let resolved = poll_ready(graph.resolve(syntax, &scope, &graph::LocalOnly))?;
    rule::check_overlapping_transient_rule_triggers(syntax, &scope)?;
    expand(syntax, &scope, resolved)
}

/// Drive a future that performs no real IO to completion on the
/// current thread. The only caller is [`analyze_local`] with a
/// [`graph::LocalOnly`] resolver, whose every method returns
/// immediately — so the future is `Ready` on first poll. Panics if
/// it ever yields `Pending`, which would mean a resolver awaited IO
/// it shouldn't have.
fn poll_ready<F: std::future::Future<Output = Result<graph::Resolved, AnalyzeError>>>(
    fut: F,
) -> Result<graph::Resolved, AnalyzeError> {
    use std::pin::pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    let mut fut = pin!(fut);
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(result) => result,
        Poll::Pending => {
            unreachable!("local analysis must not await: LocalOnly resolver never hits the branch")
        }
    }
}

/// Chain handle returned by [`analyze`]. Holds the syntax and the
/// source until [`perform`](Self::perform) consumes them with an
/// `env`.
pub struct Analyze<'s, 'a> {
    syntax: &'s Syntax,
    source: Source<'a>,
}

impl<'s, 'a> Analyze<'s, 'a> {
    /// Run `resolve` + `expand` against the source and env,
    /// yielding the document's [`Analysis<Syntax>`][Tree].
    ///
    /// `Env: QueryEnv + ConditionalSync` works on both native and
    /// wasm: [`ConditionalSync`] expands to `Send + Sync` on
    /// native (so async-trait-generated futures stay `Send` for
    /// axum handlers) and to nothing on wasm (single-threaded
    /// runtime).
    pub async fn perform<Env: QueryEnv + ConditionalSync>(
        self,
        env: &Env,
    ) -> Result<Tree<Syntax>, AnalyzeError> {
        let Analyze { syntax, source } = self;
        if syntax.expressions.is_empty() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::EmptyDocument,
                syntax.range,
            ));
        }

        let scope = Scope::new();
        let graph = graph::push(syntax)?;
        let resolver = graph::BranchResolver::new(source, env);
        let resolved = graph.resolve(syntax, &scope, &resolver).await?;
        rule::check_overlapping_transient_rule_triggers(syntax, &scope)?;
        expand(syntax, &scope, resolved)
    }
}

/// **expand** — lower the resolved document's notation sugar into
/// kernel-shaped claims and assemble the [`Analysis<Syntax>`][Tree]
/// tree. Three passes over the document, then snapshot synthesis:
///
/// 1. **query** — build a query [`Application`] per query
///    expression.
/// 2. **mutation** — build the desugared mutation [`Statement`]s
///    per assertion: domain predicate → anonymous concept,
///    `&anchor` → paired `Name` assert, omitted `this:` → injected
///    `id:<body-digest>`. `attribute!` / `concept!` heads emit the
///    `Application` `resolve` already built.
/// 3. **rule** — lift each `rule!:` into a
///    [`Statement::InstallEffect`].
///
/// Then [`synthesize_implicit_queries`] fills the implicit
/// snapshot queries. Every lowering is terminal — it emits only
/// resolved entities and substituted terms — so `expand`'s output
/// never needs re-resolution.
fn expand(
    syntax: &Syntax,
    scope: &Scope,
    resolved: graph::Resolved,
) -> Result<Tree<Syntax>, AnalyzeError> {
    let graph::Resolved { mut declared } = resolved;
    let diagnostics = scan::scan_variables(syntax);

    // The `Working` scratch carries the cross-pass accumulator
    // state: the query and mutation passes read it as they build
    // (`build_query_application`, `build_assertion_application`,
    // `collect_unbound_variables` all take `&Working`). The
    // per-expression results are captured alongside into tree
    // nodes, indexed by source expression; the `Analysis<Syntax>`
    // tree is assembled once the passes finish. `declarations` /
    // `variables` are seeded from the scope `resolve` filled.
    let mut working = Working {
        declarations: scope.declarations.lock().clone(),
        variables: scope.variables.lock().clone(),
        queries: Vec::new(),
    };

    // Per-expression tree node, by source-expression index.
    let mut nodes: Vec<Option<ExpressionAnalysis>> =
        (0..syntax.expressions.len()).map(|_| None).collect();

    // ---- query pass: build query Applications ----
    for (index, expression) in syntax.expressions.iter().enumerate() {
        if let Expression::Query(q) = expression {
            let application = build_query_application(q, scope, &working)?;
            working.queries.push(application.clone());
            nodes[index] = Some(ExpressionAnalysis::Query(Box::new(Tree {
                source: QueryNode { source: q.clone() },
                analysis: QueryNodeAnalysis {
                    application,
                    label: q.predicate.source.clone(),
                },
            })));
        }
    }

    // ---- mutation pass: build mutation Statements ----
    let mut requires: HashSet<String> = HashSet::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        match expression {
            Expression::Query(_) => {}
            Expression::Claim(Effectful {
                anchor: anchor_node,
                inner: a,
            }) => {
                let mut claims: Vec<Statement> = Vec::new();
                let mut claim_labels: Vec<Option<String>> = Vec::new();
                let predicate;
                let this;
                let anchor;
                let mut transient_entity: Option<Entity> = None;
                let mut rule_effect: Option<dialog_query::InductiveRule> = None;
                let is_declaration;

                if let Some(declaration) = declared.remove(&index) {
                    // `attribute!` / `concept!` head — `resolve`
                    // already built the application. Inline
                    // attribute definitions inside a `concept!`'s
                    // `with:` map are emitted as their own
                    // assertions *before* the concept itself, so
                    // the attribute facts are present on the
                    // branch (queryable via `attribute:`) by the
                    // time anything reads back.
                    for inline in declaration.inline_attributes {
                        collect_unbound_variables(&inline, &working, &mut requires);
                        claims.push(Statement::Assert(inline));
                        claim_labels.push(None);
                    }
                    // `predicate`/`this`/`anchor` describe the head for
                    // the analysis tree. A normal declaration derives
                    // them from its asserted application; a
                    // retraction-only `concept!:` body has no
                    // assertion, so probe its first retraction
                    // application instead.
                    let probe = declaration
                        .application
                        .as_ref()
                        .or(declaration.retractions.first());
                    // A declaration head applies the built-in
                    // `concept` / `attribute` schema — that schema
                    // is durable; the declared concept's own
                    // transience is a field of the body, not the
                    // predicate this assertion applies.
                    predicate = probe
                        .map(|app| predicate_of(app, false))
                        .unwrap_or(Predicate::Domain(a.predicate.source.clone()));
                    this = probe
                        .map(|app| app.this().clone())
                        .unwrap_or(ThisIntent::Derived);
                    anchor = declaration
                        .application
                        .as_ref()
                        .and_then(|app| app.name().map(str::to_owned));
                    // Retractions emit first so the dissociate sees the
                    // prior state before any re-assert lands.
                    for retraction in declaration.retractions {
                        collect_unbound_variables(&retraction, &working, &mut requires);
                        claims.push(Statement::Retract(retraction));
                        claim_labels.push(None);
                    }
                    if let Some(application) = declaration.application {
                        collect_unbound_variables(&application, &working, &mut requires);
                        claims.push(Statement::Assert(application));
                        claim_labels.push(None);
                    }
                    is_declaration = true;
                } else if is_rule_claim(a) {
                    // `rule!:` claims lower to a single
                    // `Statement::Assert(Application::Rule(..))` for
                    // installs or `Statement::Retract(Application::Rule(..))`
                    // for retracts. The dispatcher in
                    // `rule::lift_rule_claim` picks by inspecting
                    // the body for `..: _`; both paths produce a
                    // `Rule` value (fresh on install, branch-resolved
                    // on retract).
                    predicate = Predicate::Domain(a.predicate.source.clone());
                    anchor = anchor_node.as_ref().map(|n| n.name.clone());
                    match rule::lift_rule_claim(a, scope, &working)? {
                        Some(rule::RuleAction::Install { rule }) => {
                            // Rules are content-addressed, so the
                            // install always lands at the rule's own
                            // content-derived `this()`.
                            let intent = ThisIntent::Derived;
                            this = intent.clone();
                            rule_effect = Some((*rule).clone());
                            claims
                                .push(Statement::Assert(Application::Rule { rule, this: intent }));
                            claim_labels.push(None);
                        }
                        Some(rule::RuleAction::Retract { rule, this: entity }) => {
                            let intent = ThisIntent::Uri(entity);
                            this = intent.clone();
                            claims
                                .push(Statement::Retract(Application::Rule { rule, this: intent }));
                            claim_labels.push(None);
                        }
                        Some(rule::RuleAction::InstallDeductive { rule }) => {
                            let intent = ThisIntent::Derived;
                            this = intent.clone();
                            claims.push(Statement::Assert(Application::DeductiveRule {
                                rule,
                                this: intent,
                            }));
                            claim_labels.push(None);
                        }
                        Some(rule::RuleAction::RetractDeductive { rule, this: entity }) => {
                            let intent = ThisIntent::Uri(entity);
                            this = intent.clone();
                            claims.push(Statement::Retract(Application::DeductiveRule {
                                rule,
                                this: intent,
                            }));
                            claim_labels.push(None);
                        }
                        None => {
                            // Retract of a rule that isn't installed:
                            // silently no-op, matching the prior
                            // RetractEffect behaviour. The notation
                            // still parses and analyzes; the analyzer
                            // just emits no claim.
                            this = a
                                .fields
                                .iter()
                                .find(|f| f.name == "this")
                                .and_then(|f| match &f.value {
                                    tonk_notation::FieldValue::Uri(s) => s.parse().ok(),
                                    _ => None,
                                })
                                .map(ThisIntent::Uri)
                                .unwrap_or(ThisIntent::Derived);
                        }
                    }
                    is_declaration = false;
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
                    // Anchor is passed separately because the
                    // syntax `Application` no longer carries it
                    // (it lives on the surrounding `Effectful`).
                    // For now we thread it through where needed
                    // and leave `build_assertion_application`'s
                    // signature unchanged — the body still reads
                    // `assertion.fields` only, since the anchor
                    // affects naming downstream.
                    let plan =
                        build_assertion_application(a, anchor_node.as_ref(), scope, &mut working)?;
                    let probe = plan.assert.as_ref().or(plan.retract.as_ref());
                    predicate = probe
                        .map(|app| predicate_of(app, plan.transient))
                        .unwrap_or(Predicate::Domain(a.predicate.source.clone()));
                    this = probe
                        .map(|app| app.this().clone())
                        .unwrap_or(ThisIntent::Derived);
                    anchor = anchor_node.as_ref().map(|n| n.name.clone());
                    if let Some(retract_app) = plan.retract {
                        collect_unbound_variables(&retract_app, &working, &mut requires);
                        claims.push(Statement::Retract(retract_app));
                        claim_labels.push(Some(a.predicate.source.clone()));
                    }
                    if let Some(assert_app) = plan.assert {
                        collect_unbound_variables(&assert_app, &working, &mut requires);
                        // A transient-concept assertion: record the
                        // concept entity so the evaluator buckets
                        // its claims for the effects fixpoint.
                        if plan.transient
                            && let Application::Concept { query, .. } = &assert_app
                        {
                            transient_entity = Some(query.predicate.this());
                        }
                        claims.push(Statement::Assert(assert_app));
                        claim_labels.push(Some(a.predicate.source.clone()));
                    }
                    is_declaration = false;
                }

                nodes[index] = Some(ExpressionAnalysis::Assertion(Box::new(Tree {
                    source: AssertionNode {
                        source: a.clone(),
                        anchor: anchor_node.clone(),
                    },
                    analysis: AssertionAnalysis {
                        predicate,
                        this,
                        anchor,
                        claims,
                        declaration: is_declaration,
                        labels: claim_labels,
                        transient: transient_entity,
                        effect: rule_effect,
                    },
                })));
            }
        }
    }

    // (Rule lifting now happens inside the Claim arm above:
    // `rule!:` is a Claim with predicate `rule`; the analyzer
    // dispatches on the predicate name via `is_rule_claim`.)

    // requires must be subset of query bindings (analysis-time
    // variables already filtered out by `collect_unbound_variables`).
    if working.queries.is_empty() {
        if let Some(name) = requires.iter().next().cloned() {
            return Err(AnalyzeErrorKind::UnboundMutationVariable { name }.into());
        }
    } else {
        let bindings = working.query_bindings();
        for name in &requires {
            if !bindings.contains(name) {
                return Err(
                    AnalyzeErrorKind::UnboundMutationVariable { name: name.clone() }.into(),
                );
            }
        }
    }

    // Assemble the document tree, in source order.
    let mut expressions: Vec<Tree<Expression>> = Vec::new();
    for (expression, node) in syntax.expressions.iter().zip(nodes) {
        let analysis = node.expect("every expression is analyzed by the expand passes");
        expressions.push(Tree {
            source: expression.clone(),
            analysis,
        });
    }
    let mut document = DocumentAnalysis {
        expressions,
        synthesized: Vec::new(),
        declarations: working.declarations,
        variables: working.variables,
        diagnostics,
    };

    // ---- snapshot synthesis: implicit snapshot queries ----
    // Reads the assembled tree's statements and the user queries,
    // fills `document.synthesized`.
    synthesize_implicit_queries(&mut document);

    Ok(Tree {
        source: syntax.clone(),
        analysis: document,
    })
}

/// Classify an [`Application`]'s head as a [`Predicate`] for the
/// tree's [`AssertionAnalysis`]. `Concept` carries the resolved
/// descriptor tagged with its true durability — `transient` is
/// the flag the analyzer recovered from the head concept's
/// `dialog.concept/transient` marker. `Domain` carries the claim
/// domain prefix; a domain head names no concept, so it is
/// always durable (and the [`Predicate::Domain`] arm carries no
/// descriptor anyway).
/// `true` when a claim's predicate is the built-in `rule` concept.
/// `rule!:` is structurally a [`Expression::Claim`] over `rule`;
/// the analyzer dispatches on the predicate to decide whether to
/// lift the body into an inductive rule (via [`rule::lift_rule`])
/// or treat it as a regular concept assertion.
fn is_rule_claim(application: &tonk_notation::Application) -> bool {
    matches!(&application.predicate.name, HeadName::Concept(name) if name == "rule")
}

fn predicate_of(application: &Application, transient: bool) -> Predicate {
    match application {
        Application::Concept { query, .. } => {
            let descriptor = query.predicate.clone();
            Predicate::Concept(if transient {
                ConceptDescriptor::Transient(descriptor)
            } else {
                ConceptDescriptor::Durable(descriptor)
            })
        }
        Application::Domain { application, .. } => Predicate::Domain(application.domain.clone()),
        Application::Rule { .. } | Application::DeductiveRule { .. } => {
            Predicate::Domain("rule".to_owned())
        }
        Application::Resolver { query, .. } => Predicate::Domain(query.name().to_owned()),
    }
}

/// Synthesize snapshot queries so every mutation-touched entity
/// gets read back into the response, even when the user wrote no
/// explicit query for it. Fills [`DocumentAnalysis::synthesized`].
///
/// Skipped entirely for `attribute!` / `concept!` declarations
/// (the meta heads — their state is in the schema branch, not
/// user-visible facts).
///
/// Entities that already appear as a `this:` constant in some
/// existing query are skipped: the user query will pick them up.
fn synthesize_implicit_queries(document: &mut DocumentAnalysis) {
    use crate::analysis::SynthesizedQuery;
    use dialog_artifacts::Value;
    use dialog_query::Term;

    let statements = document.statements();
    if statements.is_empty() {
        return;
    }

    // Existing query coverage: which entities (by their string
    // form) does some existing query enumerate via a constant
    // `this:` term?
    let mut covered: HashSet<String> = HashSet::new();
    for query in document.queries() {
        if let Some(Term::Constant(Value::Entity(e))) =
            query.analysis.application.parameters().get("this")
        {
            covered.insert(e.to_string());
        }
    }

    let mut implicit: Vec<SynthesizedQuery> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for planned in &statements {
        if planned.declaration {
            continue;
        }
        // Rules don't write a snapshotable entity — install /
        // retract of `db.effect/*` claims contributes no
        // implicit query.
        let application = planned.statement.application();
        if matches!(
            application,
            Application::Rule { .. } | Application::DeductiveRule { .. }
        ) {
            continue;
        }
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
                        attributes: d.attributes.clone(),
                    },
                    this: ThisIntent::Uri(entity),
                    name: None,
                }
            }
            // Rules were filtered out above; the rule-snapshot path
            // is the read-side `rule:` query, not a per-write
            // synthesised one.
            Application::Rule { .. } | Application::DeductiveRule { .. } => unreachable!(
                "rule applications should have been filtered out by the matches! check"
            ),
            // Snapshots read back what a MUTATION touched; a resolver
            // is read-only, so it never appears among them.
            Application::Resolver { .. } => unreachable!(
                "resolver applications are read-only and never produce a mutation to snapshot"
            ),
        };
        // Reuse the assertion's head name so the rendered
        // result block carries `person` (or whatever) instead
        // of the `?` fallback. `entity_key` (a URI) is the
        // worst-case fallback; the originating assertion's head
        // should always be present.
        let label = planned.label.clone().unwrap_or(entity_key);
        implicit.push(SynthesizedQuery {
            application: snapshot,
            label,
        });
    }

    // Snapshot queries land in `synthesized`, kept apart from the
    // user-written queries — a snapshot of a fresh assert target
    // reads a not-yet-existing entity and returns zero rows;
    // joining it into the user queries would zero the join that
    // feeds mutation planning. The renderer reads both; the
    // evaluator's planning path reads only the user queries.
    document.synthesized = implicit;
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
    use dialog_artifacts::{Entity, Value};
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_query::{ConceptDescriptor, Term, the};
    use dialog_repository::Branch;
    use tonk_core::meta::AnchorName;
    use tonk_notation::parse;
    use tonk_schema::concept::AnonymousConcept;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Fixture wrapping a fresh branch + operator. The analyzer's
    /// resolution chain talks to this branch through `Source`;
    /// tests that need a branch-published concept assert one via
    /// [`Fixture::concept`] / [`Fixture::concept_typed`].
    ///
    /// Bound needed to query *and* commit through an operator —
    /// `QueryEnv` covers the resolution chain; `Provider<Publish>`
    /// covers the commit path the test fixtures use to assert
    /// concept facts onto the branch.
    trait FixtureEnv:
        tonk_schema::concept::QueryEnv
        + dialog_query::Provider<dialog_effects::memory::Publish>
        + dialog_query::Provider<dialog_effects::archive::Import>
        + dialog_query::Provider<dialog_effects::authority::Attest>
    {
    }

    impl<T> FixtureEnv for T where
        T: tonk_schema::concept::QueryEnv
            + dialog_query::Provider<dialog_effects::memory::Publish>
            + dialog_query::Provider<dialog_effects::archive::Import>
            + dialog_query::Provider<dialog_effects::authority::Attest>
    {
    }

    /// `Op` is the concrete operator type [`test_operator_with_profile`]
    /// returns; tests build fixtures via [`new_fixture`] and
    /// never need to name it directly.
    struct Fixture<Op>
    where
        Op: FixtureEnv,
    {
        operator: Op,
        branch: Branch,
    }

    /// Open a fresh test repo with one empty `main` branch.
    async fn new_fixture() -> Fixture<impl FixtureEnv> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo
            .branch("main")
            .open()
            .perform(&operator)
            .await
            .expect("test branch opens");
        Fixture { operator, branch }
    }

    impl<Op> Fixture<Op>
    where
        Op: FixtureEnv,
    {
        /// Run the analyzer against the fixture's branch.
        async fn analyze(&self, syntax: &Syntax) -> Result<Tree<Syntax>, AnalyzeError> {
            super::analyze(syntax, Source::from(&self.branch))
                .perform(&self.operator)
                .await
        }

        /// Assert a concept on the branch by name, with each field
        /// declared as `(field, the, "Text")`. Publishes the
        /// `id:<name>` claim so the analyzer's name-based lookup
        /// can find it.
        async fn concept(&self, name: &str, fields: &[(&str, &str)]) {
            let typed: Vec<(&str, &str, &str)> =
                fields.iter().map(|(f, the)| (*f, *the, "Text")).collect();
            self.concept_typed(name, &typed).await;
        }

        /// Like [`concept`] but each field carries an explicit
        /// `as:` type name (e.g. `"UnsignedInteger"`).
        async fn concept_typed(&self, name: &str, fields: &[(&str, &str, &str)]) {
            let mut with = serde_json::Map::new();
            for (field, the, ty) in fields {
                with.insert(
                    (*field).into(),
                    serde_json::json!({ "the": the, "as": ty, "cardinality": "one" }),
                );
            }
            let descriptor: ConceptDescriptor =
                serde_json::from_value(serde_json::json!({ "with": with }))
                    .expect("descriptor JSON is well-formed");
            self.assert_concept_named(name, &descriptor).await;
        }

        /// Like [`concept_typed`] but each field carries an
        /// `optional` flag, so the registered concept exercises the
        /// optional-attribute paths (completeness, set-widening,
        /// marker round-trip).
        async fn concept_typed_optional(&self, name: &str, fields: &[(&str, &str, &str, bool)]) {
            let mut with = serde_json::Map::new();
            for (field, the, ty, optional) in fields {
                let mut spec = serde_json::json!({ "the": the, "as": ty, "cardinality": "one" });
                if *optional {
                    spec.as_object_mut()
                        .unwrap()
                        .insert("optional".into(), serde_json::Value::Bool(true));
                }
                with.insert((*field).into(), spec);
            }
            let descriptor: ConceptDescriptor =
                serde_json::from_value(serde_json::json!({ "with": with }))
                    .expect("descriptor JSON is well-formed");
            self.assert_concept_named(name, &descriptor).await;
        }

        /// Commit the attribute facts every field of `descriptor`
        /// references, the concept marker claim, and an
        /// `id:<name>` referent so name resolution finds the
        /// concept entity.
        async fn assert_concept_named(&self, name: &str, descriptor: &ConceptDescriptor) {
            let mut txn = self.branch.transaction();
            for (_, attr) in descriptor.with().iter() {
                let attr_entity: Entity = attr.to_uri().parse().expect("attribute URI");
                let type_label = attr
                    .content_type()
                    .and_then(|t| serde_json::to_value(t).ok())
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "Text".to_owned());
                txn = txn
                    .assert(
                        the!("db.attribute/id")
                            .of(attr_entity.clone())
                            .is(attr.the().to_string()),
                    )
                    .assert(
                        the!("db.attribute/type")
                            .of(attr_entity.clone())
                            .is(type_label),
                    )
                    .assert(
                        the!("db.attribute/cardinality")
                            .of(attr_entity.clone())
                            .is("one".to_owned()),
                    )
                    .assert(
                        the!("db.meta/description")
                            .of(attr_entity)
                            .is(String::new()),
                    );
            }
            let concept_entity = descriptor.this();
            let id_entity: Entity = format!("id:{name}").parse().expect("id:<name> parses");
            txn = txn.assert(the!("db.name/referent").of(id_entity).is(concept_entity));
            txn = txn.assert(AnonymousConcept::new(descriptor.clone()));
            txn.commit()
                .perform(&self.operator)
                .await
                .expect("concept assertion commits");
        }

        /// Assert standalone attributes on the branch and publish
        /// each under a name. Builds a throwaway concept descriptor
        /// so [`assert_concept_named`] writes the `db.attribute/
        /// {id,type,cardinality}` claims an `AttributeDefinition`
        /// needs, then publishes a `db.name/referent` for each
        /// attribute entity so a bare-symbol reference resolves it by
        /// name (mirrors `attribute!: &name` in real notation).
        ///
        /// Each entry is `(published name, field key, `the` URI,
        /// type label)`.
        async fn attributes(&self, attrs: &[(&str, &str, &str, &str)]) {
            let mut with = serde_json::Map::new();
            for (_, field, the, ty) in attrs {
                with.insert(
                    (*field).into(),
                    serde_json::json!({ "the": the, "as": ty, "cardinality": "one" }),
                );
            }
            let descriptor: ConceptDescriptor =
                serde_json::from_value(serde_json::json!({ "with": with }))
                    .expect("descriptor JSON is well-formed");
            self.assert_concept_named("__attrs", &descriptor).await;
            for (name, field, _, _) in attrs {
                let attr = descriptor
                    .with()
                    .iter()
                    .find(|(f, _)| f == field)
                    .map(|(_, a)| a)
                    .expect("field is in the descriptor");
                let entity: Entity = attr.to_uri().parse().expect("attribute URI");
                self.publish_name(name, entity).await;
            }
        }

        /// Assert a concept at an explicit pinned `entity` (rather
        /// than the content-derived `descriptor.this()`), so a test
        /// can reference it via `this: <entity>` in the notation —
        /// the form a `concept!:` field retraction targets. Each
        /// field is `(field, the, "Text")`; `optional` widens none.
        async fn assert_concept_at(&self, entity: &Entity, fields: &[(&str, &str)]) {
            let mut with = serde_json::Map::new();
            for (field, the) in fields {
                with.insert(
                    (*field).into(),
                    serde_json::json!({ "the": the, "as": "Text", "cardinality": "one" }),
                );
            }
            let descriptor: ConceptDescriptor =
                serde_json::from_value(serde_json::json!({ "with": with }))
                    .expect("descriptor JSON is well-formed");
            let mut txn = self.branch.transaction();
            for (_, attr) in descriptor.with().iter() {
                let attr_entity: Entity = attr.to_uri().parse().expect("attribute URI");
                txn = txn
                    .assert(
                        the!("db.attribute/id")
                            .of(attr_entity.clone())
                            .is(attr.the().to_string()),
                    )
                    .assert(
                        the!("db.attribute/type")
                            .of(attr_entity.clone())
                            .is("Text".to_owned()),
                    )
                    .assert(
                        the!("db.attribute/cardinality")
                            .of(attr_entity.clone())
                            .is("one".to_owned()),
                    )
                    .assert(
                        the!("db.meta/description")
                            .of(attr_entity)
                            .is(String::new()),
                    );
            }
            // Pin the concept at `entity` by emitting its facts there
            // directly (mirrors `emit_concept_facts`): the marker plus
            // one `db.concept.with/<field>` per field.
            txn = txn.assert(
                the!("db.meta/concept")
                    .of(entity.clone())
                    .is("db:concept".parse::<Entity>().expect("db:concept")),
            );
            for (field, attr) in descriptor.with().iter() {
                let attr_entity: Entity = attr.to_uri().parse().expect("attribute URI");
                let the = format!("db.concept.with/{field}")
                    .parse::<dialog_query::attribute::The>()
                    .expect("with attribute parses");
                txn = txn.assert(the.of(entity.clone()).is(attr_entity));
            }
            txn.commit()
                .perform(&self.operator)
                .await
                .expect("pinned concept assertion commits");
        }

        /// Publish a `db.name/referent` claim binding `name`
        /// to `entity`. Used by tests that need a name to resolve
        /// to a specific entity (not a concept the test asserted).
        async fn publish_name(&self, name: &str, entity: Entity) {
            let id_entity: Entity = format!("id:{name}").parse().expect("id:<name> parses");
            self.branch
                .transaction()
                .assert(the!("db.name/referent").of(id_entity).is(entity))
                .commit()
                .perform(&self.operator)
                .await
                .expect("name publication commits");
        }
    }

    /// A flat view of the analysis tree — the document-order
    /// projections the analyzer tests assert against. Built from
    /// the [`Tree<Syntax>`] via its accessor methods; lets the
    /// tests read `statements` / `queries` / `requires` without
    /// each one walking the tree by hand.
    #[derive(Debug)]
    struct Flat {
        declarations: HashMap<String, Entity>,
        variables: HashMap<String, Entity>,
        query: Option<FlatQuery>,
        mutate: FlatMutate,
    }

    #[derive(Debug)]
    struct FlatQuery {
        queries: Vec<Application>,
        synthesized: Vec<Application>,
    }

    impl FlatQuery {
        /// User-named variable slots the user-written queries
        /// bind at evaluation time; auto-generated `__N` names
        /// are excluded.
        fn bindings(&self) -> HashSet<String> {
            let mut out = HashSet::new();
            for application in &self.queries {
                for name in application.bindings() {
                    if !name.starts_with("__") {
                        out.insert(name);
                    }
                }
            }
            out
        }
    }

    #[derive(Debug)]
    struct FlatMutate {
        statements: Vec<Statement>,
        requires: HashSet<String>,
    }

    /// Project the analysis tree into its flat, document-order
    /// view. `requires` is recomputed from the planned
    /// statements exactly as the analyzer enforces it: variable
    /// names a statement reads that are not analysis-time
    /// `variables`.
    fn flat(tree: Tree<Syntax>) -> Flat {
        let document = tree.analysis;
        let statements: Vec<Statement> = document
            .statements()
            .into_iter()
            .map(|p| p.statement)
            .collect();
        let mut requires: HashSet<String> = HashSet::new();
        for statement in &statements {
            let application = statement.application();
            for name in application.bindings() {
                if !document.variables.contains_key(&name) {
                    requires.insert(name);
                }
            }
        }
        let queries: Vec<Application> = document
            .queries()
            .map(|q| q.analysis.application.clone())
            .collect();
        let synthesized: Vec<Application> = document
            .synthesized
            .iter()
            .map(|s| s.application.clone())
            .collect();
        // `query` is present when the document carries any
        // query — user-written or analyzer-synthesized.
        let query = if queries.is_empty() && synthesized.is_empty() {
            None
        } else {
            Some(FlatQuery {
                queries,
                synthesized,
            })
        };
        Flat {
            declarations: document.declarations.clone(),
            variables: document.variables.clone(),
            query,
            mutate: FlatMutate {
                statements,
                requires,
            },
        }
    }

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

    /// Run the analyzer against an empty branch — the new world's
    /// equivalent of a document-only analysis pass.
    async fn analyze_empty(syntax: &Syntax) -> Result<Tree<Syntax>, AnalyzeError> {
        let fixture = new_fixture().await;
        fixture.analyze(syntax).await
    }

    /// An instance head's `&anchor` must be resolvable by a later
    /// field reference in the same document, under both the full
    /// (branch-backed) pipeline and the env-free local path.
    #[dialog_common::test]
    fn it_resolves_an_instance_anchor_referenced_by_a_later_field() {
        let syntax = must_parse(
            "\
concept!: &thing
  description: \"a thing\"
  with:
    source:
      description: \"src\"
      the: x.y/source
      cardinality: one
      as: text

thing!: &my-thing
  source: \"home\"

concept!: &holder
  description: \"holds\"
  with:
    target:
      description: \"e\"
      the: x.y/target
      cardinality: one
      as: entity

holder!: &my-holder
  target: my-thing
",
        );
        let result = super::analyze_local(&syntax);
        assert!(
            result.is_ok(),
            "analyze_local should resolve the `my-thing` anchor referenced \
             by the later `holder!` field: {:?}",
            result.err(),
        );
    }

    /// A pinned instance (`this: <uri>`) referenced by a later field
    /// must resolve to that exact pinned URI — NOT a body-derived
    /// entity. Regression: the anchor pass derived the entity from a
    /// body digest that skips `this:`, so `target: my-thing` resolved
    /// to a random `did:key` instead of the pin.
    #[test]
    fn it_resolves_a_pinned_instance_anchor_to_its_pinned_uri() {
        let syntax = must_parse(
            "\
concept!: &thing
  description: \"a thing\"
  with:
    source:
      description: \"src\"
      the: x.y/source
      cardinality: one
      as: text

thing!: &my-thing
  this: id:my-thing
  source: \"home\"

concept!: &holder
  description: \"holds\"
  with:
    target:
      description: \"e\"
      the: x.y/target
      cardinality: one
      as: entity

holder!: &my-holder
  this: id:my-holder
  target: my-thing
",
        );
        let tree = super::analyze_local(&syntax).expect("analyzes");
        // Find the `holder` assert and read its `target` term — it
        // must be the pinned `id:my-thing`, not a derived entity.
        let target = flat(tree)
            .mutate
            .statements
            .iter()
            .find_map(|s| match s {
                Statement::Assert(Application::Concept { query, .. }) => {
                    query.terms.get("target").and_then(as_constant_entity)
                }
                _ => None,
            })
            .expect("a holder assert with a resolved target entity");
        let expected: Entity = "id:my-thing".parse().expect("valid uri");
        assert_eq!(
            target, expected,
            "target must resolve to the pinned `id:my-thing`, not a derived entity",
        );
    }

    /// A small concept spec the tests pass into [`analyze_with`]:
    /// the published name plus the field set as `(field, the, type)`
    /// triples. Replaces the old `FixedConcept` struct.
    struct ConceptSpec {
        name: &'static str,
        fields: Vec<(&'static str, &'static str, &'static str)>,
    }

    fn fixed_concept(name: &'static str, fields: &[(&'static str, &'static str)]) -> ConceptSpec {
        ConceptSpec {
            name,
            fields: fields.iter().map(|(f, t)| (*f, *t, "Text")).collect(),
        }
    }

    fn fixed_concept_typed(
        name: &'static str,
        fields: &[(&'static str, &'static str, &'static str)],
    ) -> ConceptSpec {
        ConceptSpec {
            name,
            fields: fields.to_vec(),
        }
    }

    /// Open a fresh branch, assert the spec's concept on it with
    /// the right attribute facts + a published name, and run the
    /// analyzer. Replaces the old `let resolver = fixed_concept(...);
    /// analyze_with(&syntax, &resolver)` pattern.
    async fn analyze_with(
        syntax: &Syntax,
        spec: &ConceptSpec,
    ) -> Result<Tree<Syntax>, AnalyzeError> {
        analyze_with_concepts(syntax, std::slice::from_ref(spec)).await
    }

    /// Like [`analyze_with`] but registers a set of concepts —
    /// useful for fixtures whose rule derives one concept from
    /// another. The branch holds every spec before the analyzer
    /// runs.
    async fn analyze_with_concepts(
        syntax: &Syntax,
        specs: &[ConceptSpec],
    ) -> Result<Tree<Syntax>, AnalyzeError> {
        let fixture = new_fixture().await;
        for spec in specs {
            fixture.concept_typed(spec.name, &spec.fields).await;
        }
        fixture.analyze(syntax).await
    }

    #[dialog_common::test]
    async fn it_rejects_empty_document() {
        let syntax = Syntax {
            expressions: Vec::new(),
            range: lsp_types::Range::default(),
        };
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.variables.is_empty());
        assert!(analysis.query.is_none());
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(analysis.mutate.requires.is_empty());
        let Statement::Assert(Application::Concept { .. }) = &analysis.mutate.statements[0] else {
            panic!("expected Assert(Concept)");
        };
    }

    /// A concept whose name contains a `/` (`demo/stuff`) is a
    /// concept head, not a URI head: the `/` is part of the name and
    /// the left side has no dotted domain. The assertion resolves
    /// the concept and lowers like any other.
    #[dialog_common::test]
    async fn it_asserts_a_concept_whose_name_contains_a_slash() {
        let syntax = must_parse(
            r#"
demo/stuff!:
  stuff: "1"
"#,
        );
        let spec = fixed_concept("demo/stuff", &[("stuff", "xyz.tonk.demo/stuff")]);
        let analysis = flat(analyze_with(&syntax, &spec).await.unwrap());
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Assert(Application::Concept { query, .. }) = &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept) for `demo/stuff!:`");
        };
        assert!(
            query.terms.contains("stuff"),
            "the `stuff` field should be carried on the assertion",
        );
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.declarations.contains_key("person-age"));
        assert!(analysis.declarations.contains_key("person"));
        assert!(analysis.query.is_none());
        // 3 statements — 2 attribute + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
    }

    /// A `concept!`'s `with:` field can reference a `/`-namespaced
    /// anchor (`issue/title`) the same way it references a bare one.
    /// The slash-bearing token is a qualified symbol — a name lookup
    /// against the in-doc anchor table — not a string literal, so it
    /// resolves instead of erroring `E_UNSUPPORTED_FIELD_VALUE`.
    #[dialog_common::test]
    async fn it_resolves_namespaced_attribute_referenced_by_a_concept() {
        let syntax = must_parse(
            r#"
attribute!: &issue/title
  the:         io.gozala.issue/title
  as:          Text
  cardinality: one
  description: "Issue title"
concept!: &issue
  description: "Work item"
  with:
    title: issue/title
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.declarations.contains_key("issue/title"));
        assert!(analysis.declarations.contains_key("issue"));
        // 2 statements — 1 attribute + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 2);
    }

    /// `concept!` `with:` accepts inline attribute definitions
    /// alongside bare-symbol references. Each inline definition
    /// becomes its own `Statement::Assert` so the attribute
    /// surfaces in `attribute:` queries; the inline attrs are
    /// anonymous (no `db.meta/name` claim, since the field
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        assert_eq!(name.as_ref().map(AnchorName::as_str), Some("person"));
    }

    /// A `maybe:` block declares optional fields. The descriptor
    /// carries the optional flag on those fields and the required
    /// flag on `with:` fields.
    #[dialog_common::test]
    async fn it_marks_maybe_block_fields_optional() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "A person"
  with:
    name:
      description: "Name"
      the: xyz.tonk.person/name
      as:  Text
  maybe:
    nickname:
      description: "Nickname"
      the: xyz.tonk.person/nickname
      as:  Text
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let person = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = person else {
            panic!("expected concept assertion");
        };
        // The emitted concept-schema records each field's attribute
        // link under `with.<field>`, plus a boolean optional marker
        // `optional.<field>` for `maybe:` fields only.
        let field_names: Vec<&str> = query.predicate.with().iter().map(|(n, _)| n).collect();
        assert!(field_names.contains(&"with.name"));
        assert!(field_names.contains(&"with.nickname"));
        assert!(
            field_names.contains(&"optional.nickname"),
            "optional `maybe:` field must emit an optional marker"
        );
        assert!(
            !field_names.contains(&"optional.name"),
            "required `with:` field must not emit an optional marker"
        );
        // And the marker term is actually populated on the assertion.
        assert!(
            query.terms.get("optional.nickname").is_some(),
            "expected optional.nickname term on the concept assertion"
        );
        assert!(query.terms.get("optional.name").is_none());
    }

    /// A concept with only `maybe:` fields (no required field) is
    /// rejected — it would constrain nothing and match every entity.
    #[dialog_common::test]
    async fn it_rejects_concept_with_only_optional_fields() {
        let syntax = must_parse(
            r#"
concept!: &person
  maybe:
    nickname:
      description: "Nickname"
      the: xyz.tonk.person/nickname
      as:  Text
"#,
        );
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidConceptBody { .. }),
            "expected InvalidConceptBody for all-optional concept, got {err:?}"
        );
    }

    /// A field declared in both `with:` and `maybe:` is a hard
    /// error — a field is required or optional, never both.
    #[dialog_common::test]
    async fn it_rejects_field_in_both_with_and_maybe() {
        let syntax = must_parse(
            r#"
concept!: &person
  with:
    name:
      description: "Name"
      the: xyz.tonk.person/name
      as:  Text
  maybe:
    name:
      description: "Name"
      the: xyz.tonk.person/name
      as:  Text
"#,
        );
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::DuplicateConceptField { field, .. } if field == "name"
            ),
            "expected DuplicateConceptField for `name`, got {err:?}"
        );
        assert_eq!(err.kind.code(), "E_DUPLICATE_CONCEPT_FIELD");
    }

    /// A `maybe:` field can be a bare reference to an attribute
    /// declared earlier in the document (not just an inline
    /// definition). The referenced field is still marked optional.
    #[dialog_common::test]
    async fn it_marks_maybe_bare_reference_optional() {
        let syntax = must_parse(
            r#"
attribute!: &person-nick
  the:         xyz.tonk.person/nickname
  as:          Text
  cardinality: one
  description: "Nickname"
concept!: &person
  with:
    name:
      description: "Name"
      the: xyz.tonk.person/name
      as:  Text
  maybe:
    nickname: person-nick
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let person = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = person else {
            panic!("expected concept assertion");
        };
        // The `maybe:` bare reference emits an optional marker.
        let field_names: Vec<&str> = query.predicate.with().iter().map(|(n, _)| n).collect();
        assert!(
            field_names.contains(&"optional.nickname"),
            "bare-reference `maybe:` field must emit an optional marker; saw {field_names:?}"
        );
        assert!(query.terms.get("optional.nickname").is_some());
    }

    /// A `maybe:` field referencing an attribute that lives on the
    /// branch (not declared in the document) must resolve, exactly
    /// like a `with:` field does. The prefetch phase has to gather
    /// references from both blocks — gathering only `with:` left
    /// branch attributes referenced solely from `maybe:` unresolved,
    /// surfacing as a spurious "unknown name" error.
    #[dialog_common::test]
    async fn it_resolves_branch_attribute_referenced_from_maybe() {
        // The branch holds two attributes published as the bare
        // (dot-free) names `dir/title` and `dir/icon` so a reference
        // parses as a name lookup, not a dotted-domain URI.
        let fixture = new_fixture().await;
        fixture
            .attributes(&[
                ("dir/title", "title", "xyz.tonk.dir/title", "Text"),
                ("dir/icon", "icon", "xyz.tonk.dir/icon", "Text"),
            ])
            .await;

        // A fresh concept references `title` from `with:` (control)
        // and `icon` from `maybe:` (the path the bug broke) — both
        // by published branch name, neither declared in this doc.
        let syntax = must_parse(
            r#"
concept!: &card
  description: "A card"
  with:
    title: dir/title
  maybe:
    icon: dir/icon
"#,
        );
        let analysis = flat(fixture.analyze(&syntax).await.unwrap());
        assert!(analysis.declarations.contains_key("card"));
        let card = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = card else {
            panic!("expected concept assertion");
        };
        let field_names: Vec<&str> = query.predicate.with().iter().map(|(n, _)| n).collect();
        assert!(
            field_names.contains(&"with.title"),
            "required `with:` reference must resolve; saw {field_names:?}"
        );
        assert!(
            field_names.contains(&"with.icon"),
            "optional `maybe:` reference to a branch attribute must resolve; saw {field_names:?}"
        );
        assert!(
            field_names.contains(&"optional.icon"),
            "the `maybe:` field must still be marked optional; saw {field_names:?}"
        );
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
      the:         db.view/source
      as:          Entity
      cardinality: one
view!: &title
  source: person
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::InvalidAttributeBody { reason } if reason.contains("description")),
            "expected InvalidAttributeBody about description, got {err:?}"
        );
    }

    /// Variable-form `attribute!:` with `this: ?foo` (no anchor)
    /// lands in `variables`, not `declarations`, and does NOT
    /// emit a `db.meta/name` claim (the name is doc-scoped
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
    /// `db.meta/name` claim on `id:<name>`, not as a
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let Statement::Assert(Application::Concept { query, name, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(query.terms.get("name").is_none());
        assert_eq!(name.as_ref().map(AnchorName::as_str), Some("person-name"));
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
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::DuplicateName { .. }),
            "expected DuplicateName, got {err:?}"
        );
    }

    /// A concept declared under a built-in's name → `ReservedName`.
    ///
    /// Premise heads resolve formulas, constraints and resolvers
    /// before concepts, so such a concept could never be referenced:
    /// every mention would silently mean the built-in. Reporting it
    /// at the declaration is the only place the user can still act.
    #[dialog_common::test]
    async fn it_rejects_a_concept_named_after_a_builtin() {
        for reserved in ["tree/node", "math/sum"] {
            let syntax = must_parse(&format!(
                r#"
concept!: &{reserved}
  with:
    tag:
      the: x.y/tag
      as: Text
      cardinality: one
      description: "T"
"#
            ));
            let err = match analyze_empty(&syntax).await {
                Err(err) => err,
                Ok(_) => panic!("`{reserved}` must be rejected as a concept name"),
            };
            assert!(
                matches!(err.kind, AnalyzeErrorKind::ReservedName { .. }),
                "expected ReservedName for {reserved:?}, got {err:?}"
            );
        }
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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

    /// `concept!: { this: <pinned>, with: { f: _ } }` retracts the
    /// named field from a stored concept: a `Statement::Retract`
    /// carrying the `with.f` term bound to the field's stored
    /// attribute entity, and nothing else (no marker, no other
    /// field).
    #[dialog_common::test]
    async fn it_retracts_a_named_field_from_a_concept() {
        let fixture = new_fixture().await;
        let entity: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv"
            .parse()
            .expect("valid entity");
        fixture
            .assert_concept_at(
                &entity,
                &[
                    ("name", "io.gozala.person/name"),
                    ("age", "io.gozala.person/age"),
                ],
            )
            .await;
        let syntax = must_parse(
            r#"
concept!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  with:
    age: _
"#,
        );
        let analysis = flat(fixture.analyze(&syntax).await.unwrap());
        // A pure retraction emits exactly one statement: the retract.
        let retracts: Vec<_> = analysis
            .mutate
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::Retract(Application::Concept { query, .. }) => Some(query),
                _ => None,
            })
            .collect();
        assert_eq!(retracts.len(), 1, "exactly one concept retraction");
        let q = retracts[0];
        // `with.age` is blank: the evaluator dissociates whatever the
        // branch holds for age (the instance-retraction model).
        assert!(
            matches!(
                q.terms.get("with.age"),
                Some(Term::Variable { name: None, .. })
            ),
            "with.age is a blank retraction directive",
        );
        // `name` is untouched — no term for it, and no marker term.
        assert!(
            q.terms.get("with.name").is_none(),
            "the kept field is not retracted",
        );
        assert!(
            q.terms.get("concept").is_none(),
            "the concept marker is not retracted",
        );
        // No assert side for a retraction-only body.
        assert!(
            !analysis
                .mutate
                .statements
                .iter()
                .any(|s| matches!(s, Statement::Assert(Application::Concept { .. }))),
            "retraction-only body emits no concept assertion",
        );
    }

    /// `..: _` on a pinned concept retracts every stored field.
    #[dialog_common::test]
    async fn it_retracts_every_field_from_a_concept_via_rest() {
        let fixture = new_fixture().await;
        let entity: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv"
            .parse()
            .expect("valid entity");
        fixture
            .assert_concept_at(
                &entity,
                &[
                    ("name", "io.gozala.person/name"),
                    ("age", "io.gozala.person/age"),
                ],
            )
            .await;
        let syntax = must_parse(
            r#"
concept!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  ..: _
"#,
        );
        let analysis = flat(fixture.analyze(&syntax).await.unwrap());
        let q = analysis
            .mutate
            .statements
            .iter()
            .find_map(|s| match s {
                Statement::Retract(Application::Concept { query, .. }) => Some(query),
                _ => None,
            })
            .expect("a concept retraction");
        for field in ["name", "age"] {
            assert!(
                q.terms.get(&format!("with.{field}")).is_some(),
                "`..: _` retracts every stored field, including {field}",
            );
        }
    }

    /// Retracting a field the concept doesn't carry is a hard error
    /// (strict policy): there's no stored triple to dissociate.
    #[dialog_common::test]
    async fn it_rejects_retracting_an_absent_field() {
        let fixture = new_fixture().await;
        let entity: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv"
            .parse()
            .expect("valid entity");
        fixture
            .assert_concept_at(&entity, &[("name", "io.gozala.person/name")])
            .await;
        let syntax = must_parse(
            r#"
concept!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  with:
    nonexistent: _
"#,
        );
        let err = fixture.analyze(&syntax).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidConceptBody { .. }),
            "expected InvalidConceptBody for absent-field retraction, got {err:?}",
        );
    }

    /// Retracting against a concept that doesn't exist on the branch
    /// is a hard error — the attribute value to dissociate is unknown.
    #[dialog_common::test]
    async fn it_rejects_retraction_against_unknown_concept() {
        let syntax = must_parse(
            r#"
concept!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  with:
    age: _
"#,
        );
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidConceptBody { .. }),
            "expected InvalidConceptBody for unknown-concept retraction, got {err:?}",
        );
    }

    /// `..` is the top-level rest-retraction marker, not a field
    /// name. Nested inside a `with:`/`maybe:` block it must be a
    /// parse error, not a silent "retract a field named `..`".
    #[dialog_common::test]
    async fn it_rejects_rest_marker_nested_in_a_block() {
        for block in ["with", "maybe"] {
            let syntax = must_parse(&format!(
                "concept!:\n  this: id:thing\n  {block}:\n    ..: _\n"
            ));
            let err = analyze_empty(&syntax).await.unwrap_err();
            assert!(
                matches!(err.kind, AnalyzeErrorKind::InvalidConceptBody { .. }),
                "expected InvalidConceptBody for `..` nested in `{block}:`, got {err:?}",
            );
        }
    }

    /// A partial assertion to a concrete-URI entity sets only the
    /// fields it names; unmentioned fields are omitted from the
    /// assert (not carried as blanks), so the claim lowers cleanly
    /// to the wire — a blank would be rejected by wire lowering.
    /// This is what lets one entity accumulate a cardinality-many
    /// field across several assertions without repeating its other
    /// fields each time.
    #[dialog_common::test]
    async fn it_omits_unmentioned_fields_on_a_partial_assert() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  name: "Alice"
"#,
        );
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let tree = analyze_with(&syntax, &resolver).await.unwrap();
        let analysis = flat(tree.clone());
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept) for the partial assertion");
        };
        // `this` and the named `name` are present; the unmentioned
        // `age` is omitted entirely.
        assert!(matches!(
            q.terms.get("this"),
            Some(Term::Constant(Value::Entity(_)))
        ));
        assert!(matches!(
            q.terms.get("name"),
            Some(Term::Constant(Value::String(_)))
        ));
        assert!(
            q.terms.get("age").is_none(),
            "unmentioned `age` should be omitted from the assert, got {:?}",
            q.terms.get("age"),
        );
        // And it lowers to a wire claim without a NonConstantTerm error.
        tree.analysis
            .lower_to_claims()
            .expect("partial assertion lowers to a wire claim");
    }

    /// A bare integer literal written into an `unsigned-integer`
    /// field is schema-coerced to `Value::UnsignedInt`. The
    /// notation parser always parses `0` as a signed
    /// `Scalar::Integer`; the analyzer knows the field's declared
    /// type and must produce an unsigned term so induction over
    /// `math/sum` (unsigned-only) doesn't fail with `TypeMismatch`.
    #[dialog_common::test]
    async fn it_coerces_integer_literal_to_unsigned_for_unsigned_field() {
        let syntax = must_parse(
            r#"
counter!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  count: 0
"#,
        );
        let resolver = fixed_concept_typed(
            "counter",
            &[("count", "xyz.tonk.counter/count", "UnsignedInteger")],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(
            matches!(
                q.terms.get("count"),
                Some(Term::Constant(Value::UnsignedInt(0)))
            ),
            "count literal should coerce to UnsignedInt, got {:?}",
            q.terms.get("count")
        );
    }

    /// A bare integer literal in a `signed-integer` field stays
    /// signed — the coercion is type-directed, not blanket.
    #[dialog_common::test]
    async fn it_keeps_integer_literal_signed_for_signed_field() {
        let syntax = must_parse(
            r#"
reading!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  value: 7
"#,
        );
        let resolver = fixed_concept_typed(
            "reading",
            &[("value", "xyz.tonk.reading/value", "SignedInteger")],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(
            matches!(
                q.terms.get("value"),
                Some(Term::Constant(Value::SignedInt(7)))
            ),
            "value literal should stay SignedInt, got {:?}",
            q.terms.get("value")
        );
    }

    /// A bare integer literal written into a `text` field is a
    /// type mismatch, not a silent miscast. Storing `3` as a
    /// `SignedInt` under a Text-typed attribute makes the entity
    /// invisible to its own (strictly-typed) concept query, so the
    /// analyzer rejects it up front and points at the field.
    /// User-reported: `age: 3` into an `as: text` field.
    #[dialog_common::test]
    async fn it_rejects_integer_literal_for_text_field() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: 3
"#,
        );
        let resolver = fixed_concept_typed("person", &[("age", "xyz.tonk.person/age", "Text")]);
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
        let AnalyzeErrorKind::TypeMismatch { field, .. } = &err.kind else {
            panic!("expected TypeMismatch, got {:?}", err.kind);
        };
        assert_eq!(field, "age", "the diagnostic must name the offending field");
        assert!(
            err.range.is_some(),
            "the diagnostic must carry a range so the editor can highlight the value"
        );
    }

    /// A quoted string written into a `text` field is fine — the
    /// type matches, so no diagnostic. Guards against the rejection
    /// above over-firing on valid input.
    #[dialog_common::test]
    async fn it_accepts_string_literal_for_text_field() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: "3"
"#,
        );
        let resolver = fixed_concept_typed("person", &[("age", "xyz.tonk.person/age", "Text")]);
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(
            matches!(q.terms.get("age"), Some(Term::Constant(Value::String(s))) if s == "3"),
            "age should stay a Text value, got {:?}",
            q.terms.get("age")
        );
    }

    /// A claim head (`squash.bug:`) has no schema, so its fields
    /// carry no declared type. With `expected = None`, any literal
    /// is accepted, typed by its spelling — bare `3` is unsigned —
    /// and never raises `TypeMismatch`. Guards the "no type
    /// specified accepts any type" rule.
    #[dialog_common::test]
    async fn it_accepts_any_literal_for_untyped_claim_field() {
        let syntax = must_parse(
            r#"
xyz.tonk.person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  age: 3
"#,
        );
        // No concept registered — `xyz.tonk.person` is a domain
        // (claim) head, so the `age` slot has no declared type.
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let Statement::Assert(application) = &analysis.mutate.statements[0] else {
            panic!("expected an Assert statement");
        };
        let term = application.parameters().get("age").cloned();
        assert!(
            matches!(term, Some(Term::Constant(Value::UnsignedInt(3)))),
            "a bare integer on an untyped claim field is unsigned by spelling, got {term:?}"
        );
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
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(matches!(err.kind, AnalyzeErrorKind::UnknownConcept { .. }));
    }

    /// Built-in `attribute:` resolves without a branch:
    /// the registry is consulted before the source-backed
    /// resolution chain, so an empty branch still gets the
    /// built-ins.
    #[dialog_common::test]
    async fn it_resolves_builtin_attribute_under_empty_branch() {
        let syntax = must_parse(
            r#"
attribute:
  this: ?a
  description: ?d
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        // The four anonymous-attribute fields — id/type/
        // cardinality/description — must all be present in the
        // unified term map. `name` is intentionally not in the
        // built-in `attribute:` view (only anchor-form attrs
        // carry a `db.meta/name` claim).
        for field in ["id", "type", "cardinality", "description"] {
            assert!(query.terms.contains(field), "missing {field}");
        }
    }

    /// Built-in `branch:` (and other Rust-defined repository
    /// concepts) resolve through the registry, not the resolver,
    /// even though they have no branch-side `concept!`
    /// definition.
    #[dialog_common::test]
    async fn it_resolves_builtin_branch_on_empty_branch() {
        let syntax = must_parse(
            r#"
branch:
  this: ?b
  name: ?name
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let tonk_schema::transact::Application::Domain { application: d, .. } = &q.queries[0]
        else {
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
    /// `db.meta/name` attribute (cardinality one).
    #[dialog_common::test]
    async fn it_resolves_builtin_name_concept() {
        let syntax = must_parse(
            r#"
name:
  this: ?n
  entity: ?e
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        // `(this, db.meta/concept, db:concept)`. We can read
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Assert(Application::Concept { name, this, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_ref().map(AnchorName::as_str), Some("alice"));
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        let Statement::Assert(Application::Concept { name, this, query }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_ref().map(AnchorName::as_str), Some("alice"));
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
        // The mutation expression is the second statement.
        let Statement::Assert(Application::Concept { name, this, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_ref().map(AnchorName::as_str), Some("latest-alice"));
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let fixture = new_fixture().await;
        fixture.concept("person", &[("name", "x.y/name")]).await;
        // Publish `evil` as a name bound to the reserved `db:concept`
        // entity. The analyzer's bare-symbol resolution flows through
        // `lookup_named_entity` and lands on the protected URI.
        let target: Entity = "db:concept".parse().expect("db:concept parses");
        fixture.publish_name("evil", target).await;
        let syntax = must_parse(
            r#"
person!:
  this: evil
  name: "x"
"#,
        );
        let err = fixture.analyze(&syntax).await.unwrap_err();
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.query.is_some());
    }

    /// `this: alice` (bare symbol) resolves through the name
    /// table — Stage 2.5. A bare symbol that doesn't match any
    /// in-doc declaration or branch name surfaces
    /// `UnknownNameReference` with `field: "this"`.
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::UnknownNameReference { field, name } if field == "this" && name == "ghost"),
            "expected UnknownNameReference on `this` with name=\"ghost\", got {err:?}"
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
    /// when no in-doc declaration matches. Publishes an
    /// `id:alice` referent to a fixed entity, then exercises the
    /// analyzer's bare-symbol → name lookup path.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_in_this_via_branch_name_index() {
        let fixture = new_fixture().await;
        fixture
            .concept("person", &[("name", "io.gozala.person/name")])
            .await;
        let alice_entity: Entity = "did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv"
            .parse()
            .unwrap();
        fixture.publish_name("alice", alice_entity.clone()).await;
        let syntax = must_parse(
            r#"
person!:
  this: alice
  name: "Renamed"
"#,
        );
        let analysis = flat(fixture.analyze(&syntax).await.unwrap());
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed("thing", &[("active", "x.y/active", "Boolean")]);
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed("thing", &[("weight", "x.y/weight", "Float")]);
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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

    /// `this: <uri>` on a `concept!` pins the concept entity to that
    /// URI instead of the content-derived hash, and the concept's
    /// instances derive from the pinned entity — so a pinned concept
    /// stays referenceable by a stable URI.
    #[dialog_common::test]
    async fn it_pins_concept_entity_via_this_uri() {
        let syntax = must_parse(
            r#"
concept!: &view
  this: tonk:view
  description: "A view"
  with:
    model:
      description: "Concept this view renders"
      the:         xyz.tonk.view/model
      as:          Entity
      cardinality: one
    display:
      description: "HTML template"
      the:         xyz.tonk.view/display
      as:          Text
      cardinality: one
view!:
  model: tonk:view
  display: "x"
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());

        // The declared concept entity is the pinned URI — the
        // `&view` anchor publishes `view` -> `tonk:view`, not the
        // descriptor hash.
        let pinned: Entity = "tonk:view".parse().unwrap();
        assert_eq!(
            analysis.declarations.get("view"),
            Some(&pinned),
            "concept `view` should be pinned to `tonk:view`",
        );

        // The view instance derives its subject from the pinned
        // predicate entity, so its `this` is `derive_this(tonk:view,
        // {model, display})` — NOT derived from the descriptor hash.
        let instance = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = instance else {
            panic!("expected view instance assertion");
        };
        let derived_with_pin = {
            use tonk_core::claim::ValueMap;
            use tonk_schema::transact::derive_this;
            let mut body = ValueMap::new();
            body.insert("model".into(), Value::Entity(pinned.clone()));
            body.insert("display".into(), Value::String("x".into()));
            derive_this(&pinned, &body)
        };
        match query.terms.get("this") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert_eq!(
                    e, &derived_with_pin,
                    "instance entity must derive from the pinned predicate entity"
                );
            }
            other => panic!("instance `this` should be a derived entity, got {other:?}"),
        }
    }

    /// Wire convergence for a PINNED concept. The notation path
    /// derives a pinned concept's instance from the pinned entity.
    /// The wire path (`application_plan_from_predicate`) carries no
    /// pin — its `ConceptDescriptor` only knows `descriptor.this()`
    /// (the content hash). The two still converge in practice
    /// because every real wire producer (`lower_statement`) carries
    /// `this` explicitly in the payload, which makes the wire path
    /// SKIP derivation. This test pins that contract: with `this`
    /// carried, wire == notation; with `this` omitted, the wire path
    /// derives a DIFFERENT entity (the documented edge — a hand-
    /// rolled caller that omits `this` for a pinned concept).
    #[dialog_common::test]
    async fn it_converges_wire_path_for_pinned_concept_when_this_is_carried() {
        use dialog_query::AttributeDescriptor;
        use dialog_query::artifact::Type;
        use dialog_query::attribute::Cardinality as DialogCardinality;
        use dialog_query::concept::descriptor::ConceptDescriptor as DialogConceptDescriptor;
        use tonk_core::claim::{ConceptDescriptor as WireDescriptor, SourceApplication, ValueMap};
        use tonk_schema::transact::{
            ApplicationPlan, application_plan_from_predicate, derive_this,
        };

        let pinned: Entity = "tonk:view".parse().unwrap();
        let mut body = ValueMap::new();
        body.insert("display".into(), Value::String("x".into()));
        let notation_this = derive_this(&pinned, &body);

        // The wire descriptor for the `view` concept — `{display}`.
        let descriptor = DialogConceptDescriptor::try_from(vec![(
            "display",
            AttributeDescriptor::new(
                "xyz.tonk.view/display".parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        )])
        .unwrap();

        // Wire path WITH `this` carried (the real bootstrap shape):
        // derivation is skipped, the carried entity is used verbatim.
        let mut params_with_this = ValueMap::new();
        params_with_this.insert("display".into(), Value::String("x".into()));
        params_with_this.insert("this".into(), Value::Entity(notation_this.clone()));
        let plan = application_plan_from_predicate(
            SourceApplication {
                predicate: WireDescriptor::Durable(descriptor.clone()),
                parameters: params_with_this,
                name: None,
            }
            .try_into()
            .expect("wire application validates"),
        );
        let ApplicationPlan::Concept(plan) = &plan else {
            panic!("expected concept plan");
        };
        let wire_this = match plan.statement.terms.get("this").expect("this present") {
            dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
            other => panic!("expected entity constant, got {other:?}"),
        };
        assert_eq!(
            wire_this, notation_this,
            "with `this` carried, the wire path must match the pinned notation entity",
        );

        // Wire path WITHOUT `this`: derives from the descriptor hash,
        // which is NOT the pinned entity. This documents the edge.
        let mut params_no_this = ValueMap::new();
        params_no_this.insert("display".into(), Value::String("x".into()));
        let plan = application_plan_from_predicate(
            SourceApplication {
                predicate: WireDescriptor::Durable(descriptor),
                parameters: params_no_this,
                name: None,
            }
            .try_into()
            .expect("wire application validates"),
        );
        let ApplicationPlan::Concept(plan) = &plan else {
            panic!("expected concept plan");
        };
        let wire_this_no_pin = match plan.statement.terms.get("this").expect("this present") {
            dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
            other => panic!("expected entity constant, got {other:?}"),
        };
        assert_ne!(
            wire_this_no_pin, notation_this,
            "without a carried `this`, the wire path cannot reproduce the pin \
             (documents the hand-rolled-caller edge)",
        );
    }

    /// A bare symbol that doesn't resolve anywhere surfaces
    /// `UnknownNameReference` (with the symbol's name in `name`).
    #[dialog_common::test]
    async fn it_rejects_unresolvable_bare_symbol() {
        let syntax = must_parse(
            r#"
person!:
  name: ghost
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
        assert!(
            matches!(&err.kind, AnalyzeErrorKind::UnknownNameReference { name, .. } if name == "ghost"),
            "expected UnknownNameReference{{name:\"ghost\"}}, got {err:?}"
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
        analyze_with(&syntax, &resolver).await.unwrap();
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
        assert!(
            matches!(err.kind, AnalyzeErrorKind::InvalidClaimAttribute { .. }),
            "expected InvalidClaimAttribute, got {err:?}"
        );
    }

    /// A failing resolver propagates as `ResolverFailed`. The
    /// old `Resolver`-trait world made simulating an I/O failure
    /// trivial; under the source-and-env world the failure has
    /// to come from a misbehaving `QueryEnv`, which the existing
    /// helpers don't model. Re-enabling this test waits on a
    /// failure-injecting env helper.
    #[dialog_common::test]
    #[ignore = "scope-env refactor: needs a failure-injecting QueryEnv helper"]
    async fn it_surfaces_resolver_failures() {
        let syntax = must_parse(
            r#"
person:
  name: "Alice"
"#,
        );
        let err = analyze_empty(&syntax).await.unwrap_err();
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
    async fn it_resolves_builtin_concept_on_empty_branch() {
        let syntax = must_parse(
            r#"
concept:
  this: ?c
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.query.is_some());
    }

    /// Built-in `rule:` resolves through the registry without a
    /// branch. Returns the rule-of-rule descriptor whose entity
    /// is `db:rule`; the query side is populated.
    #[dialog_common::test]
    async fn it_resolves_builtin_rule_on_empty_branch() {
        let syntax = must_parse(
            r#"
rule:
  this: ?r
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_replica_on_empty_branch() {
        let syntax = must_parse(
            r#"
replica:
  this: ?r
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_remote_on_empty_branch() {
        let syntax = must_parse(
            r#"
remote:
  this: ?r
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
        assert!(analysis.query.is_some());
    }

    #[dialog_common::test]
    async fn it_resolves_builtin_tracking_branch_on_empty_branch() {
        let syntax = must_parse(
            r#"
tracking-branch:
  this: ?t
"#,
        );
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
                assert_eq!(
                    name.as_ref().map(AnchorName::as_str),
                    Some("person"),
                    "concept itself anchored"
                );
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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
            analyze_empty(&syntax)
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
        analyze_empty(&syntax).await.unwrap();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        analyze_empty(&syntax).await.unwrap();
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
        analyze_empty(&syntax).await.unwrap();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        analyze_empty(&syntax).await.unwrap();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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

    /// Omitting an *optional* field on a fresh entity is fine —
    /// `IncompleteAssertion` only counts required fields. Here the
    /// body sets the required `name` but omits the optional
    /// `nickname`; the assertion must succeed.
    #[dialog_common::test]
    async fn it_allows_assertion_omitting_optional_field() {
        let syntax = must_parse(
            r#"
person!:
  name: "Alice"
"#,
        );
        let fixture = new_fixture().await;
        fixture
            .concept_typed_optional(
                "person",
                &[
                    ("name", "io.gozala.person/name", "Text", false),
                    ("nickname", "io.gozala.person/nickname", "Text", true),
                ],
            )
            .await;
        let analysis = fixture.analyze(&syntax).await;
        assert!(
            analysis.is_ok(),
            "omitting an optional field must not raise IncompleteAssertion, got {:?}",
            analysis.err()
        );
    }

    /// The complement: with an optional field present in the
    /// schema, omitting the *required* field on a fresh entity
    /// still raises `IncompleteAssertion`, and the optional field
    /// is not listed as missing.
    #[dialog_common::test]
    async fn it_still_requires_required_field_when_optional_present() {
        let syntax = must_parse(
            r#"
person!:
  nickname: "Al"
"#,
        );
        let fixture = new_fixture().await;
        fixture
            .concept_typed_optional(
                "person",
                &[
                    ("name", "io.gozala.person/name", "Text", false),
                    ("nickname", "io.gozala.person/nickname", "Text", true),
                ],
            )
            .await;
        let err = fixture.analyze(&syntax).await.unwrap_err();
        assert!(
            matches!(
                &err.kind,
                AnalyzeErrorKind::IncompleteAssertion { missing, .. }
                    if missing == &vec!["name".to_string()]
            ),
            "expected IncompleteAssertion listing only `name`, got {err:?}"
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        analyze_with(&syntax, &resolver).await.unwrap();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        analyze_with(&syntax, &resolver).await.unwrap();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        analyze_with(&syntax, &resolver).await.unwrap();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        analyze_with(&syntax, &resolver).await.unwrap();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let err = analyze_empty(&syntax).await.unwrap_err();
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
        let err = analyze_with(&syntax, &resolver).await.unwrap_err();
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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
        let resolver = fixed_concept_typed(
            "person",
            &[
                ("name", "io.gozala.person/name", "Text"),
                ("age", "io.gozala.person/age", "SignedInteger"),
            ],
        );
        let analysis = flat(analyze_with(&syntax, &resolver).await.unwrap());
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

    /// The analyzer builds a genuine `Analysis<Syntax>` tree:
    /// one analyzed node per top-level expression, in document
    /// order, each carrying its variant-specific analysis. The
    /// document here mixes a `concept!` declaration, an
    /// assertion, a query, and a `rule!:`.
    #[dialog_common::test]
    async fn it_builds_a_populated_analysis_tree() {
        use crate::analysis::{ExpressionAnalysis, Predicate};

        let syntax = must_parse(
            r#"
concept!: &thing
  description: "Thing concept"
  with:
    label:
      description: "Label of the thing"
      the:         xyz.tonk.thing/label
      as:          Text
      cardinality: one
person!:
  this: did:key:zPersonAlice
  name: "Alice"
person:
  this: ?p
  name: ?n
rule!:
  assert!: greeting
  when:
    - assert: person
      where: { this: ?this, name: ?greeting }
"#,
        );
        let tree = analyze_with_concepts(
            &syntax,
            &[
                fixed_concept("person", &[("name", "io.gozala.person/name")]),
                fixed_concept("greeting", &[("greeting", "io.gozala.greeting/greeting")]),
            ],
        )
        .await
        .unwrap();

        // The tree mirrors the document — one node per expression.
        assert_eq!(
            tree.analysis.expressions.len(),
            4,
            "tree must hold one analyzed node per top-level expression"
        );
        assert!(
            tree.analysis.declarations.contains_key("thing"),
            "document-level declarations are threaded onto the tree"
        );

        // Node 0 — `concept!` declaration assertion.
        match &tree.analysis.expressions[0].analysis {
            ExpressionAnalysis::Assertion(node) => {
                assert!(
                    node.analysis.declaration,
                    "concept! head is flagged as a declaration"
                );
                assert!(
                    !node.analysis.claims.is_empty(),
                    "the concept! declaration produced lowered statements"
                );
            }
            other => panic!("expected an assertion node, got {other:?}"),
        }

        // Node 1 — `person!` assertion: a concept predicate, a
        // URI `this`, and at least one lowered claim.
        match &tree.analysis.expressions[1].analysis {
            ExpressionAnalysis::Assertion(node) => {
                assert!(
                    matches!(node.analysis.predicate, Predicate::Concept(_)),
                    "person! resolved to a concept predicate"
                );
                assert!(
                    !node.analysis.claims.is_empty(),
                    "person! produced lowered claims"
                );
            }
            other => panic!("expected an assertion node, got {other:?}"),
        }

        // Node 2 — `person:` query carries its built application.
        match &tree.analysis.expressions[2].analysis {
            ExpressionAnalysis::Query(node) => {
                assert_eq!(node.analysis.label, "person");
            }
            other => panic!("expected a query node, got {other:?}"),
        }

        // Node 3 — `rule!:` is a Claim whose analysis carries the
        // lifted effect on `AssertionAnalysis::effect`.
        let ExpressionAnalysis::Assertion(node) = &tree.analysis.expressions[3].analysis else {
            panic!("the rule!: expression should be a Claim assertion");
        };
        assert!(
            node.analysis.effect.is_some(),
            "the rule!: claim should carry the lifted Effect"
        );
    }

    /// An assertion against a transient concept carries the
    /// `Transient` variant on `AssertionAnalysis::predicate`;
    /// one against a durable concept carries `Durable`. The
    /// analyzer recovers the tag from the head concept's
    /// `dialog.concept/transient` marker at tree-build time.
    #[dialog_common::test]
    async fn it_tags_the_assertion_predicate_with_its_durability() {
        use crate::analysis::{ExpressionAnalysis, Predicate};
        use tonk_core::claim::ConceptDescriptor;

        let syntax = must_parse(
            r#"
concept!: &ping
  transient:
  with:
    tag:
      description: "Tag"
      the:         io.gozala.ping/tag
      as:          Text
      cardinality: one
concept!: &pong
  with:
    tag:
      description: "Tag"
      the:         io.gozala.pong/tag
      as:          Text
      cardinality: one
ping!:
  this: did:key:zPingSubject
  tag:  "hi"
pong!:
  this: did:key:zPongSubject
  tag:  "bye"
"#,
        );
        let tree = analyze_empty(&syntax).await.unwrap();

        // Node 2 — `ping!` instance: transient concept predicate.
        match &tree.analysis.expressions[2].analysis {
            ExpressionAnalysis::Assertion(node) => assert!(
                matches!(
                    node.analysis.predicate,
                    Predicate::Concept(ConceptDescriptor::Transient(_))
                ),
                "ping! resolved to a transient concept; predicate must carry Transient, \
                 got {:?}",
                node.analysis.predicate
            ),
            other => panic!("expected an assertion node, got {other:?}"),
        }

        // Node 3 — `pong!` instance: durable concept predicate.
        match &tree.analysis.expressions[3].analysis {
            ExpressionAnalysis::Assertion(node) => assert!(
                matches!(
                    node.analysis.predicate,
                    Predicate::Concept(ConceptDescriptor::Durable(_))
                ),
                "pong! resolved to a durable concept; predicate must carry Durable, \
                 got {:?}",
                node.analysis.predicate
            ),
            other => panic!("expected an assertion node, got {other:?}"),
        }
    }

    /// `DocumentAnalysis::statements()` projects the tree into
    /// apply order: every assertion's lowered claims in document
    /// order, then a trailing `InstallEffect` per `rule!:`. A
    /// rule declared *before* an assertion in source still sorts
    /// after it — the evaluator depends on this ordering.
    #[dialog_common::test]
    async fn it_orders_statements_assertions_before_rule_effects() {
        use tonk_schema::transact::Statement;

        let syntax = must_parse(
            r#"
rule!:
  assert!: greeting
  when:
    - assert: person
      where: { this: ?this, name: ?greeting }
person!:
  this: did:key:zStatementOrder
  name: "Alice"
"#,
        );
        let tree = analyze_with_concepts(
            &syntax,
            &[
                fixed_concept("person", &[("name", "io.gozala.person/name")]),
                fixed_concept("greeting", &[("greeting", "io.gozala.greeting/greeting")]),
            ],
        )
        .await
        .unwrap();
        let statements = tree.analysis.statements();

        // The rule is the first expression in source, but its
        // Rule install must sort last.
        assert!(
            !statements.is_empty(),
            "the document produced planned statements"
        );
        let last = statements.last().expect("at least one statement");
        assert!(
            matches!(last.statement, Statement::Assert(Application::Rule { .. })),
            "the rule!: install sorts after every assertion statement, got {:?}",
            last.statement
        );
        assert!(
            statements[..statements.len() - 1]
                .iter()
                .all(|s| !matches!(s.statement, Statement::Assert(Application::Rule { .. }))),
            "only the trailing statement is a rule install"
        );
    }

    /// A field a rule premise omits — or writes as a blank (`_`) —
    /// lifts to a true `Term::blank()` wildcard, not a named
    /// anonymous variable. This matters for `unless:` premises:
    /// negations never bind variables, so the planner treats a named
    /// variable in a negation as a required-but-unbound binding and
    /// rejects the rule with `RequiredBindings`. A blank is skipped
    /// as a wildcard, so the rule compiles.
    ///
    /// Regression: a `unless: tonk/binder { this: ?this }` premise
    /// against a `tonk/binder` concept with an extra `active` field
    /// (left unmentioned) failed to compile because the omitted
    /// `active` became `__N`.
    #[dialog_common::test]
    async fn it_compiles_a_negation_premise_omitting_a_concept_field() {
        let specs = [
            fixed_concept_typed(
                "replica",
                &[("subject", "dialog.replica/subject", "Entity")],
            ),
            fixed_concept_typed("binder", &[("active", "xyz.tonk.binder/active", "Entity")]),
        ];

        // The head `binder`'s `this` is bound by the positive
        // `replica` premise, `active` by the `==`. The `unless`
        // negation reads only `this`; its omitted `active` field must
        // lift to a blank wildcard, not a `__N` variable the planner
        // would demand be bound.
        let omitted = must_parse(
            r#"
rule!:
  assert!: binder
  when:
    - assert: replica
      where: { this: ?this }
    - assert: ==
      where: { this: ?active, is: about:blank }
  unless:
    - assert: binder
      where: { this: ?this }
"#,
        );
        assert!(
            analyze_with_concepts(&omitted, &specs).await.is_ok(),
            "a negation premise omitting a concept field should compile",
        );

        // Same rule, but `unless` writes `active` as an explicit
        // blank (`_`) rather than omitting it.
        let blanked = must_parse(
            r#"
rule!:
  assert!: binder
  when:
    - assert: replica
      where: { this: ?this }
    - assert: ==
      where: { this: ?active, is: about:blank }
  unless:
    - assert: binder
      where: { this: ?this, active: _ }
"#,
        );
        assert!(
            analyze_with_concepts(&blanked, &specs).await.is_ok(),
            "a negation premise with an explicit `_` field should compile",
        );
    }

    /// `transient_entities()` returns exactly the concept
    /// entities asserted against a `transient:` concept — the
    /// durable concept's entity is absent.
    #[dialog_common::test]
    async fn it_collects_only_transient_concept_entities() {
        let syntax = must_parse(
            r#"
concept!: &ping
  transient:
  with:
    tag:
      description: "Tag"
      the:         io.gozala.ping/tag
      as:          Text
      cardinality: one
concept!: &pong
  with:
    tag:
      description: "Tag"
      the:         io.gozala.pong/tag
      as:          Text
      cardinality: one
ping!:
  this: did:key:zPingSubject
  tag:  "hi"
pong!:
  this: did:key:zPongSubject
  tag:  "bye"
"#,
        );
        let tree = analyze_empty(&syntax).await.unwrap();
        let transient = tree.analysis.transient_entities();

        assert_eq!(
            transient.len(),
            1,
            "exactly one concept — ping — is transient; got {transient:?}"
        );
    }

    /// `command!:` is a transient concept: the same body written as
    /// `command!:` and as `concept!:` + `transient:` must derive the
    /// same concept entity and the same transient classification, so
    /// the committed facts (and the wire shape) are identical.
    #[dialog_common::test]
    async fn it_defines_command_as_transient_concept() {
        let command_doc = must_parse(
            r#"
command!: &ping
  with:
    tag:
      description: "Tag"
      the:         io.gozala.ping/tag
      as:          Text
      cardinality: one
ping!:
  this: did:key:zPingSubject
  tag:  "hi"
"#,
        );
        let concept_doc = must_parse(
            r#"
concept!: &ping
  transient:
  with:
    tag:
      description: "Tag"
      the:         io.gozala.ping/tag
      as:          Text
      cardinality: one
ping!:
  this: did:key:zPingSubject
  tag:  "hi"
"#,
        );

        let command_transient = analyze_empty(&command_doc)
            .await
            .unwrap()
            .analysis
            .transient_entities();
        let concept_transient = analyze_empty(&concept_doc)
            .await
            .unwrap()
            .analysis
            .transient_entities();

        assert_eq!(
            command_transient.len(),
            1,
            "the command's concept entity is transient; got {command_transient:?}"
        );
        assert_eq!(
            command_transient, concept_transient,
            "command!: and concept!: + transient: derive the same transient concept entity"
        );
    }

    /// `has_statements()` is the `/evaluate` route's commit
    /// signal. A query-only document reports `false`; a
    /// `rule!:`-only document reports `true` (a rule is a
    /// mutation). `has_no_queries()` is the mirror for the read
    /// side.
    #[dialog_common::test]
    async fn it_reports_statement_and_query_presence() {
        let specs = [
            fixed_concept("person", &[("name", "io.gozala.person/name")]),
            fixed_concept("greeting", &[("greeting", "io.gozala.greeting/greeting")]),
        ];

        // Query-only — no statements, has a query.
        let query_only =
            analyze_with_concepts(&must_parse("person:\n  this: ?p\n  name: ?n\n"), &specs)
                .await
                .unwrap();
        assert!(
            !query_only.analysis.has_statements(),
            "a query-only document has no planned statements"
        );
        assert!(
            !query_only.analysis.has_no_queries(),
            "a query-only document carries a query"
        );

        // Rule-only — has statements (the rule is a mutation),
        // no query.
        let rule_only = analyze_with_concepts(
            &must_parse(
                "rule!:\n  assert!: greeting\n  when:\n    - assert: person\n      \
                 where: { this: ?this, name: ?greeting }\n",
            ),
            &specs,
        )
        .await
        .unwrap();
        assert!(
            rule_only.analysis.has_statements(),
            "a rule-only document has a planned statement — the rule is a mutation"
        );
        assert!(
            rule_only.analysis.has_no_queries(),
            "a rule-only document carries no query"
        );
    }

    /// `queries()` yields the user-written query expressions in
    /// document order, skipping assertions and rules.
    #[dialog_common::test]
    async fn it_yields_query_nodes_in_document_order() {
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let syntax = must_parse(
            r#"
person:
  this: ?a
  name: ?n
person!:
  this: did:key:zQueryOrder
  name: "Alice"
person:
  this: ?b
  name: ?m
"#,
        );
        let tree = analyze_with(&syntax, &resolver).await.unwrap();
        let labels: Vec<&str> = tree
            .analysis
            .queries()
            .map(|q| q.analysis.label.as_str())
            .collect();

        // Two query expressions, the assertion between them
        // skipped — order preserved.
        assert_eq!(
            labels,
            vec!["person", "person"],
            "queries() yields both query nodes, in document order, skipping the assertion"
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
        let analysis = flat(analyze_empty(&syntax).await.unwrap());
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

#[cfg(test)]
mod library_analysis_tests {
    use super::*;

    /// Every shipped library file must analyze.
    ///
    /// They are fetched and lowered at repository creation, so a rule the
    /// analyzer rejects does not fail a build — it fails every new space,
    /// at runtime, with no test having said so.
    #[test]
    fn it_analyzes_the_shipped_libraries() {
        // Core first: the other files reference concepts core declares
        // (`view`, `route`, `name`), and lowering runs against a branch that
        // already has them. Analyzing a file alone reports those as unknown
        // concepts, which says nothing about the file. `profile.yaml`
        // declares its own copies of the shared concepts, so it analyzes
        // standalone.
        let chained: [(&str, &str); 3] = [
            (
                "notebook.yaml",
                include_str!("../../tonk-core/assets/library/notebook.yaml"),
            ),
            (
                "prose.yaml",
                include_str!("../../tonk-core/assets/library/prose.yaml"),
            ),
            (
                "table.yaml",
                include_str!("../../tonk-core/assets/library/table.yaml"),
            ),
        ];
        let core = include_str!("../../tonk-core/assets/library/core.yaml");
        for (name, body) in chained {
            assert_analyzes(name, &format!("{core}\n{body}"));
        }
        assert_analyzes(
            "profile.yaml",
            include_str!("../../tonk-core/assets/library/profile.yaml"),
        );
    }

    fn assert_analyzes(name: &str, source: &str) {
        let parsed = tonk_notation::parse(source);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name} must parse: {:#?}",
            parsed.diagnostics
        );
        let syntax = parsed
            .syntax
            .unwrap_or_else(|| panic!("{name} yields a syntax tree"));
        let result = analyze_local(&syntax);
        assert!(result.is_ok(), "{name} must analyze: {:?}", result.err());
    }
}
