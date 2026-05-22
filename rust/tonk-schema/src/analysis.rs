//! The `Analysis<T>` tree IR.
//!
//! `analyze`'s output is structurally an `Analysis<T>` — each
//! parsed [`tonk_notation`] syntax node paired with the analysis
//! computed for it, threaded through one generic. See
//! `plan/runtime.md`, section "The Analysis<T> tree".
//!
//! ```text
//! Analysis<Syntax>      .analysis = DocumentAnalysis
//!   Analysis<Expression>  .analysis = ExpressionAnalysis (per variant)
//!     Analysis<Query>       .analysis = QueryNodeAnalysis
//!     Analysis<Assertion>   .analysis = AssertionAnalysis
//!     Analysis<Rule>        .analysis = RuleAnalysis
//! ```
//!
//! The tree mirrors the document: one [`Analysis<Expression>`]
//! per top-level expression, in document order. One-to-many
//! lowering (an anchored assertion → two claims) is the
//! associated type holding a `Vec` — the claims nest *under* the
//! one [`Analysis<Assertion>`], so there is no back-pointer and
//! no flat claim list.
//!
//! This increment lands the tree types and has the analyzer
//! produce the tree internally; [`Analysis::flatten`] reconstructs
//! the flat [`crate::transact::Analysis`] every existing consumer
//! still expects. Migrating consumers to walk the tree directly
//! is a later increment.

use std::collections::{HashMap, HashSet};

use dialog_artifacts::Entity;
use tonk_notation::{Assertion, Expression, Query, Rule, Syntax};

use crate::analyzer::AnalyzeDiagnostic;
use crate::effect::Effect;
use crate::mutation::ConceptDescriptor;
use crate::transact::{self, Application, MutationAnalysis, QueryAnalysis, Statement, ThisIntent};

/// A syntax node type that carries a computed analysis payload.
///
/// The pairing is structural — diagnostics and result projection
/// read the span / head name straight off the `source` side of
/// [`Analysis`], the computed annotations off the `analysis` side.
pub trait Analyzable {
    /// The analysis payload computed for this syntax node.
    type Analysis;
}

/// A syntax node paired with its analysis.
///
/// `Analysis<Syntax>` is the whole document, `Analysis<Expression>`
/// is one top-level expression, `Analysis<Assertion>` is one
/// assertion. The generic threads the syntax/analysis pairing
/// through every level of the tree.
#[derive(Debug, Clone)]
pub struct Analysis<T: Analyzable> {
    /// The parsed syntax node this analysis was computed for.
    pub source: T,
    /// The analysis computed for `source`.
    pub analysis: T::Analysis,
}

// ---------------------------------------------------------------- //
// Document level                                                   //
// ---------------------------------------------------------------- //

impl Analyzable for Syntax {
    type Analysis = DocumentAnalysis;
}

/// Document-level analysis: the per-expression subtree plus the
/// document-wide tables (`declarations`, `variables`) and the
/// accumulated `diagnostics`.
#[derive(Debug, Clone, Default)]
pub struct DocumentAnalysis {
    /// One analysed node per top-level expression, in document
    /// order — the tree mirrors the document.
    pub expressions: Vec<Analysis<Expression>>,
    /// `name` → entity. Anchor-form heads. Mirrors
    /// [`transact::Analysis::declarations`].
    pub declarations: HashMap<String, Entity>,
    /// `?foo` → entity. Variable-form heads with content-derived
    /// entities. Mirrors [`transact::Analysis::variables`].
    pub variables: HashMap<String, Entity>,
    /// Non-fatal findings accumulated during the pass.
    pub diagnostics: Vec<AnalyzeDiagnostic>,
}

// ---------------------------------------------------------------- //
// Expression level                                                 //
// ---------------------------------------------------------------- //

impl Analyzable for Expression {
    type Analysis = ExpressionAnalysis;
}

