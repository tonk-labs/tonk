//! Shared analyze → compile → evaluate pipeline for
//! asserted-notation.
//!
//! The lifecycle is driven by three chain handles that hang off
//! [`tonk_notation::Syntax`], each a nested prefix of the next:
//!
//! ```ignore
//! use tonk_evaluator::evaluate::{SyntaxAnalyzeExt, SyntaxCompileExt, SyntaxEvaluateExt};
//!
//! syntax.analyze(source).perform(env).await?;   // -> Analysis
//! syntax.compile(source).perform(env).await?;   // -> Compiled
//! syntax.evaluate(txn).perform(&branch, env).await?; // -> Evaluated
//! ```
//!
//! - [`SyntaxAnalyzeExt::analyze`] is pure-read — takes a
//!   [`Source`] (a `&Branch` or `&Transaction`, anything
//!   `Into<Source>`) and yields an [`Analysis`].
//! - [`SyntaxCompileExt::compile`] runs `analyze` under the hood
//!   and yields a [`Compiled`] — a thin handle over the resolved
//!   document's runnable operations.
//! - [`SyntaxEvaluateExt::evaluate`] takes a caller-created
//!   [`Transaction`], runs `compile` under the hood, runs the
//!   operations, and yields an [`Evaluated`] holding the
//!   transaction with the document's changes staged. It does
//!   *not* commit.
//!
//! ```ignore
//! let evaluated = syntax
//!     .evaluate(branch.transaction())
//!     .perform(&branch, env).await?;
//! // evaluated.txn — overlay reflects pending mutations
//! // evaluated.transients — bucket to hand to `induce`
//! // evaluated.matches — pre-mutation per-expression match blocks
//! // evaluated.commits — claim count + entity bindings for the response envelope
//! // evaluated.analysis — re-run queries against the overlay to get post-mutation matches
//! ```
//!
//! [`Evaluated::commit`] is a shortcut that chains `induce` and the
//! durable commit:
//!
//! ```ignore
//! let result = syntax
//!     .evaluate(branch.transaction())
//!     .perform(&branch, env).await?
//!     .commit()
//!     .perform(&branch, env).await?;
//! // result.revision, result.matches_after, ...
//! ```
//!
//! Pre-existing helpers (per-expression query, natural join, match
//! rendering) stay inside this module — they're called both during
//! `Evaluate::perform` (for `matches_before`) and by
//! [`EvaluatedCommit::perform`] (for `matches_after` after commit).

use std::collections::BTreeMap;

use dialog_artifacts::{Changes, Entity, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::{ConceptDescriptor, ConceptQuery, Output as _, Parameters, Term};
use dialog_repository::{Branch, RemoteSite, Revision, Transaction};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_notation::Syntax;

use tonk_analyzer::analysis::{Analysis, ExpressionAnalysis, SynthesizedQuery};
use tonk_analyzer::analyzer;
use tonk_core::transact::{Application, ApplicationPlan, Planner as _, Statement};
use tonk_schema::concept::{QueryEnv, application_to_plan};
use tonk_schema::query_source::Source;

use crate::effect_query::EffectStatement;

// ---------------------------------------------------------------- //
// Public response types                                            //
// ---------------------------------------------------------------- //

/// Matches for one source-expression query, projected back into
/// the user's view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryMatchBlock {
    /// Display label for the source expression
    /// (`person ?alice:` → `"person"`).
    pub label: String,
    /// One entry per matched entity.
    pub results: Vec<QueryResult>,
}

/// One match — an entity plus its bound field values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    /// Canonical entity URI for the match.
    pub this: String,
    /// Field name → bound value.
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// Commit-side summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitSummary {
    /// Number of EAV claims committed (asserts + retracts).
    pub claims: usize,
    /// Variable name (or `"this"` for anonymous heads) →
    /// entity URI for every head the mutation touched.
    pub entities: BTreeMap<String, String>,
}

// ---------------------------------------------------------------- //
// Errors                                                           //
// ---------------------------------------------------------------- //

/// Failure modes for [`Evaluate::perform`]. Callers map these
/// onto whatever envelope they expose (HTTP status, CLI exit
/// code).
#[derive(Debug, Error)]
pub enum EvaluateError {
    /// The analyzer rejected the document (unknown name, type
    /// mismatch, scope violation, etc.). Caller should treat
    /// this as a 400 / parse-error class failure. Carrying the
    /// full [`analyzer::AnalyzeError`] (rather than a flattened
    /// string) lets HTTP callers surface the source range and
    /// stable error code as a structured response — the editor
    /// uses both to position a squiggle and route quickfixes.
    #[error("{0}")]
    Analyze(#[from] analyzer::AnalyzeError),
    /// A query against the branch failed (storage / engine).
    #[error("{0}")]
    Query(String),
    /// Variable substitution into a mutation `Application`
    /// failed (a referenced `?var` isn't bound).
    #[error("{0}")]
    Plan(String),
}

// ---------------------------------------------------------------- //
// Env trait alias                                                  //
// ---------------------------------------------------------------- //

/// Environment bound for [`run`] — covers both query selects
/// (via `Branch::query().select(...).perform`) and commits (via
/// `Branch::commit(...).perform`). Mirrors the union of dialog's
/// `SelectQuery::perform` and `Commit::perform` bounds so the
/// signature stays a single trait alias.
pub trait EvaluateEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> EvaluateEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

// ---------------------------------------------------------------- //
// Lifecycle entry points — three Syntax-hung chain handles          //
//                                                                   //
//   syntax.analyze(source).perform(env)   -> Analysis               //
//   syntax.compile(source).perform(env)   -> Compiled               //
//   syntax.evaluate(txn).perform(branch, env) -> Evaluated          //
//                                                                   //
// Each runs the prior under the hood. `Syntax` is a foreign type    //
// (`tonk_notation::Syntax`), so these are local extension traits    //
// `impl`'d for it here — local trait, foreign type.                 //
// ---------------------------------------------------------------- //

/// Adds [`Self::analyze`] to [`tonk_notation::Syntax`]. The first
/// lifecycle entry point: resolve the document against a source.
pub trait SyntaxAnalyzeExt {
    /// Stage an analysis of this document against `source`.
    /// Returns a chain handle; call `.perform(env)` to run
    /// `resolve` + `expand` and get an [`Analysis`].
    fn analyze<S>(&self, source: S) -> Analyze<'_, S>;
}

impl SyntaxAnalyzeExt for Syntax {
    fn analyze<S>(&self, source: S) -> Analyze<'_, S> {
        Analyze {
            syntax: self,
            source,
        }
    }
}

/// Chain handle for [`SyntaxAnalyzeExt::analyze`]. Holds the
/// syntax and the source until `.perform(env)` consumes them.
pub struct Analyze<'s, S> {
    syntax: &'s Syntax,
    source: S,
}

