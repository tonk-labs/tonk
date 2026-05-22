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
//! The tree is the interface every consumer reads — `evaluate.rs`,
//! `effects.rs`, the worker route all walk it directly. The
//! accessor methods on [`DocumentAnalysis`] project the
//! per-expression nodes into the document-order views (queries,
//! statements, transient entities) the evaluator drives off.

use std::collections::{HashMap, HashSet};

use dialog_artifacts::Entity;
use tonk_notation::{Assertion, Expression, Query, Rule, Syntax};

use crate::analyzer::AnalyzeDiagnostic;
use tonk_core::effect::Effect;
use tonk_core::mutation::ConceptDescriptor;
use tonk_core::transact::{Application, Statement, ThisIntent};

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
    /// Implicit snapshot queries the analyzer synthesized so the
    /// editor's before/after view surfaces a mutated entity even
    /// when the user wrote no query for it. Kept separate from
    /// the user-written queries inside `expressions`: a snapshot
    /// of a fresh assert target reads a not-yet-existing entity
    /// and returns zero rows, so it must never join into the
    /// frame set that feeds mutation planning.
    pub synthesized: Vec<SynthesizedQuery>,
    /// `name` → entity. Anchor-form heads.
    pub declarations: HashMap<String, Entity>,
    /// `?foo` → entity. Variable-form heads with content-derived
    /// entities.
    pub variables: HashMap<String, Entity>,
    /// Non-fatal findings accumulated during the pass.
    pub diagnostics: Vec<AnalyzeDiagnostic>,
}

/// An analyzer-synthesized snapshot query — an [`Application`]
/// plus the originating assertion's head name for display.
#[derive(Debug, Clone)]
pub struct SynthesizedQuery {
    /// The snapshot query to evaluate.
    pub application: Application,
    /// Display label — the originating assertion's head name.
    pub label: String,
}

/// One planned statement in document-apply order, with its
/// display label and the declaration flag.
///
/// The flat projection of the tree's write side: walking
/// [`DocumentAnalysis::statements`] yields these in the order the
/// evaluator must apply them.
#[derive(Debug, Clone)]
pub struct PlannedStatement {
    /// The lowered statement.
    pub statement: Statement,
    /// Display label — `None` for declaration-head statements.
    pub label: Option<String>,
    /// `true` when this came from an `attribute!` / `concept!`
    /// declaration head.
    pub declaration: bool,
}

impl DocumentAnalysis {
    /// The user-written query expressions, in document order.
    pub fn queries(&self) -> impl Iterator<Item = &Analysis<Query>> {
        self.expressions.iter().filter_map(|e| match &e.analysis {
            ExpressionAnalysis::Query(node) => Some(node.as_ref()),
            _ => None,
        })
    }

    /// `true` when the document carries no query expression.
    pub fn has_no_queries(&self) -> bool {
        self.queries().next().is_none()
    }

    /// Every planned statement, in the order the evaluator must
    /// apply them — each assertion's `claims` in document order,
    /// then a trailing [`Statement::InstallEffect`] per `rule!:`.
    pub fn statements(&self) -> Vec<PlannedStatement> {
        let mut out = Vec::new();
        for expression in &self.expressions {
            if let ExpressionAnalysis::Assertion(node) = &expression.analysis {
                let assertion = &node.analysis;
                for (statement, label) in assertion.claims.iter().zip(&assertion.labels) {
                    out.push(PlannedStatement {
                        statement: statement.clone(),
                        label: label.clone(),
                        declaration: assertion.declaration,
                    });
                }
            }
        }
        // Rule-lifted effects land after every assertion's
        // statements — the analyzer appends them last.
        for expression in &self.expressions {
            if let ExpressionAnalysis::Rule(node) = &expression.analysis {
                out.push(PlannedStatement {
                    statement: Statement::InstallEffect(node.analysis.effect.clone()),
                    label: None,
                    declaration: false,
                });
            }
        }
        out
    }

    /// `true` when the document has at least one planned
    /// statement — the single commit signal for the `/evaluate`
    /// route. A `rule!:` is a mutation, so a rule-only document
    /// reports `true`.
    pub fn has_statements(&self) -> bool {
        self.expressions.iter().any(|e| match &e.analysis {
            ExpressionAnalysis::Assertion(node) => !node.analysis.claims.is_empty(),
            ExpressionAnalysis::Rule(_) => true,
            ExpressionAnalysis::Query(_) => false,
        })
    }

    /// Concept entities whose facts are transient — an `Assert`
    /// against one of these is routed into the effects-fixpoint
    /// seed and swept before the durable commit.
    pub fn transient_entities(&self) -> HashSet<Entity> {
        let mut out = HashSet::new();
        for expression in &self.expressions {
            if let ExpressionAnalysis::Assertion(node) = &expression.analysis
                && let Some(entity) = &node.analysis.transient
            {
                out.insert(entity.clone());
            }
        }
        out
    }
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
