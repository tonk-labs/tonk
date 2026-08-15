//! The `Analysis<T>` tree IR.
//!
//! `analyze`'s output is structurally an `Analysis<T>` — each
//! parsed [`tonk_notation`] syntax node paired with the analysis
//! computed for it, threaded through one generic. See
//! `plan/runtime.md`, section "The Analysis<T> tree".
//!
//! ```text
//! Analysis<Syntax>         .analysis = DocumentAnalysis
//!   Analysis<Expression>   .analysis = ExpressionAnalysis (per variant)
//!     Analysis<Application>  .analysis = QueryNodeAnalysis   (queries)
//!     Analysis<Application>  .analysis = AssertionAnalysis   (claims)
//! ```
//!
//! Claims and queries share the same syntactic shape
//! ([`tonk_notation::Application`]) — only the wrapping
//! [`Expression`] variant distinguishes them. A `rule!:` is a
//! claim whose predicate is the built-in `rule` concept; when the
//! analyzer recognises it the produced [`AssertionAnalysis`]
//! carries an `effect` payload that turns the lowered statement
//! into a [`Statement::InstallEffect`] instead of a per-field
//! claim.
//!
//! The tree mirrors the document: one [`Analysis<Expression>`]
//! per top-level expression, in document order. One-to-many
//! lowering (an anchored assertion → two claims) is the
//! associated type holding a `Vec` — the claims nest *under* the
//! one [`Analysis<Application>`], so there is no back-pointer and
//! no flat claim list.
//!
//! The tree is the interface every consumer reads — `evaluate.rs`,
//! `effects.rs`, the worker route all walk it directly. The
//! accessor methods on [`DocumentAnalysis`] project the
//! per-expression nodes into the document-order views (queries,
//! statements, transient entities) the evaluator drives off.

use std::collections::{HashMap, HashSet};

use dialog_artifacts::Entity;
use tonk_notation::{Application as SyntaxApplication, Expression, Syntax};