/// Per-expression analysis, dispatched on the expression variant.
/// Each arm holds the sub-node whose `.source` is the concrete
/// payload type (`Query`, `Assertion`, `Rule`).
#[derive(Debug, Clone)]
pub enum ExpressionAnalysis {
    /// A `head:` query expression.
    Query(Box<Analysis<Query>>),
    /// A `head!:` assertion expression.
    Assertion(Box<Analysis<Assertion>>),
    /// A `rule!:` inductive-rule expression.
    Rule(Box<Analysis<Rule>>),
}

// ---------------------------------------------------------------- //
// Query                                                            //
// ---------------------------------------------------------------- //

impl Analyzable for Query {
    type Analysis = QueryNodeAnalysis;
}

/// Analysis of a single `head:` query expression.
#[derive(Debug, Clone)]
pub struct QueryNodeAnalysis {
    /// The built [`Application`] for this query — bare-symbol and
    /// analysis-time `?var` references already substituted.
    pub application: Application,
    /// Display label — the head's source name.
    pub label: String,
}

// ---------------------------------------------------------------- //
// Assertion                                                        //
// ---------------------------------------------------------------- //

impl Analyzable for Assertion {
    type Analysis = AssertionAnalysis;
}

/// Analysis of a single `head!:` assertion expression.
///
/// `predicate`, `this`, and `anchor` capture the resolved source
/// shape (the `resolve` phase). `claims` is the lowered,
/// kernel-shaped write (the `expand` phase) — for this increment
/// it holds the [`Statement`]s the analyzer's Phase 3 produced
/// for this assertion (an assert side, a retract side, or both).
#[derive(Debug, Clone)]
pub struct AssertionAnalysis {
    /// What this assertion's head resolved to — a concept
    /// descriptor (durable or transient) or a claim domain.
    pub predicate: Predicate,
    /// Where the entity in `this:` comes from.
    pub this: ThisIntent,
    /// `&anchor` on the value side, if any.
    pub anchor: Option<String>,
    /// The lowered statements this assertion produced, in the
    /// order they must be applied (retract before assert). Plays
    /// the role of the spec's `claims: Vec<Claim>` — `Claim` as a
    /// distinct kernel type does not exist yet, so the lowered
    /// form is the [`Statement`] the current analyzer emits.
    pub claims: Vec<Statement>,
    /// `true` when this assertion came from an `attribute!` /
    /// `concept!` declaration head — its statements are excluded
    /// from auto-snapshot synthesis.
    pub declaration: bool,
    /// Display label for each entry of `claims`, parallel to it.
    /// `None` for declaration-head statements.
    pub labels: Vec<Option<String>>,
    /// Concept entity to record as transient, if this assertion's
    /// head resolved to a transient concept and produced an
    /// `Assert`.
    pub transient: Option<Entity>,
}

/// What an assertion's head resolved to.
#[derive(Debug, Clone)]
pub enum Predicate {
    /// A resolved concept — durable or transient.
    Concept(ConceptDescriptor),
    /// A claim domain (`xyz.tonk`) — has no schema.
    Domain(String),
}

// ---------------------------------------------------------------- //
// Rule                                                             //
// ---------------------------------------------------------------- //

impl Analyzable for Rule {
    type Analysis = RuleAnalysis;
}

/// Analysis of a single `rule!:` expression — the lifted
/// inductive [`Effect`] it installs.
#[derive(Debug, Clone)]
pub struct RuleAnalysis {
    /// The effect lifted from the rule, installed as a
    /// [`Statement::InstallEffect`].
    pub effect: Effect,
}

// ---------------------------------------------------------------- //
// flatten — the bridge to the flat transact::Analysis              //
// ---------------------------------------------------------------- //