impl<'s, S> Analyze<'s, S> {
    /// Run `resolve` + `expand` against the source, yielding the
    /// document's [`Analysis<Syntax>`][Analysis] tree. Pure-read
    /// — no mutation, no commit.
    pub async fn perform<'e, Env: QueryEnv>(
        self,
        env: &'e Env,
    ) -> Result<Analysis<Syntax>, EvaluateError>
    where
        S: Into<Source<'e>>,
    {
        let Analyze { syntax, source } = self;
        let resolver = analyzer::SourceResolver::new(source, env);
        analyzer::analyze(syntax, &resolver)
            .await
            .map_err(EvaluateError::Analyze)
    }
}

/// Adds [`Self::compile`] to [`tonk_notation::Syntax`]. The
/// second lifecycle entry point: `analyze`, then lower the
/// resolved document to runnable operations.
pub trait SyntaxCompileExt {
    /// Stage a compilation of this document against `source`.
    /// Returns a chain handle; call `.perform(env)` to run
    /// `analyze` and lower the result to a [`Compiled`].
    fn compile<S>(&self, source: S) -> Compile<'_, S>;
}

impl SyntaxCompileExt for Syntax {
    fn compile<S>(&self, source: S) -> Compile<'_, S> {
        Compile {
            syntax: self,
            source,
        }
    }
}

/// Chain handle for [`SyntaxCompileExt::compile`].
pub struct Compile<'s, S> {
    syntax: &'s Syntax,
    source: S,
}

impl<'s, S> Compile<'s, S> {
    /// Run `analyze` under the hood, then carry the resolved
    /// document's runnable operations in a [`Compiled`]. Pure-read.
    pub async fn perform<'e, Env: QueryEnv>(self, env: &'e Env) -> Result<Compiled, EvaluateError>
    where
        S: Into<Source<'e>>,
    {
        let Compile { syntax, source } = self;
        let analysis = syntax.analyze(source).perform(env).await?;
        Ok(Compiled { analysis })
    }
}

/// Result of [`Compile::perform`] — the resolved document's
/// runnable operations.
///
/// `Compiled` is a thin handle over the [`Analysis<Syntax>`][Analysis]
/// tree: the read side is the per-expression query nodes, the
/// write side is the planned [`Statement`]s nested under each
/// assertion — including the [`Statement::InstallEffect`] each
/// `rule!:` lifts into. `evaluate` walks the tree and runs those
/// operations against a transaction.
pub struct Compiled {
    /// The resolved, lowered document tree.
    pub analysis: Analysis<Syntax>,
}

/// Adds [`Self::evaluate`] to [`tonk_notation::Syntax`]. The
/// third lifecycle entry point: `compile`, then run the
/// operations against a caller-created transaction.
pub trait SyntaxEvaluateExt {
    /// Stage an evaluation of this document against `txn`.
    /// Returns a chain handle; call `.perform(branch, env)` to
    /// run `compile`, execute the operations, and stage the
    /// document's changes onto the transaction.
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a>;
}

impl SyntaxEvaluateExt for Syntax {
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a> {
        Evaluate { syntax: self, txn }
    }
}

/// Chain handle for an evaluation. Holds the syntax and the
/// transaction until `.perform(branch, env)` consumes them.
pub struct Evaluate<'s, 'a> {
    syntax: &'s Syntax,
    txn: Transaction<'a>,
}

impl<'s, 'a> Evaluate<'s, 'a> {
    /// Analyze the syntax, run pre-mutation queries, plan every
    /// mutation `Statement` per match frame, and apply the
    /// resulting claims to `self.txn`. The transaction is
    /// returned in [`Evaluated::txn`] with mutations baked into
    /// its overlay — caller commits, runs `induce`, or drops as
    /// they see fit.
    ///
    /// `branch` is required for analyzer introspection lookups
    /// and pre-mutation query reads — the same branch the
    /// transaction is open against. Passing it explicitly until
    /// dialog exposes a `Transaction::branch()` accessor.
    pub async fn perform<Env: EvaluateEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Evaluated<'a>, EvaluateError> {
        let Evaluate { syntax, mut txn } = self;

        // Run `compile` under the hood — `analyze` + lowering to
        // runnable operations — then walk the tree below.
        let Compiled { analysis } = syntax.compile(branch).perform(env).await?;
        let document = &analysis.analysis;

        // ---- Build base bindings frame from analysis-derived vars ----
        let mut base = Parameters::new();
        for (name, entity) in &document.variables {
            base.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
        }

        // ---- Per-expression queries + post-join ----
        let user_queries = collect_queries(document);
        let pre_results = if user_queries.is_empty() && document.synthesized.is_empty() {
            None
        } else {
            Some(run_query(&user_queries, &document.synthesized, branch, env).await?)
        };
        let pre_matches: Vec<Parameters> = match &pre_results {
            Some(r) if !r.joined.is_empty() => r.joined.clone(),
            _ => vec![Parameters::new()],
        };

        // ---- Commit-summary seed: published declarations + analysis variables ----
        let mut commits = CommitSummary::default();
        for (key, entity) in &document.declarations {
            commits.entities.insert(key.clone(), entity.to_string());
        }
        for (key, entity) in &document.variables {
            commits
                .entities
                .insert(format!("?{key}"), entity.to_string());
        }

        // ---- Plan + apply mutations to the caller's transaction ----
        let statements: Vec<Statement> = document
            .statements()
            .into_iter()
            .map(|p| p.statement)
            .collect();
        // Concept entities whose facts are transient — an `Assert`
        // against one of these seeds the effects-fixpoint bucket.
        let transient_entities = document.transient_entities();
        let mut claim_count = 0usize;
        // Retraction targets resolved by querying the branch
        // up-front so we don't interleave reads with mutation
        // accumulation against the transaction.
        let mut retract_claims: Vec<RawClaim> = Vec::new();
        // Transient-concept assertions, accumulated so the caller
        // can hand them to `induce` as the effects-fixpoint seed.
        let mut transients = Changes::new();
        if !statements.is_empty() {
            for match_frame in &pre_matches {
                let mut frame = base.clone();
                for (k, v) in match_frame.iter() {
                    frame.insert(k.clone(), v.clone());
                }
                for statement in &statements {
                    match statement {
                        Statement::Assert(application) => {
                            let plan = application
                                .clone()
                                .plan(&frame)
                                .map_err(|e| EvaluateError::Plan(format!("plan failed: {e}")))?;
                            claim_count += count_emitted_claims(&plan);
                            // A transient-concept assertion also
                            // seeds the transient bucket so the
                            // effects fixpoint fires on it and it's
                            // swept before the durable commit.
                            if transient_entities.contains(&plan.statement.predicate.this()) {
                                crate::effects::accumulate_head_facts(
                                    &plan.statement,
                                    &mut transients,
                                );
                            }
                            txn = txn.assert(plan);
                        }
                        Statement::Retract(application) => {
                            let plan = application
                                .clone()
                                .plan(&frame)
                                .map_err(|e| EvaluateError::Plan(format!("plan failed: {e}")))?;
                            let resolved = resolve_retraction_targets(plan, branch, env).await?;
                            claim_count += resolved.len();
                            retract_claims.extend(resolved);
                        }
                        Statement::InstallEffect(effect) => {
                            // A `rule!:` is a mutation: installing
                            // it writes the `dialog.effect/*` facts
                            // the reactor's induce loop reads. Each
                            // effect emits a constant set of claims
                            // (marker + source + conclusion +
                            // polarity + one `on:` entry per
                            // attribute the body reads) — bump the
                            // count by 4 + premise-attribute count.
                            claim_count += 4 + effect.on_entities().len();
                            txn = txn.assert(EffectStatement(effect.clone()));
                        }
                    }
                }
            }
            for claim in retract_claims {
                txn = txn.retract(claim);
            }
        }
        commits.claims = claim_count;