use crate::analyzer::AnalyzeDiagnostic;
use tonk_core::claim::{ConceptDescriptor, SourceApplication, SourceClaim, TransactRequest};
use tonk_core::effect::Effect;
use tonk_schema::transact::{Application, Statement, ThisIntent};

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
    pub fn queries(&self) -> impl Iterator<Item = &Analysis<QueryNode>> {
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
    /// apply them: every non-rule claim's `claims` in document
    /// order, then every `rule!:` claim's lone
    /// [`Statement::InstallEffect`]. Rule installs come last
    /// regardless of source position because the evaluator commits
    /// claim facts before reading them through rule premises;
    /// installing a rule whose body references not-yet-asserted
    /// facts would produce no novelty on first commit.
    pub fn statements(&self) -> Vec<PlannedStatement> {
        let mut out = Vec::new();
        // Pass 1: every non-rule claim's lowered statements, in
        // document order. A rule claim sets `effect = Some(_)` on
        // its analysis, so we skip those here.
        for expression in &self.expressions {
            if let ExpressionAnalysis::Assertion(node) = &expression.analysis
                && node.analysis.effect.is_none()
            {
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
        // Pass 2: rule installs come last so any concept facts the
        // rule body reads are already on the branch.
        for expression in &self.expressions {
            if let ExpressionAnalysis::Assertion(node) = &expression.analysis
                && node.analysis.effect.is_some()
            {
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
        out
    }

    /// `true` when the document has at least one planned
    /// statement — the single commit signal for the `/evaluate`
    /// route. A `rule!:` is a claim, so a rule-only document
    /// reports `true` through the regular Assertion path.
    pub fn has_statements(&self) -> bool {
        self.expressions.iter().any(|e| match &e.analysis {
            ExpressionAnalysis::Assertion(node) => !node.analysis.claims.is_empty(),
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

    /// Lower the analyzed document into a [`TransactRequest`] — the
    /// typed wire shape the `/transact` route accepts and the
    /// `claim!` macro emits. Every planned statement becomes a
    /// [`Claim`], its predicate tagged durable or transient from
    /// [`transient_entities`](Self::transient_entities).
    ///
    /// Only concept applications lower; a `Domain` or `Rule`
    /// application has no [`TransactRequest`] representation yet, so
    /// the document is rejected. (Bootstrap documents — the macro's
    /// only input today — are concept claims throughout.)
    pub fn lower_to_claims(&self) -> Result<TransactRequest, AnalyzeLowerError> {
        let transient = self.transient_entities();
        let mut claims = Vec::new();
        for planned in self.statements() {
            // Rule installs/retracts have no `Claim` representation;
            // they are carried out-of-band by `lower_to_rules`. Skip
            // them here so a bootstrap that mixes concept claims and
            // `rule!:` blocks still lowers its claims cleanly.
            if matches!(
                &planned.statement,
                Statement::Assert(Application::Rule { .. })
                    | Statement::Retract(Application::Rule { .. })
                    | Statement::Assert(Application::DeductiveRule { .. })
                    | Statement::Retract(Application::DeductiveRule { .. })
            ) {
                continue;
            }
            let claim = lower_statement(&planned.statement, &transient)?;
            claims.push(claim);
        }
        Ok(TransactRequest { claims })
    }

    /// Lift every `rule!:` install in the document into a
    /// [`Rule`](tonk_schema::rule::Rule), in the same
    /// document/eval order as [`statements`](Self::statements). These
    /// have no [`TransactRequest`] representation (the `Claim` wire
    /// can't carry `db.effect/*` triples), so they are returned
    /// separately for a seed loop to `assert` directly.
    ///
    /// Only `assert` rule installs are returned; a rule retract has
    /// no place in a fresh bootstrap.
    pub fn rule_installs(&self) -> Vec<tonk_schema::rule::Rule> {
        let mut rules = Vec::new();
        for planned in self.statements() {
            if let Statement::Assert(Application::Rule { rule, .. }) = &planned.statement {
                rules.push((**rule).clone());
            }
        }
        rules
    }

    /// Lift every deductive `rule!:` install (the `assert:` no-bang
    /// form) in the document into a
    /// [`DeductiveRule`](tonk_schema::deductive_rule::DeductiveRule),
    /// in document/eval order. Like [`rule_installs`](Self::rule_installs),
    /// these have no [`TransactRequest`] representation and are returned
    /// separately for a seed loop to `assert` directly.
    pub fn deductive_rule_installs(&self) -> Vec<tonk_schema::deductive_rule::DeductiveRule> {
        let mut rules = Vec::new();
        for planned in self.statements() {
            if let Statement::Assert(Application::DeductiveRule { rule, .. }) = &planned.statement {
                rules.push((**rule).clone());
            }
        }
        rules
    }
}

/// Error from [`DocumentAnalysis::lower_to_claims`] — an
/// application shape with no `TransactRequest` representation.
#[derive(Debug, thiserror::Error)]
pub enum AnalyzeLowerError {
    /// A `rule!:` install/retract — rules aren't representable as
    /// `Claim`s on the structured transact wire (yet).
    #[error("rule applications cannot be lowered to a transact claim")]
    Rule,
    /// A bare `xyz.tonk/...` domain claim — no concept descriptor
    /// to carry on the predicate.
    #[error("domain applications cannot be lowered to a transact claim")]
    Domain,
    /// A parameter slot held a logic variable or blank rather than
    /// a concrete value, so it can't be carried on the wire.
    #[error("parameter {field:?} is not a concrete value (got {term:?})")]
    NonConstantTerm {
        /// Field whose binding wasn't a constant.
        field: String,
        /// Debug rendering of the offending term.
        term: String,
    },
}

/// Lower one [`Statement`] to a wire [`Claim`], tagging the
/// predicate's durability from the transient-entity set.
fn lower_statement(
    statement: &Statement,
    transient: &HashSet<Entity>,
) -> Result<SourceClaim, AnalyzeLowerError> {
    let (application, is_assert) = match statement {
        Statement::Assert(app) => (app, true),
        Statement::Retract(app) => (app, false),
    };
    let (query, name) = match application {
        Application::Concept { query, name, .. } => (query, name.clone()),
        Application::Domain { .. } => return Err(AnalyzeLowerError::Domain),
        Application::Rule { .. } | Application::DeductiveRule { .. } => {
            return Err(AnalyzeLowerError::Rule);
        }
    };
    let this = query.terms.get("this").and_then(term_entity);
    let descriptor = query.predicate.clone();
    let predicate = if this.is_some_and(|e| transient.contains(&e)) {
        ConceptDescriptor::Transient(descriptor)
    } else {
        ConceptDescriptor::Durable(descriptor)
    };
    let mut parameters = tonk_core::claim::ValueMap::new();
    for (key, term) in query.terms.iter() {
        let value = match term {
            dialog_query::Term::Constant(v) => v.clone(),
            other => {
                return Err(AnalyzeLowerError::NonConstantTerm {
                    field: key.clone(),
                    term: format!("{other:?}"),
                });
            }
        };
        parameters.insert(key.clone(), value);
    }
    let application = SourceApplication {
        predicate,
        parameters,
        name,
    };
    Ok(if is_assert {
        SourceClaim::Assert(application)
    } else {
        SourceClaim::Retract(application)
    })
}

/// Extract a bound entity from a term, if it carries one.
fn term_entity(term: &dialog_query::Term<dialog_query::Any>) -> Option<Entity> {
    match term {
        dialog_query::Term::Constant(dialog_artifacts::Value::Entity(entity)) => {
            Some(entity.clone())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- //
// Expression level                                                 //
// ---------------------------------------------------------------- //

impl Analyzable for Expression {
    type Analysis = ExpressionAnalysis;
}

/// Per-expression analysis, dispatched on the expression variant.
/// Each arm holds a sub-node whose `.source` is a
/// [`tonk_notation::Application`] — the same syntactic shape for
/// queries and claims; the analysis payload differs by arm. We
/// thread that distinction through marker newtypes
/// ([`QueryNode`] / [`AssertionNode`]) so a single [`Analyzable`]
/// impl per marker keeps the generic happy.
#[derive(Debug, Clone)]
pub enum ExpressionAnalysis {
    /// A `head:` query expression.
    Query(Box<Analysis<QueryNode>>),
    /// A `head!:` claim expression (assert or retract). A `rule!:`
    /// claim lands here too, with the lifted [`Effect`] carried on
    /// the [`AssertionAnalysis::effect`] field.
    Assertion(Box<Analysis<AssertionNode>>),
}

// ---------------------------------------------------------------- //
// Application — the syntax shape shared by queries and claims      //
// ---------------------------------------------------------------- //
//
// Both arms wrap a [`tonk_notation::Application`] (queries are bare,
// claims wrap it in an [`Effectful`]). The marker newtypes below
// keep the `Analyzable` impls distinct so [`Analysis<QueryNode>`]
// uses `QueryNodeAnalysis` and [`Analysis<AssertionNode>`] uses
// `AssertionAnalysis`.

/// Marker wrapping a query's source [`Application`].
#[derive(Debug, Clone)]
pub struct QueryNode {
    /// The query's application as it appeared in the document.
    pub source: SyntaxApplication,
}

/// Marker wrapping a claim's source [`Application`] (plus the
/// optional anchor lifted off the [`tonk_notation::Effectful`]).
#[derive(Debug, Clone)]
pub struct AssertionNode {
    /// The claim's application as it appeared in the document.
    pub source: SyntaxApplication,
    /// `&anchor` from the `Effectful` wrapper, if any.
    pub anchor: Option<tonk_notation::Anchor>,
}

impl Analyzable for QueryNode {
    type Analysis = QueryNodeAnalysis;
}

impl Analyzable for AssertionNode {
    type Analysis = AssertionAnalysis;
}

/// Analysis of a single `head:` query expression.
///
/// Queries and claims share the same notation shape, so this lives
/// alongside [`AssertionAnalysis`]; the [`ExpressionAnalysis`]
/// wrapper picks which one applies to a given node.
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

/// Analysis of a single `head!:` claim expression.
///
/// `predicate`, `this`, and `anchor` capture the resolved source
/// shape (the `resolve` phase). `claims` is the lowered,
/// kernel-shaped write (the `expand` phase) — for an ordinary claim
/// it holds the [`Statement`]s for assert / retract; for a `rule!:`
/// claim it holds a single [`Statement::InstallEffect`].
#[derive(Debug, Clone)]
pub struct AssertionAnalysis {
    /// What this claim's head resolved to — a concept descriptor
    /// (durable or transient) or a claim domain.
    pub predicate: Predicate,
    /// Where the entity in `this:` comes from.
    pub this: ThisIntent,
    /// `&anchor` on the value side, if any.
    pub anchor: Option<String>,
    /// The lowered statements this claim produced, in the order
    /// they must be applied (retract before assert).
    pub claims: Vec<Statement>,
    /// `true` when this claim came from an `attribute!` /
    /// `concept!` declaration head — its statements are excluded
    /// from auto-snapshot synthesis.
    pub declaration: bool,
    /// Display label for each entry of `claims`, parallel to it.
    /// `None` for declaration-head statements.
    pub labels: Vec<Option<String>>,
    /// Concept entity to record as transient, if this claim's head
    /// resolved to a transient concept and produced an `Assert`.
    pub transient: Option<Entity>,
    /// `Some(effect)` when this claim's predicate is the built-in
    /// `rule` concept and the body lifts cleanly into an
    /// inductive [`Effect`]. The lowered [`Statement`] then sits in
    /// `claims` as a [`Statement::InstallEffect`], so the regular
    /// document-order walk picks it up like any other write.
    pub effect: Option<Effect>,
}

/// What an assertion's head resolved to.
#[derive(Debug, Clone)]
pub enum Predicate {
    /// A resolved concept — durable or transient.
    Concept(ConceptDescriptor),
    /// A claim domain (`xyz.tonk`) — has no schema.
    Domain(String),
}