impl Analysis<Syntax> {
    /// Reconstruct the flat [`transact::Analysis`] from the tree.
    ///
    /// The bridge for this increment: every existing consumer
    /// (`evaluate.rs`, `effects.rs`, the renderer, the tests)
    /// reads the flat struct, so `analyze` walks the tree it built
    /// and reassembles the flat form bit-for-bit.
    ///
    /// Reassembly order matters and mirrors the analyzer's phases:
    ///
    /// - **queries** — every `Analysis<Query>` in document order
    ///   contributes one entry to `query.queries` / `query.labels`.
    /// - **statements** — every `Analysis<Assertion>` contributes
    ///   its `claims` (retract before assert, declaration inline
    ///   attributes already ordered inside `claims`); every
    ///   `Analysis<Rule>` contributes a trailing
    ///   `Statement::InstallEffect`.
    /// - **synthesized** — auto-snapshot queries are derived from
    ///   the assembled statements, exactly as the analyzer's
    ///   Phase 4 does.
    pub fn flatten(self) -> transact::Analysis {
        let DocumentAnalysis {
            expressions,
            declarations,
            variables,
            diagnostics,
        } = self.analysis;

        let mut queries: Vec<Application> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        let mut statements: Vec<Statement> = Vec::new();
        let mut statement_labels: Vec<Option<String>> = Vec::new();
        let mut declaration_statement_indexes: HashSet<usize> = HashSet::new();
        let mut requires: HashSet<String> = HashSet::new();
        let mut transient: HashSet<Entity> = HashSet::new();
        let mut effects: Vec<Effect> = Vec::new();

        for expression in expressions {
            match expression.analysis {
                ExpressionAnalysis::Query(node) => {
                    queries.push(node.analysis.application);
                    labels.push(node.analysis.label);
                }
                ExpressionAnalysis::Assertion(node) => {
                    let assertion = node.analysis;
                    if let Some(entity) = assertion.transient {
                        transient.insert(entity);
                    }
                    for (statement, label) in assertion.claims.into_iter().zip(assertion.labels) {
                        if let Some(application) = statement.application() {
                            collect_unbound(application, &variables, &mut requires);
                        }
                        if assertion.declaration {
                            declaration_statement_indexes.insert(statements.len());
                        }
                        statements.push(statement);
                        statement_labels.push(label);
                    }
                }
                ExpressionAnalysis::Rule(node) => {
                    effects.push(node.analysis.effect);
                }
            }
        }

        // Rule-lifted effects land after every assertion's
        // statements — the analyzer appends them in Phase 3b.
        for effect in effects {
            statements.push(Statement::InstallEffect(effect));
            statement_labels.push(None);
        }

        let query = if queries.is_empty() {
            None
        } else {
            Some(QueryAnalysis {
                queries,
                labels,
                ..Default::default()
            })
        };

        let mut analysis = transact::Analysis {
            declarations,
            variables,
            query,
            mutate: MutationAnalysis {
                statements,
                requires,
                transient,
            },
            diagnostics,
        };

        synthesize_implicit_queries(
            &mut analysis,
            &statement_labels,
            &declaration_statement_indexes,
        );

        analysis
    }
}

/// Variable names an [`Application`] reads that are not bound by
/// an analysis-time `variables` entry — they must be supplied by
/// a query at planning time. Mirrors the analyzer's
/// `collect_unbound_variables`.
fn collect_unbound(
    application: &Application,
    variables: &HashMap<String, Entity>,
    out: &mut HashSet<String>,
) {
    for name in application.bindings() {
        if !variables.contains_key(&name) {
            out.insert(name);
        }
    }
}

/// Synthesize auto-snapshot queries for mutation-touched
/// entities. A re-export of the analyzer's Phase 4 so `flatten`
/// produces the exact same `synthesized` block.
fn synthesize_implicit_queries(
    analysis: &mut transact::Analysis,
    statement_labels: &[Option<String>],
    declaration_statement_indexes: &HashSet<usize>,
) {
    crate::analyzer::synthesize_implicit_queries(
        analysis,
        statement_labels,
        declaration_statement_indexes,
    );
}