        let matches = render_match_blocks(&analysis.analysis, pre_results.as_ref());

        Ok(Evaluated {
            txn,
            transients,
            matches,
            commits,
            analysis,
        })
    }
}

/// Result of [`Evaluate::perform`].
///
/// The transaction's overlay reflects every mutation the
/// document carried; querying `txn.query()` sees the
/// post-mutation state. To run effects + commit, call
/// [`Self::commit`].
pub struct Evaluated<'a> {
    /// Transaction with mutations applied to its overlay.
    pub txn: Transaction<'a>,
    /// Transient claims the document asserted — one entry per
    /// field of every assertion whose concept is declared
    /// `transient:`. Hand to
    /// [`crate::effects::TransactionExt::induce`] as the
    /// effects-fixpoint seed; `induce` fires the installed rules
    /// on these and sweeps them so they never reach durable
    /// storage. Empty when the document asserts no transient
    /// concepts.
    pub transients: Changes,
    /// Pre-mutation per-source-expression match blocks. For
    /// post-mutation matches, re-run the analyzer's queries
    /// against `txn.query()` using the analysis carried below.
    pub matches: Vec<QueryMatchBlock>,
    /// Commit-side summary — claim count + entity bindings
    /// surfaced to response envelopes.
    pub commits: CommitSummary,
    /// The analyzer's output tree. Callers re-run its queries
    /// against the transaction overlay (or the post-commit
    /// branch) to compute post-mutation matches.
    pub analysis: Analysis<Syntax>,
}

impl<'a> Evaluated<'a> {
    /// Shortcut: run effects (via `induce`) and commit the
    /// transaction. Re-queries the branch post-commit so the
    /// returned [`EvaluateResult`] carries both `matches_before`
    /// (from `self.matches`) and `matches_after`.
    pub fn commit(self) -> EvaluatedCommit<'a> {
        EvaluatedCommit { evaluated: self }
    }
}

/// Chain handle for committing an [`Evaluated`].
pub struct EvaluatedCommit<'a> {
    evaluated: Evaluated<'a>,
}

impl<'a> EvaluatedCommit<'a> {
    /// Run `induce` against the transaction, commit, and
    /// re-query the post-commit branch state for
    /// `matches_after`.
    pub async fn perform<Env: EvaluateEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<EvaluateResult, EvaluateError> {
        use crate::effects::TransactionExt as _;

        let Evaluated {
            txn,
            transients,
            matches: matches_before,
            commits,
            analysis,
        } = self.evaluated;

        let txn = txn
            .induce(transients)
            .perform(env)
            .await
            .map_err(|e| EvaluateError::Query(format!("induce failed: {e}")))?;
        let revision = txn
            .commit()
            .perform(env)
            .await
            .map_err(|e| EvaluateError::Query(format!("commit failed: {e}")))?;

        // Post-commit re-query for matches_after. For
        // pure-mutation docs (no queries at all), the after
        // block is empty.
        let document = &analysis.analysis;
        let user_queries = collect_queries(document);
        let post_results = if user_queries.is_empty() && document.synthesized.is_empty() {
            None
        } else {
            Some(run_query(&user_queries, &document.synthesized, branch, env).await?)
        };
        let matches_after = render_match_blocks(document, post_results.as_ref());

        Ok(EvaluateResult {
            revision,
            matches_before,
            matches_after,
            commits,
            analysis,
        })
    }
}

/// Result of [`EvaluatedCommit::perform`] — the durable
/// revision plus both before/after match views and the
/// analyzer's output.
pub struct EvaluateResult {
    /// Durable revision the commit produced.
    pub revision: Revision,
    /// Pre-mutation per-source-expression matches.
    pub matches_before: Vec<QueryMatchBlock>,
    /// Post-commit per-source-expression matches — the user's
    /// view of what's now in the branch.
    pub matches_after: Vec<QueryMatchBlock>,
    /// Commit-side summary.
    pub commits: CommitSummary,
    /// The analyzer's output tree, in case the caller wants
    /// further queries against the analysis.
    pub analysis: Analysis<Syntax>,
}

/// A query application paired with its display label — the
/// per-expression unit `run_query` / `render_match_blocks` work
/// over. Projected from the tree's query nodes (and from its
/// synthesized snapshots).
#[derive(Clone)]
struct LabeledQuery {
    application: Application,
    label: String,
}

/// The user-written query expressions, in document order,
/// projected from the analysis tree.
fn collect_queries(document: &tonk_analyzer::analysis::DocumentAnalysis) -> Vec<LabeledQuery> {
    document
        .expressions
        .iter()
        .filter_map(|expression| match &expression.analysis {
            ExpressionAnalysis::Query(node) => Some(LabeledQuery {
                application: node.analysis.application.clone(),
                label: node.analysis.label.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Per-expression query results plus the joined frames for
/// mutation planning.
///
/// Each expression runs its own [`ConceptQuery`] independently;
/// the driver hash-joins frames on shared user-named variables.
/// Disjoint expressions cross-product (no shared variable to
/// constrain on); connected expressions equi-join.
struct QueryResults {
    /// Per-expression frames for the *user-written* queries, in
    /// document order. Each frame carries every user-named
    /// variable bound by that expression's query.
    per_expression: Vec<Vec<Parameters>>,
    /// The natural join of `per_expression`. Used for mutation
    /// planning (one row = one substitution into a [`Statement`]).
    /// Equivalent to the cross-product when no expressions share
    /// variables. Synthesized snapshots are deliberately excluded
    /// — a snapshot of a fresh assert target returns zero rows
    /// and would zero the join.
    joined: Vec<Parameters>,
    /// Per-expression frames for the analyzer's *synthesized*
    /// snapshot queries (Phase 4), parallel to
    /// [`DocumentAnalysis::synthesized`][tonk_analyzer::analysis::DocumentAnalysis::synthesized].
    /// Each snapshot runs standalone — never joined — and is
    /// rendered as its own match block.
    synthesized_per_expression: Vec<Vec<Parameters>>,
}

/// Run each expression's [`Application`] independently and join
/// their frames on shared user-named variables.
///
/// `Application` impls `dialog_query::Application` and dispatches
/// internally to the right [`tonk_schema::concept::QueryPlan`] (built-in
/// or branch concept), so this loop is uniform across head kinds.
async fn run_query<Env: EvaluateEnv>(
    queries: &[LabeledQuery],
    synthesized: &[SynthesizedQuery],
    branch: &Branch,
    env: &Env,
) -> Result<QueryResults, EvaluateError> {
    let mut per_expression = Vec::with_capacity(queries.len());
    for query in queries {
        let frames = collect_matches(query.application.clone(), branch, env).await?;
        per_expression.push(frames);
    }
    let joined = natural_join(&per_expression);

    // Synthesized snapshots run standalone — never joined into
    // `joined`, so an empty snapshot can't zero mutation
    // planning's binding set.
    let mut synthesized_per_expression = Vec::with_capacity(synthesized.len());
    for snapshot in synthesized {
        let frames = collect_matches(snapshot.application.clone(), branch, env).await?;
        synthesized_per_expression.push(frames);
    }

    Ok(QueryResults {
        per_expression,
        joined,
        synthesized_per_expression,
    })
}

/// Natural join of N frame relations on shared user-named
/// variables (`Parameters` keys). Disjoint frames cross-product;
/// connected frames keep only combinations that agree on shared
/// keys.
///
/// O(rows × frames-per-rel) — fine for the typical 1–10s of
/// matches per expression we see in practice.
fn natural_join(per_expression: &[Vec<Parameters>]) -> Vec<Parameters> {
    let mut acc: Vec<Parameters> = vec![Parameters::new()];
    for frames in per_expression {
        let mut next = Vec::new();
        for prefix in &acc {
            for frame in frames {
                if let Some(combined) = merge_frames(prefix, frame) {
                    next.push(combined);
                }
            }
        }
        acc = next;
        if acc.is_empty() {
            // Conjunction failed — no rows can satisfy the join.
            return Vec::new();
        }
    }
    acc
}

/// Merge two frames; return `None` when they disagree on a
/// shared key (the join row is filtered out).
fn merge_frames(a: &Parameters, b: &Parameters) -> Option<Parameters> {
    let mut combined = a.clone();
    for (k, v) in b.iter() {
        if let Some(existing) = combined.get(k) {
            if existing != v {
                return None;
            }
        } else {
            combined.insert(k.clone(), v.clone());
        }
    }
    Some(combined)
}

/// One concrete `(the, of, is)` triple ready for `tx.retract`.
/// Wraps dialog's `Statement` trait so retraction targets land
/// in the transaction the same way an `ApplicationPlan` does.
struct RawClaim {
    the: dialog_artifacts::Attribute,
    of: Entity,
    is: Value,
}

impl dialog_artifacts::Statement for RawClaim {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        update.associate(self.the, self.of, self.is);
    }
    fn retract(self, update: &mut impl dialog_artifacts::Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

/// Resolve a retraction `ApplicationPlan` to concrete
/// `(the, of, is)` triples by querying the branch.
///
/// Walks the plan's predicate. For each field whose term is
/// `Term::Variable { name: None, .. }` (a blank), runs an
/// `AttributeQuery` against `(the, this, *)` and emits one
/// `RawClaim` per match. Bound `Term::Constant` fields are
/// **not** retracted — they're treated as match anchors. Per
/// `analysis-spec.md` example 5b: `name: "Alice"` anchors,
/// `age: _` is the only field dissociated.
async fn resolve_retraction_targets<Env: EvaluateEnv>(
    plan: ApplicationPlan,
    branch: &Branch,
    env: &Env,
) -> Result<Vec<RawClaim>, EvaluateError> {
    let Some(this_term) = plan.statement.terms.get("this") else {
        return Ok(Vec::new());
    };
    let this_entity = match this_term {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    for (field_name, attribute) in plan.statement.predicate.with().iter() {
        let term = match plan.statement.terms.get(field_name) {
            Some(t) => t,
            None => continue,
        };
        // Bound constant ≠ retraction target; only blanks (and
        // bare Term::Variable, which the planner would have
        // errored on already if unbound) get dissociated.
        if !matches!(term, Term::Variable { name: None, .. }) {
            continue;
        }
        let the_term: dialog_query::attribute::The = attribute.the().clone();
        let query = dialog_query::AttributeQuery::new(
            Term::from(the_term),
            Term::from(this_entity.clone()),
            Term::<dialog_query::Any>::var("v"),
            Term::<dialog_query::attribute::Cause>::blank(),
            None,
        );
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(query)
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| {
                EvaluateError::Query(format!(
                    "retraction query failed for ({:?}, {of}): {e:?}",
                    attribute.the(),
                    of = this_entity
                ))
            })?;
        for claim in claims {
            out.push(RawClaim {
                the: claim.the.into(),
                of: this_entity.clone(),
                is: claim.is,
            });
        }
    }
    Ok(out)
}

/// Estimate how many EAVs an [`ApplicationPlan`] will emit on
/// commit — one per non-blank field. The dialog transaction API
/// doesn't expose a count after the fact, so we tally
/// per-statement here as the transaction is built.
fn count_emitted_claims(plan: &ApplicationPlan) -> usize {
    let mut n = 0;
    for (field_name, _attr) in plan.statement.predicate.with().iter() {
        if field_name == "this" {
            continue;
        }
        if let Some(term) = plan.statement.terms.get(field_name)
            && matches!(term, Term::Constant(_))
        {
            n += 1;
        }
    }
    n
}

/// Run a single expression's [`Application`] against the branch
/// and collect every match frame as a [`Parameters`] by
/// extracting every bound variable from each
/// [`ConceptConclusion`].
async fn collect_matches<Env: EvaluateEnv>(
    application: Application,
    branch: &Branch,
    env: &Env,
) -> Result<Vec<Parameters>, EvaluateError> {
    // Capture the variable names present in the application's
    // parameters so we can ask the conclusion for their bindings.
    let mut variable_names: Vec<String> = Vec::new();
    for (_, term) in application.parameters().iter() {
        if let Term::Variable {
            name: Some(name), ..
        } = term
            && !variable_names.contains(name)
        {
            variable_names.push(name.clone());
        }
    }

    let conclusions: Vec<ConceptConclusion> = branch
        .query()
        .select(application_to_plan(application))
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| EvaluateError::Query(format!("query execution failed: {e:?}")))?;

    let mut frames = Vec::with_capacity(conclusions.len());
    for conclusion in conclusions {
        let mut frame = Parameters::new();
        let source = conclusion.source();
        for name in &variable_names {
            if let Ok(value) = source.lookup(&Term::<dialog_query::Any>::var(name)) {
                frame.insert(name.clone(), Term::Constant(value));
            }
        }
        frames.push(frame);
    }

    if frames.is_empty() {
        // No matches — but still need one frame so the caller's
        // "for each frame, plan and commit" loop runs zero times
        // (rather than "one empty frame" which would commit a
        // mutation whose ?vars are all unbound).
        Ok(vec![])
    } else {
        Ok(frames)
    }
}

/// Render per-source-expression match blocks.
///
/// Each block projects from the joined frames so connected
/// queries display the filtered intersection. Within an
/// expression, frames are deduplicated by their `this` entity to
/// avoid showing the same row repeatedly when an unrelated
/// expression's cross-product introduces duplicates.
fn render_match_blocks(
    document: &tonk_analyzer::analysis::DocumentAnalysis,
    results: Option<&QueryResults>,
) -> Vec<QueryMatchBlock> {
    let Some(results) = results else {
        return Vec::new();
    };

    let user_queries = collect_queries(document);
    if user_queries.is_empty() && document.synthesized.is_empty() {
        return Vec::new();
    }

    let mut blocks = Vec::with_capacity(user_queries.len() + document.synthesized.len());

    // User-written queries: each block draws from the joined
    // frames so connected queries display the filtered
    // intersection.
    for (i, query) in user_queries.iter().enumerate() {
        // Source frames: prefer joined when non-empty (so
        // connected queries see the filter); otherwise fall back
        // to the expression's own frames so disjoint expressions
        // still surface their solo matches when another
        // expression returned zero rows and zeroed the join.
        let source_frames: &[Parameters] = if !results.joined.is_empty() {
            &results.joined
        } else {
            results
                .per_expression
                .get(i)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };
        blocks.push(render_block(
            query.label.clone(),
            &query.application,
            source_frames,
        ));
    }

    // Synthesized snapshot queries: each runs standalone (no
    // join), rendered from its own per-expression frames.
    for (i, snapshot) in document.synthesized.iter().enumerate() {
        let source_frames: &[Parameters] = results
            .synthesized_per_expression
            .get(i)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        blocks.push(render_block(
            snapshot.label.clone(),
            &snapshot.application,
            source_frames,
        ));
    }
    blocks
}

/// Build one [`QueryMatchBlock`] for `application` over
/// `source_frames`, deduplicating rows by the variables the
/// expression binds.
fn render_block(
    label: String,
    application: &Application,
    source_frames: &[Parameters],
) -> QueryMatchBlock {
    let descriptor = match application {
        Application::Concept { query: q, .. } => q.predicate.clone(),
        Application::Domain { application: d, .. } => ConceptQuery::from(d.clone()).predicate,
    };

    // Variable names this expression binds — used to dedupe rows.
    let mut my_vars: Vec<String> = Vec::new();
    let terms = match application {
        Application::Concept { query: q, .. } => &q.terms,
        Application::Domain { application: d, .. } => &d.parameters,
    };
    for (_, term) in terms.iter() {
        if let Term::Variable {
            name: Some(name), ..
        } = term
            && !my_vars.contains(name)
        {
            my_vars.push(name.clone());
        }
    }

    let mut seen = std::collections::HashSet::<Vec<String>>::new();
    let mut block_results = Vec::new();
    for frame in source_frames {
        let mut key = Vec::with_capacity(my_vars.len());
        for var in &my_vars {
            key.push(match frame.get(var) {
                Some(Term::Constant(v)) => format!("{v:?}"),
                _ => String::new(),
            });
        }
        if !seen.insert(key) {
            continue;
        }
        block_results.push(render_one_result(&descriptor, application, frame));
    }
    QueryMatchBlock {
        label,
        results: block_results,
    }
}

fn render_one_result(
    descriptor: &ConceptDescriptor,
    application: &Application,
    frame: &Parameters,
) -> QueryResult {
    let terms = match application {
        Application::Concept { query: q, .. } => &q.terms,
        Application::Domain { application: d, .. } => &d.parameters,
    };

    let this = terms
        .get("this")
        .and_then(|term| match term {
            Term::Constant(Value::Entity(e)) => Some(e.to_string()),
            Term::Variable {
                name: Some(name), ..
            } => frame.get(name).and_then(|t| match t {
                Term::Constant(Value::Entity(e)) => Some(e.to_string()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();

    let mut fields = BTreeMap::new();
    for (field_name, _attr) in descriptor.with().iter() {
        let Some(term) = terms.get(field_name) else {
            continue;
        };
        let value = match term {
            Term::Constant(value) => value_to_json(value),
            // Named variables — user `?var` and analyzer-minted
            // `__N` from `_` blanks both land here. The frame
            // binds them either way, so the same lookup works
            // for both. The auto name leaks into nothing
            // user-visible because we project under
            // `field_name`, not the variable name.
            Term::Variable {
                name: Some(name), ..
            } => match frame.get(name) {
                Some(Term::Constant(value)) => value_to_json(value),
                _ => continue,
            },
            _ => continue,
        };
        fields.insert(field_name.to_owned(), value);
    }

    QueryResult { this, fields }
}

fn value_to_json(value: &Value) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonk_schema::concept::{AnonymousConcept, TransientConcept};
    // Aliased so the `.assert()` trait method stays in scope
    // without shadowing the analyzer's `Statement` enum.
    use dialog_artifacts::Statement as ArtifactsStatement;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::{AttributeDescriptor, the};
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};
    use tonk_notation::parse;

    /// Build a 1-field cardinality-one concept descriptor —
    /// matches the helper in `effects.rs::tests`. Inlined to
    /// keep tests modules self-contained.
    fn one_text_field(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        )])
    }

    /// Install the `dialog.attribute/*` and `dialog.meta/description`
    /// facts a concept's fields need so the analyzer can rehydrate
    /// the descriptor from the branch.
    fn install_attribute_facts<'a>(
        mut txn: Transaction<'a>,
        descriptor: &ConceptDescriptor,
    ) -> Transaction<'a> {
        for (_, attr) in descriptor.with().iter() {
            let attr_entity: dialog_artifacts::Entity =
                attr.to_uri().parse().expect("attribute URI");
            txn = txn
                .assert(
                    the!("dialog.attribute/id")
                        .of(attr_entity.clone())
                        .is(format!("{}/{}", attr.domain(), attr.name())),
                )
                .assert(
                    the!("dialog.attribute/type")
                        .of(attr_entity.clone())
                        .is("Text".to_string()),
                )
                .assert(
                    the!("dialog.attribute/cardinality")
                        .of(attr_entity.clone())
                        .is("one".to_string()),
                )
                .assert(
                    the!("dialog.meta/description")
                        .of(attr_entity)
                        .is(String::new()),
                );
        }
        txn
    }

    /// Install a concept and publish it under a name so the
    /// analyzer's resolver finds it via `lookup_concept`. Uses
    /// the existing `name!` desugar through a `dialog.meta/name`
    /// claim against `id:<name>`.
    fn install_named_concept<'a>(
        txn: Transaction<'a>,
        name: &str,
        descriptor: &ConceptDescriptor,
        transient: bool,
    ) -> Transaction<'a> {
        let entity = descriptor.this();
        // Publish the name — `id:<name>` carries the
        // `dialog.meta/name` claim pointing at the concept
        // entity. This is the same shape `name!:` produces.
        let id_entity: dialog_artifacts::Entity =
            format!("id:{name}").parse().expect("id:<name> is valid");
        let mut txn = txn.assert(
            the!("dialog.name/referent")
                .of(id_entity)
                .is(entity.clone()),
        );
        if transient {
            txn = txn.assert(TransientConcept::new(descriptor.clone()));
        } else {
            txn = txn.assert(AnonymousConcept::new(descriptor.clone()));
        }
        txn
    }

    /// End-to-end: install concepts + attributes on the branch,
    /// then submit a notation document that declares a `rule!:`
    /// plus a transient `ping!:` assertion. The analyzer lifts
    /// the rule into an `Effect`; `Evaluate::perform` installs
    /// it on the branch; the reactor's `induce` loop fires it
    /// on the in-flight transient; the durable `pong` head
    /// lands.
    /// A document that queries one entity and asserts to a
    /// *different constant* entity must plan: the assert's `?var`
    /// is bound by the user query.
    ///
    /// Regression guard. Phase 4 synthesizes a snapshot query for
    /// the assert target (`id:demo-copy`), which does not exist
    /// pre-commit and returns zero rows. That snapshot must stay
    /// out of the join that feeds mutation planning — it lives in
    /// `DocumentAnalysis::synthesized`, not the query nodes — or it
    /// zeroes the join and the assert's `?var` goes unbound.
    #[dialog_common::test]
    async fn it_binds_assert_var_from_query_for_distinct_target() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Seed: id:demo's referent points at a target entity.
        let target: dialog_artifacts::Entity = "did:key:zReproTarget".parse()?;
        let id_demo: dialog_artifacts::Entity = "id:demo".parse()?;
        branch
            .transaction()
            .assert(the!("dialog.name/referent").of(id_demo).is(target.clone()))
            .commit()
            .perform(&operator)
            .await?;

        // Query id:demo's referent into ?demo, then assert it as
        // id:demo-copy's referent — a different, fresh entity.
        let doc = "\
name:\n\
\x20 this: id:demo\n\
\x20 entity: ?demo\n\
\n\
name!:\n\
\x20 this: id:demo-copy\n\
\x20 entity: ?demo\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate: {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit: {e}"))?;

        // id:demo-copy now carries the copied referent.
        let id_copy: dialog_artifacts::Entity = "id:demo-copy".parse()?;
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.name/referent"))
                    .of(Term::<dialog_artifacts::Entity>::from(id_copy))
                    .is(Term::<dialog_artifacts::Entity>::var("referent")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(claims.len(), 1, "expected the copied referent claim");
        assert_eq!(claims[0].is, Value::Entity(target));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_installs_and_fires_a_notation_rule() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let ping = one_text_field("io.gozala.ping", "tag");
        let pong = one_text_field("io.gozala.pong", "tag");

        // Pre-install concepts + the transient marker on ping.
        // (Concept-declaration notation doesn't yet take a
        // `transient: true` flag; that's a separate parser
        // change. Pre-installing keeps this test focused on
        // the rule-lift path.)
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &ping);
        install = install_attribute_facts(install, &pong);
        install = install_named_concept(install, "ping", &ping, /*transient=*/ true);
        install = install_named_concept(install, "pong", &pong, /*transient=*/ false);
        install.commit().perform(&operator).await?;

        // Now run the notation document through the full chain.
        // The rule!: lifts into an Effect, lands on the branch,
        // and fires on the inline ping!: transient assertion.
        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        // First commit: install the rule.
        let evaluated = syntax
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install rule): {e}"))?;
        evaluated
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install rule): {e}"))?;

        // Sanity: the rule's dialog.effect/source claim landed.
        let effect_source_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.effect/source"))
                    .of(Term::<dialog_artifacts::Entity>::var("effect"))
                    .is(Term::<String>::var("source")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            effect_source_claims.len(),
            1,
            "expected exactly one installed effect; saw {effect_source_claims:?}"
        );

        // Second commit: submit a transient ping{this: e1, tag: "hi"}
        // through the raw Changes path. The reactor's induce
        // loop should fire the installed rule and land the
        // durable pong head.
        let subject: dialog_artifacts::Entity = "did:key:zNotationSubject".parse()?;
        let ping_tag = the!("io.gozala.ping/tag");
        let mut transients = Changes::new();
        ping_tag
            .clone()
            .of(subject.clone())
            .is("hi".to_string())
            .assert(&mut transients);

        use crate::effects::TransactionExt as _;
        branch
            .transaction()
            .integrate(transients.clone())
            .induce(transients)
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("induce: {e}"))?
            .commit()
            .perform(&operator)
            .await?;

        // Durable pong claim should be present.
        let pong_tag = the!("io.gozala.pong/tag");
        let pong_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(pong_tag)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            pong_claims.len(),
            1,
            "expected one durable pong claim; saw {pong_claims:?}"
        );

        // Ping transient should not have survived.
        let ping_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(ping_tag)
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            ping_claims.is_empty(),
            "transient ping should have been swept; saw {ping_claims:?}"
        );

        Ok(())
    }

    /// Pure-notation flow: a single document declares the
    /// transient `ping` and durable `pong` concepts (using
    /// `concept!: …, transient: true`), installs a `rule!:
    /// assert!: pong when ping`, and the second commit submits
    /// the transient through raw Changes (notation doesn't yet
    /// have a sugar for asserting a transient instance via the
    /// /transact wire path, but the durable-claim shape works
    /// directly).
    ///
    /// Verifies the `transient:` field on `concept!:` flows all
    /// the way to the `dialog.concept/transient` marker fact —
    /// without it, the rule would fail validate (no transient
    /// trigger) and the body wouldn't classify head emissions
    /// correctly.
    #[dialog_common::test]
    async fn it_declares_transient_concept_via_notation() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // One commit: declare two concepts (one transient, one
        // durable) + the rule. Concept!: declarations carry
        // their own inline attribute definitions, so the
        // attributes get registered alongside.
        let doc = "\
concept!: &ping\n\
\x20 transient:\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.ping/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n\
\n\
concept!: &pong\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.pong/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n\
\n\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        // Sanity: the transient marker landed on ping's
        // descriptor entity.
        let ping_descriptor = one_text_field("io.gozala.ping", "tag");
        let ping_entity = ping_descriptor.this();
        let marker_target: dialog_artifacts::Entity =
            "db:transient".parse().expect("db:transient is valid");
        let marker_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.concept/transient"))
                    .of(Term::from(ping_entity.clone()))
                    .is(Term::from(marker_target.clone())),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            marker_claims.len(),
            1,
            "expected one dialog.concept/transient claim on ping; saw {marker_claims:?}"
        );

        // And the durable pong concept has no marker.
        let pong_descriptor = one_text_field("io.gozala.pong", "tag");
        let pong_entity = pong_descriptor.this();
        let pong_marker_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.concept/transient"))
                    .of(Term::from(pong_entity))
                    .is(Term::from(marker_target)),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            pong_marker_claims.is_empty(),
            "durable pong should have no transient marker; saw {pong_marker_claims:?}"
        );

        // Second commit: submit a transient ping and verify
        // the rule fires through induce.
        let subject: dialog_artifacts::Entity = "did:key:zPureNotationSubject".parse()?;
        let ping_tag = the!("io.gozala.ping/tag");
        let mut transients = Changes::new();
        ping_tag
            .clone()
            .of(subject.clone())
            .is("hi".to_string())
            .assert(&mut transients);

        use crate::effects::TransactionExt as _;
        branch
            .transaction()
            .integrate(transients.clone())
            .induce(transients)
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("induce: {e}"))?
            .commit()
            .perform(&operator)
            .await?;

        let pong_tag = the!("io.gozala.pong/tag");
        let pong_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(pong_tag)
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            pong_claims.len(),
            1,
            "expected one durable pong claim; saw {pong_claims:?}"
        );

        let ping_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(ping_tag)
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            ping_claims.is_empty(),
            "transient ping should have been swept; saw {ping_claims:?}"
        );

        Ok(())
    }

    /// End-to-end counter repro: a transient `increment` command
    /// drives a `rule!:` that sums via `math/sum` into a durable
    /// `counter`. Both the `count: 0` and `by: 1` literals land in
    /// `unsigned-integer` fields. The notation parser always emits
    /// a signed `Scalar::Integer` for a non-negative literal; the
    /// analyzer's schema-directed coercion produces `UnsignedInt`
    /// terms instead, so `math/sum` (unsigned-only) doesn't fail
    /// induction with `TypeMismatch { expected: UnsignedInt,
    /// actual: SignedInt }`. The counter's durable count must
    /// update to `1`.
    #[dialog_common::test]
    async fn it_induces_unsigned_sum_from_transient_increment() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // First commit: declare the concepts + the summing rule,
        // and seed the durable counter at 0.
        let setup = "\
concept!: &counter\n\
\x20 with:\n\
\x20   count:\n\
\x20     the: xyz.tonk.counter/count\n\
\x20     as: unsigned-integer\n\
\x20     cardinality: one\n\
\x20     description: \"count\"\n\
\n\
concept!: &increment\n\
\x20 transient:\n\
\x20 with:\n\
\x20   by:\n\
\x20     the: xyz.tonk.command/increment\n\
\x20     as: unsigned-integer\n\
\x20     cardinality: one\n\
\x20     description: \"by\"\n\
\n\
rule!:\n\
\x20 assert!: counter\n\
\x20 when:\n\
\x20   - assert: increment\n\
\x20     where: { this: ?this, by: ?n }\n\
\x20   - assert: counter\n\
\x20     where: { this: ?this, count: ?m }\n\
\x20   - assert: math/sum\n\
\x20     where: { of: ?n, with: ?m, is: ?count }\n\
\n\
counter!: &counter-demo\n\
\x20 count: 0\n";
        let parsed = parse(setup);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        parsed
            .syntax
            .expect("setup syntax")
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (setup): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (setup): {e}"))?;

        // The seeded counter holds an unsigned 0.
        let count_attr = the!("xyz.tonk.counter/count");
        let counter_demo: dialog_artifacts::Entity =
            "id:counter-demo".parse().expect("id:<name> entity");
        // Resolve the named counter entity via its referent claim.
        let referent: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.name/referent"))
                    .of(Term::from(counter_demo))
                    .is(Term::<dialog_artifacts::Entity>::var("e")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(referent.len(), 1, "counter-demo referent should resolve");
        let counter_entity = match &referent[0].is {
            Value::Entity(e) => e.clone(),
            other => panic!("referent should be an entity, got {other:?}"),
        };

        let count_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(count_attr.clone())
                    .of(Term::from(counter_entity.clone()))
                    .is(Term::<u128>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(count_claims.len(), 1, "expected the seeded count claim");
        assert_eq!(count_claims[0].is, Value::UnsignedInt(0));

        // Second commit: submit a transient increment{by: 1} for
        // the counter entity. The reactor's induce loop fires the
        // rule, `math/sum` runs on unsigned operands, and the
        // durable counter count updates to 1.
        let by_attr = the!("xyz.tonk.command/increment");
        let mut transients = Changes::new();
        by_attr
            .of(counter_entity.clone())
            .is(1u128)
            .assert(&mut transients);

        use crate::effects::TransactionExt as _;
        branch
            .transaction()
            .integrate(transients.clone())
            .induce(transients)
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("induce: {e}"))?
            .commit()
            .perform(&operator)
            .await?;

        let updated: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(count_attr)
                    .of(Term::from(counter_entity))
                    .is(Term::<u128>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            updated.iter().any(|c| c.is == Value::UnsignedInt(1)),
            "counter count should update to 1; saw {updated:?}"
        );

        Ok(())
    }

    /// Regression: a document containing only a `rule!:` is a
    /// mutation document — the `!` marker says so. The lifted
    /// effect must land in `mutate.statements` as a
    /// `Statement::InstallEffect`, so the `/evaluate` route's
    /// commit guard (`!mutate.statements.is_empty()`) sees it and
    /// drives the commit. Before this, rules lived in a parallel
    /// `analysis.effects` bucket and rule-only documents were
    /// silently dropped — the rule never reached the branch.
    #[dialog_common::test]
    async fn it_lifts_an_effect_from_a_rule_only_document() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit the concepts the rule references.
        let concepts = "\
concept!: &ping\n\
\x20 transient:\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.ping/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n\
\n\
concept!: &pong\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.pong/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n";
        parse(concepts)
            .syntax
            .expect("concepts syntax")
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (concepts): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (concepts): {e}"))?;

        // A document that is *only* a rule.
        let rule_doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let evaluated = parse(rule_doc)
            .syntax
            .expect("rule syntax")
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (rule): {e}"))?;

        // The lifted rule must lower to one statement — a
        // `Statement::InstallEffect` — so the route's commit
        // guard (`has_statements`) sees it.
        let statements = evaluated.analysis.analysis.statements();
        assert_eq!(
            statements.len(),
            1,
            "rule-only document should carry one mutation statement"
        );
        let Statement::InstallEffect(effect) = &statements[0].statement else {
            panic!(
                "rule-only document should carry a Statement::InstallEffect, got {:?}",
                statements[0].statement
            );
        };
        assert_eq!(
            effect.conclusion(),
            one_text_field("io.gozala.pong", "tag").this(),
            "the installed effect's head concept should be pong"
        );

        Ok(())
    }

    /// Repro mirroring the user's report: transient `person-entered`
    /// (two fields, one numeric), a durable `person` concept, a
    /// rule person <- person-entered, then a notation instance
    /// `person-entered!:`. Expect a durable `person` to appear.
    #[dialog_common::test]
    async fn it_fires_a_rule_on_a_two_field_notation_transient() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit each document separately, as a user would across
        // editor cells / sessions: concepts first, then the rule,
        // then the instance.
        let commit_doc = async |doc: &str, label: &str| -> anyhow::Result<()> {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "{label} parse diagnostics: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&branch, &operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate ({label}): {e}"))?
                .commit()
                .perform(&branch, &operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit ({label}): {e}"))?;
            Ok(())
        };

        // Commit 1: the transient concept + the durable concept.
        commit_doc(
            "\
concept!: &person-entered\n\
\x20 transient:\n\
\x20 with:\n\
\x20   name:\n\
\x20     the: xyz.tonk.env/name\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"name\"\n\
\x20   age:\n\
\x20     the: xyz.tonk.env/age\n\
\x20     as: unsigned-integer\n\
\x20     cardinality: one\n\
\x20     description: \"age\"\n\
\n\
attribute!: &person-name\n\
\x20 description: The person's name\n\
\x20 the: xyz.tonk.person/name\n\
\x20 as: text\n\
\x20 cardinality: one\n\
\n\
attribute!: &person-age\n\
\x20 description: The person's age\n\
\x20 the: xyz.tonk.person/age\n\
\x20 as: unsigned-integer\n\
\x20 cardinality: one\n\
\n\
concept!: &person\n\
\x20 description: \"A person\"\n\
\x20 with:\n\
\x20   name: person-name\n\
\x20   age: person-age\n",
            "concepts",
        )
        .await?;

        // Commit 2: the rule — a separate document.
        commit_doc(
            "\
rule!:\n\
\x20 assert!: person\n\
\x20 when:\n\
\x20   - assert: person-entered\n\
\x20     where: { this: ?this, name: ?name, age: ?age }\n",
            "rule",
        )
        .await?;

        let parsed = parse(
            "person-entered!:\n  this: did:key:zPersonEnteredSubject\n  name: \"Tester Joe\"\n  age: 42\n",
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "instance parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let instance = parsed.syntax.expect("instance syntax");

        // Re-open the branch from storage before the instance
        // commit — the rule must be *loaded* from the branch's
        // dialog.effect/* facts, not held in memory. This is what
        // a separate /evaluate request does.
        let branch = repo.branch("main").open().perform(&operator).await?;

        instance
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (instance): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (instance): {e}"))?;

        let branch = repo.branch("main").open().perform(&operator).await?;

        // A durable person.name claim should exist.
        let person_name: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.person/name"))
                    .of(Term::<dialog_artifacts::Entity>::var("p"))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            person_name.len(),
            1,
            "rule should have produced a durable person; saw {person_name:?}"
        );

        Ok(())
    }

    /// Repro: a transient concept *instance* asserted through
    /// notation (`ping!:`) — not raw `Changes` — must seed the
    /// effects fixpoint so the installed rule fires and the
    /// durable head lands.
    #[dialog_common::test]
    async fn it_fires_a_rule_on_a_notation_transient_instance() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let install = "\
concept!: &ping\n\
\x20 transient:\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.ping/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n\
\n\
concept!: &pong\n\
\x20 with:\n\
\x20   tag:\n\
\x20     the: io.gozala.pong/tag\n\
\x20     as: text\n\
\x20     cardinality: one\n\
\x20     description: \"tag\"\n\
\n\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let syntax = parse(install).syntax.expect("install syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        // Assert a transient `ping` instance via notation — the
        // path the worker's /evaluate route runs.
        let parsed = parse("ping!:\n  this: did:key:zNotationTransientSubject\n  tag: \"hi\"\n");
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let instance = parsed.syntax.expect("instance syntax");
        instance
            .evaluate(branch.transaction())
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (instance): {e}"))?
            .commit()
            .perform(&branch, &operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (instance): {e}"))?;

        let subject: dialog_artifacts::Entity = "did:key:zNotationTransientSubject".parse()?;

        let pong_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("io.gozala.pong/tag"))
                    .of(Term::from(subject.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            pong_claims.len(),
            1,
            "rule should have produced a durable pong; saw {pong_claims:?}"
        );

        let ping_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("io.gozala.ping/tag"))
                    .of(Term::from(subject))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            ping_claims.is_empty(),
            "transient ping should have been swept; saw {ping_claims:?}"
        );

        Ok(())
    }

    /// `syntax.analyze(source).perform(env)` standalone — the
    /// first lifecycle entry point used on its own, with no
    /// `compile` or `evaluate` step. Installs a concept on the
    /// branch, then resolves a query document against the
    /// committed branch as the [`Source`]. The chain yields the
    /// document's [`Analysis`] directly.
    #[dialog_common::test]
    async fn it_analyzes_a_document_via_the_syntax_chain() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Install a `person` concept so the query head resolves
        // against the branch source.
        let person = one_text_field("io.gozala.person", "name");
        let mut install = branch.transaction();
        install = install_attribute_facts(install, &person);
        install = install_named_concept(install, "person", &person, /*transient=*/ false);
        install.commit().perform(&operator).await?;

        let parsed = parse("person:\n  this: ?alice\n  name: \"Alice\"\n");
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        // `analyze` alone — pure-read, takes the branch as the
        // source, yields the Analysis with no commit.
        let analysis = syntax
            .analyze(&branch)
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("analyze: {e}"))?;

        // A pure-query document: one query expression, no
        // lowered statements.
        let document = &analysis.analysis;
        let queries: Vec<_> = document.queries().collect();
        assert_eq!(queries.len(), 1, "expected one resolved query");
        assert!(
            document.statements().is_empty(),
            "pure-query document has no mutation statements"
        );
        let bindings = queries[0].analysis.application.bindings();
        assert!(
            bindings.contains("alice"),
            "the ?alice variable should be bound by the query"
        );
        Ok(())
    }
}
