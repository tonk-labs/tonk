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
//! syntax.evaluate(branch.transaction()).perform(env).await?; // -> Evaluated
//! ```
//!
//! - [`SyntaxAnalyzeExt::analyze`] is pure-read — takes a
//!   [`Source`] (a `&Branch` or `&Transaction`, anything
//!   `Into<Source>`) and yields an [`Analysis`].
//! - [`SyntaxCompileExt::compile`] runs `analyze` under the hood
//!   and yields a [`Compiled`] — a thin handle over the resolved
//!   document's runnable operations.
//! - [`SyntaxEvaluateExt::evaluate`] takes a `&Branch`, opens a
//!   transaction internally, runs `compile` under the hood, runs
//!   the operations, and yields an [`Evaluated`] holding the
//!   transaction with the document's changes staged. It does
//!   *not* commit.
//!
//! ```ignore
//! let evaluated = syntax
//!     .evaluate(branch.transaction())
//!     .perform(env).await?;
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
//!     .perform(env).await?
//!     .commit()
//!     .perform(env).await?;
//! // result.revision, result.matches_after, ...
//! ```
//!
//! Pre-existing helpers (per-expression query, natural join, match
//! rendering) stay inside this module — they're called both during
//! `Evaluate::perform` (for `matches_before`) and by
//! [`EvaluatedCommit::perform`] (for `matches_after` after commit).

use std::collections::BTreeMap;

use dialog_artifacts::{Entity, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_query::attribute::Relation;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::{ConceptDescriptor, ConceptQuery, Output as _, Parameters, Term};
use dialog_repository::{RemoteSite, Transaction};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_notation::Syntax;

use tonk_analyzer::analysis::{Analysis, ExpressionAnalysis, SynthesizedQuery};
use tonk_analyzer::analyzer;
use tonk_schema::concept::{QueryEnv, application_to_plan};
use tonk_schema::query_source::Source;
use tonk_schema::transact::{Application, ApplicationPlan, Planner as _, Statement};

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
//   syntax.evaluate(branch.transaction()).perform(env) -> Evaluated              //
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
        analyzer::analyze(syntax, source.into())
            .perform(env)
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
/// operations against a caller-provided transaction.
pub trait SyntaxEvaluateExt {
    /// Stage an evaluation of this document against `txn`.
    /// Returns a chain handle; call `.perform(env)` to run
    /// `compile`, execute the operations (including effect
    /// induction), and yield the transaction with the writes
    /// baked into its overlay. The chain does *not* commit —
    /// the caller decides whether to commit, drop, or compose
    /// further on `Evaluated::txn`.
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a>;
}

impl SyntaxEvaluateExt for Syntax {
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a> {
        Evaluate { syntax: self, txn }
    }
}

/// Chain handle for an evaluation. Holds the syntax and the
/// transaction until `.perform(env)` consumes them.
pub struct Evaluate<'s, 'a> {
    syntax: &'s Syntax,
    txn: Transaction<'a>,
}

impl<'s, 'a> Evaluate<'s, 'a> {
    /// Analyze the syntax, run pre-mutation queries, plan every
    /// mutation `Statement` per match frame, apply the resulting
    /// claims, and run effect induction. The transaction is
    /// returned in [`Evaluated::txn`] with every mutation baked
    /// into its overlay — including the rules' induced heads and
    /// the transient-sweep retracts. The caller decides whether
    /// to commit, drop, or query `evaluated.txn` further.
    ///
    /// All reads (resolver lookups, pre-mutation match queries,
    /// retract-target resolution, rule-retract source resolution)
    /// go through the transaction's overlay so the chain never
    /// needs a `&Branch`.
    pub async fn perform<Env: EvaluateEnv>(
        self,
        env: &Env,
    ) -> Result<Evaluated<'a>, EvaluateError> {
        let Evaluate { syntax, mut txn } = self;

        // Run `compile` under the hood. Resolution reads through
        // the txn overlay; pre-mutation overlay is empty so this
        // sees branch state.
        let Compiled { analysis } = syntax.compile(&txn).perform(env).await?;
        let document = &analysis.analysis;

        // ---- Build base bindings frame from analysis-derived vars ----
        let mut base = Parameters::new();
        for (name, entity) in &document.variables {
            base.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
        }

        // ---- Per-expression queries + post-join ----
        // Pre-mutation reads go through the txn's overlay, which
        // is empty at this point so the answer matches the branch.
        let user_queries = collect_queries(document);
        let pre_results = if user_queries.is_empty() && document.synthesized.is_empty() {
            None
        } else {
            Some(run_query(&user_queries, &document.synthesized, &txn, env).await?)
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
        // against one of these is dispatched as a command (visible to
        // reads and commit-time induction, never committed).
        let transient_entities = document.transient_entities();
        let mut claim_count = 0usize;
        // Retraction targets resolved by querying the branch
        // up-front so we don't interleave reads with mutation
        // accumulation against the transaction.
        let mut retract_claims: Vec<RawClaim> = Vec::new();
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
                            // A transient-concept assertion is a
                            // command: dispatched into the
                            // transaction so commit-time induction
                            // fires on it, never committed.
                            let mut dispatch = false;
                            if let ApplicationPlan::Concept(concept_plan) = &plan {
                                claim_count += count_emitted_claims(concept_plan);
                                dispatch = transient_entities
                                    .contains(&concept_plan.statement.predicate.this());
                            } else if let ApplicationPlan::Rule(rule) = &plan {
                                // A rule install writes the native
                                // `dialog.rule/*` set: source +
                                // induces + one `on:` entry per body
                                // attribute.
                                claim_count += 2 + tonk_schema::rule::on_entities(rule).len();
                            } else if let ApplicationPlan::DeductiveRule(rule) = &plan {
                                // A deductive rule install writes
                                // source + conclusion + one `reads`
                                // entry per body attribute.
                                claim_count += 2 + tonk_schema::rule::reads_entities(rule).len();
                            }
                            if dispatch {
                                txn = txn.dispatch(plan);
                            } else {
                                txn = txn.assert(plan);
                            }
                        }
                        Statement::Retract(application) => {
                            let plan = application
                                .clone()
                                .plan(&frame)
                                .map_err(|e| EvaluateError::Plan(format!("plan failed: {e}")))?;
                            match plan {
                                ApplicationPlan::Concept(concept_plan) => {
                                    let resolved =
                                        resolve_retraction_targets(*concept_plan, &txn, env)
                                            .await?;
                                    claim_count += resolved.len();
                                    retract_claims.extend(resolved);
                                }
                                ApplicationPlan::Rule(rule) => {
                                    // The rule was resolved off the
                                    // branch in the analyzer, and the
                                    // canonical encoding makes the
                                    // dissociate byte-exact.
                                    claim_count += 2 + tonk_schema::rule::on_entities(&rule).len();
                                    txn = txn.retract(*rule);
                                }
                                ApplicationPlan::DeductiveRule(rule) => {
                                    claim_count +=
                                        2 + tonk_schema::rule::reads_entities(&rule).len();
                                    txn = txn.retract(*rule);
                                }
                            }
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

        // Rule induction is part of COMMIT now: dialog's
        // `TransactionCommit::perform` fires installed rules over the
        // transaction's delta (durable changes and dispatched
        // transients alike) and folds their durable novelty into the
        // commit, while transients are dropped, never written. The
        // caller's txn is returned with the dispatch staged; they
        // commit or drop.
        Ok(Evaluated {
            txn,
            matches,
            commits,
            analysis,
        })
    }
}

/// Result of [`Evaluate::perform`].
///
/// The transaction's overlay reflects every mutation the
/// document carried — user-written writes, rule-induced heads,
/// transient sweeps. Caller chooses to commit (`txn.commit()`),
/// drop (rollback), or query (`txn.query()`) further. Post-
/// mutation matches are computed by the caller from the txn;
/// the chain itself never commits and never re-queries.
pub struct Evaluated<'a> {
    /// Transaction with the document's mutations + induction
    /// applied to its overlay. Caller drives commit / drop /
    /// further composition.
    pub txn: Transaction<'a>,
    /// Pre-mutation per-source-expression match blocks. For
    /// post-mutation matches, call [`Self::matches_after`].
    pub matches: Vec<QueryMatchBlock>,
    /// Commit-side summary — claim count + entity bindings
    /// surfaced to response envelopes.
    pub commits: CommitSummary,
    /// The analyzer's output tree. Callers re-run its queries
    /// against the transaction overlay to compute post-mutation
    /// matches.
    pub analysis: Analysis<Syntax>,
}

impl<'a> Evaluated<'a> {
    /// Post-mutation per-source-expression match blocks. Runs
    /// the analyzer's queries against the transaction overlay,
    /// which already reflects every applied write plus the
    /// induce pass, so this returns the same view a post-commit
    /// branch query would — without needing to commit first.
    ///
    /// Pure-mutation documents (no queries at all) get an empty
    /// vec.
    pub async fn matches_after<Env: EvaluateEnv>(
        &self,
        env: &Env,
    ) -> Result<Vec<QueryMatchBlock>, EvaluateError> {
        let document = &self.analysis.analysis;
        let user_queries = collect_queries(document);
        let post_results = if user_queries.is_empty() && document.synthesized.is_empty() {
            None
        } else {
            Some(run_query(&user_queries, &document.synthesized, &self.txn, env).await?)
        };
        Ok(render_match_blocks(document, post_results.as_ref()))
    }

    /// Convenience: hand the underlying transaction to dialog's
    /// commit chain. Same as `self.txn.commit()` — exposed on
    /// `Evaluated` so the common `.evaluate(...).perform(...)?
    /// .commit().perform(...)` chain composes without an
    /// intermediate destructure. The chain itself never commits;
    /// callers who want to commit call this (or drive
    /// `evaluated.txn.commit()` directly).
    pub fn commit(self) -> dialog_repository::TransactionCommit<'a> {
        self.txn.commit()
    }
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
    txn: &Transaction<'_>,
    env: &Env,
) -> Result<QueryResults, EvaluateError> {
    let mut per_expression = Vec::with_capacity(queries.len());
    for query in queries {
        let frames = collect_matches(query.application.clone(), txn, env).await?;
        per_expression.push(frames);
    }
    let joined = natural_join(&per_expression);

    // Synthesized snapshots run standalone — never joined into
    // `joined`, so an empty snapshot can't zero mutation
    // planning's binding set.
    let mut synthesized_per_expression = Vec::with_capacity(synthesized.len());
    for snapshot in synthesized {
        let frames = collect_matches(snapshot.application.clone(), txn, env).await?;
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

/// Resolve a concept-shaped retraction plan to concrete
/// `(the, of, is)` triples by querying the branch.
///
/// Walks the plan's predicate. For each field whose term is
/// `Term::Variable { name: None, .. }` (a blank), runs an
/// `AttributeQuery` against `(the, this, *)` and emits one
/// `RawClaim` per match. Bound `Term::Constant` fields are
/// **not** retracted — they're treated as match anchors. Per
/// `analysis-spec.md` example 5b: `name: "Alice"` anchors,
/// `age: _` is the only field dissociated.
///
/// Rule retracts use a different path — the analyzer already
/// resolves the byte-exact stored source via `Rule::retracting`,
/// so this function only handles `ApplicationPlan::Concept`.
async fn resolve_retraction_targets<Env: EvaluateEnv>(
    plan: tonk_schema::transact::ConceptPlan,
    txn: &Transaction<'_>,
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
        // A collection entry's attribute is `domain/key`, the key
        // being the literal the assertion named; without one there
        // is no single fact to retract.
        let the = match attribute.the().attribute() {
            Some(the) => the,
            None => {
                let key = plan.statement.terms.get(&Relation::key_operand(field_name));
                let Some(Term::Constant(Value::String(key))) = key else {
                    continue;
                };
                attribute.the().entry(key).map_err(|e| {
                    EvaluateError::Query(format!("retraction target for {field_name}: {e}"))
                })?
            }
        };
        let query = dialog_query::AttributeQuery::new(
            Term::Constant(Value::Symbol(the)),
            Term::from(this_entity.clone()),
            Term::<dialog_query::Any>::var("v"),
            Term::<dialog_query::attribute::Cause>::blank(),
            None,
        );
        let claims: Vec<dialog_query::Claim> = txn
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

/// Estimate how many EAVs a concept-shaped plan will emit on
/// commit — one per non-blank field. The dialog transaction API
/// doesn't expose a count after the fact, so we tally
/// per-statement here as the transaction is built.
///
/// Rule installs / retracts have a different count (4 + body
/// premises) computed separately in the call sites.
fn count_emitted_claims(plan: &tonk_schema::transact::ConceptPlan) -> usize {
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
    txn: &Transaction<'_>,
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

    let conclusions: Vec<ConceptConclusion> = txn
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
            // `lookup` yields a `Binding`; an optional field that
            // the entity lacks resolves to `Absent` and is simply
            // omitted from the frame.
            if let Ok(binding) = source.lookup(&Term::<dialog_query::Any>::var(name))
                && let Some(value) = binding.as_value()
            {
                frame.insert(name.clone(), Term::Constant(value.clone()));
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
    // A resolver has no concept descriptor — its shape is its own
    // operand slots — so it renders through its own path rather than
    // borrowing a descriptor's field list.
    if let Application::Resolver { query, terms } = application {
        return render_resolver_block(label, query, terms, source_frames);
    }

    let descriptor = match application {
        Application::Concept { query: q, .. } => q.predicate.clone(),
        Application::Domain { application: d, .. } => ConceptQuery::from(d.clone()).predicate,
        Application::Rule { .. } | Application::DeductiveRule { .. } => {
            // Rules never appear as a query expression — they are
            // write-only via Statement::Assert/Retract — so the
            // renderer's per-expression block path doesn't reach
            // here for rules.
            return QueryMatchBlock {
                label,
                results: Vec::new(),
            };
        }
        Application::Resolver { .. } => unreachable!("handled above"),
    };

    // Variable names this expression binds — used to dedupe rows.
    let mut my_vars: Vec<String> = Vec::new();
    let terms = match application {
        Application::Concept { query: q, .. } => &q.terms,
        Application::Domain { application: d, .. } => &d.parameters,
        Application::Rule { .. }
        | Application::DeductiveRule { .. }
        | Application::Resolver { .. } => {
            unreachable!("filtered above")
        }
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
        Application::Rule { .. }
        | Application::DeductiveRule { .. }
        | Application::Resolver { .. } => {
            unreachable!("render_one_result is not called for rule or resolver applications")
        }
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

/// Render a resolver expression's rows.
///
/// A resolver row is slot→value with no entity of its own, so `this`
/// stays empty and every bound operand becomes a field. Rows dedupe on
/// the same variables the concept path uses, so a resolver joined into
/// a document behaves like any other expression.
fn render_resolver_block(
    label: String,
    query: &dialog_query::ResolverQuery,
    terms: &Parameters,
    source_frames: &[Parameters],
) -> QueryMatchBlock {
    let _ = query;
    let mut my_vars: Vec<String> = Vec::new();
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
    let mut results = Vec::new();
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

        let mut fields = BTreeMap::new();
        for (slot, term) in terms.iter() {
            let value = match term {
                Term::Constant(value) => value_to_json(value),
                Term::Variable {
                    name: Some(name), ..
                } => match frame.get(name) {
                    Some(Term::Constant(value)) => value_to_json(value),
                    _ => continue,
                },
                _ => continue,
            };
            fields.insert(slot.to_owned(), value);
        }
        results.push(QueryResult {
            this: String::new(),
            fields,
        });
    }

    QueryMatchBlock { label, results }
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
    // The crate's wasm tests must run in a browser: they drive dialog
    // storage through web APIs Node.js does not provide. This lived in
    // `effect_query.rs` until the native-rules migration deleted it, and
    // its absence only surfaces on the web leg ("failed to find or
    // execute Node.js"), never in a native run.
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use dialog_artifacts::Changes;
    use dialog_artifacts::Statement as ArtifactsStatement;
    use dialog_operator::helpers::{test_operator_with_profile, test_repo};
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::{AttributeDescriptor, the};
    use tonk_notation::parse;

    /// Build a 1-field cardinality-one concept descriptor —
    /// matches the helper in `effects.rs::tests`. Inlined to
    /// keep tests modules self-contained.
    fn one_text_field(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::try_from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        )])
        .unwrap()
    }

    /// Install the `db.attribute/*` and `db.meta/description`
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
                    the!("db.attribute/id")
                        .of(attr_entity.clone())
                        .is(attr.the().to_string()),
                )
                .assert(
                    the!("db.attribute/type")
                        .of(attr_entity.clone())
                        .is("Text".to_string()),
                )
                .assert(
                    the!("db.attribute/cardinality")
                        .of(attr_entity.clone())
                        .is("one".to_string()),
                )
                .assert(
                    the!("db.meta/description")
                        .of(attr_entity)
                        .is(String::new()),
                );
        }
        txn
    }

    /// Install a concept and publish it under a name so the
    /// analyzer's resolver finds it via `lookup_concept`. Uses
    /// the existing `name!` desugar through a `db.meta/name`
    /// claim against `id:<name>`.
    fn install_named_concept<'a>(
        txn: Transaction<'a>,
        name: &str,
        descriptor: &ConceptDescriptor,
        transient: bool,
    ) -> Transaction<'a> {
        let entity = descriptor.this();
        // Publish the name — `id:<name>` carries the
        // `db.meta/name` claim pointing at the concept
        // entity. This is the same shape `name!:` produces.
        let id_entity: dialog_artifacts::Entity =
            format!("id:{name}").parse().expect("id:<name> is valid");
        let mut txn = txn.assert(the!("db.name/referent").of(id_entity).is(entity.clone()));
        if transient {
            txn = txn.assert(TransientConcept::new(descriptor.clone()));
        } else {
            txn = txn.assert(AnonymousConcept::new(descriptor.clone()));
        }
        txn
    }

    /// Repro for BUG-maybe-name-resolution: a `maybe:` map must
    /// resolve attribute references exactly like `with:` does —
    /// both by published name and by `id:` URI. The attributes are
    /// published in an *earlier commit*, so resolution must go
    /// through the branch tables, not document-local scope.
    #[dialog_common::test]
    async fn it_resolves_branch_attribute_references_in_maybe_blocks() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let docs = [
            // Commit 1: publish two attributes on the branch.
            r#"attribute!: &foo/bar
  description: "an attr"
  the: io.foo/bar
  as: text
  cardinality: one

attribute!: &foo/title
  description: "title"
  the: io.foo/title
  as: text
  cardinality: one
"#,
            // Commit 2: reference foo/bar by NAME under `maybe:`.
            r#"concept!: &by-name
  description: "x"
  with:
    title: foo/title
  maybe:
    bar: foo/bar
"#,
            // Commit 3: reference foo/bar by `id:` URI under `maybe:`.
            r#"concept!: &by-uri
  description: "x"
  with:
    title: foo/title
  maybe:
    bar: id:foo/bar
"#,
        ];
        for doc in docs {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "parse diagnostics for {doc:?}: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate failed for {doc:?}: {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit failed for {doc:?}: {e}"))?;
        }
        Ok(())
    }

    /// Repro for BUG-domain-head-cardinality-many: a
    /// `cardinality: many` attribute must accumulate values when
    /// asserted through a domain head (`repro.demo!:`), exactly as
    /// it does through a concept head. The domain head used to
    /// synthesize a cardinality-one descriptor, so each write
    /// replaced the prior value (last-write-wins).
    #[dialog_common::test]
    async fn it_accumulates_many_valued_attributes_through_domain_heads() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let docs = [
            // Publish the many-valued attribute.
            r#"attribute!: &edge
  description: "A many-valued entity edge"
  the: repro.demo/edge
  as: entity
  cardinality: many
"#,
            // Two domain-head writes against the same entity, in
            // separate commits.
            r#"repro.demo!:
  this: id:a
  edge: id:b
"#,
            r#"repro.demo!:
  this: id:a
  edge: id:c
"#,
        ];
        for doc in docs {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "parse diagnostics for {doc:?}: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate failed for {doc:?}: {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit failed for {doc:?}: {e}"))?;
        }

        // Both edges must be stored: cardinality-many accumulates.
        let a: dialog_artifacts::Entity = "id:a".parse()?;
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("repro.demo/edge"))
                    .of(Term::<dialog_artifacts::Entity>::from(a))
                    .is(Term::<dialog_artifacts::Entity>::var("edge")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let mut edges: Vec<String> = claims
            .iter()
            .map(|claim| format!("{:?}", claim.is))
            .collect();
        edges.sort();
        assert_eq!(
            claims.len(),
            2,
            "a cardinality-many attribute accumulates through a domain head; stored: {edges:?}"
        );
        Ok(())
    }

    /// A raw domain write (`io.test.person!:`) types its values by the
    /// branch-declared attribute, exactly as a concept head does. A
    /// bare integer literal infers signed; stored that way into an
    /// unsigned-declared attribute it is invisible to every typed
    /// read — the concept never matches and the fix looks like data
    /// loss.
    #[dialog_common::test]
    async fn it_types_a_domain_write_by_the_declared_attribute() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let docs = [
            r#"concept!: &person
  description: "A person"
  with:
    name:
      description: "Name"
      the: io.test.person/name
      as: text
    age:
      description: "Age"
      the: io.test.person/age
      as: unsigned-integer
"#,
            // The raw domain write carries a bare literal for the
            // unsigned-declared attribute.
            r#"io.test.person!:
  this: test:1
  name: "Gozala"
  age: 41
"#,
        ];
        for doc in docs {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "parse diagnostics for {doc:?}: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate failed for {doc:?}: {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit failed for {doc:?}: {e}"))?;
        }

        // The stored value carries the DECLARED type, so the concept's
        // typed read matches it.
        let entity: dialog_artifacts::Entity = "test:1".parse()?;
        let the: dialog_artifacts::Attribute = "io.test.person/age".parse()?;
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::new(
                Term::Constant(dialog_artifacts::Value::Symbol(the)),
                Term::<dialog_artifacts::Entity>::from(entity),
                Term::<dialog_query::Any>::var("age"),
                Term::blank(),
                None,
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(claims.len(), 1, "one age fact");
        assert_eq!(
            claims[0].is,
            Value::UnsignedInt(41),
            "the bare literal conforms to the declared unsigned type",
        );
        Ok(())
    }

    /// A keyed-collection field, end to end through notation: the
    /// concept declares `block` as every position-named entry of a
    /// domain, an assertion writes one entry under a literal key,
    /// and the facts land under `domain/key`.
    #[dialog_common::test]
    async fn it_writes_collection_entries_under_their_keys() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let docs = [
            r#"concept!: &notebook
  description: "A notebook of ordered blocks"
  with:
    title:
      description: "The notebook's title"
      the: xyz.test.notebook/title
      as: text
    block:
      description: "The notebook's blocks, in document order"
      the: xyz.test.notebook
      as: {[position]: entity}
"#,
            r#"notebook!:
  this: id:nb
  title: "Scratch"
  block: {N: id:block/1}
"#,
            r#"notebook!:
  this: id:nb
  block: {N5: id:block/2}
"#,
        ];
        for doc in docs {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "parse diagnostics for {doc:?}: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate failed for {doc:?}: {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit failed for {doc:?}: {e}"))?;
        }

        let nb: dialog_artifacts::Entity = "id:nb".parse()?;
        let mut entries: Vec<(String, String)> = Vec::new();
        for key in ["N", "N5"] {
            // A position-named attribute is not a `The` (those are the
            // symbol-named half), so the lookup pins the raw attribute.
            let the: dialog_artifacts::Attribute = format!("xyz.test.notebook/{key}").parse()?;
            let claims: Vec<dialog_query::Claim> = branch
                .query()
                .select(dialog_query::AttributeQuery::new(
                    Term::Constant(dialog_artifacts::Value::Symbol(the)),
                    Term::<dialog_artifacts::Entity>::from(nb.clone()),
                    Term::<dialog_query::Any>::var("block"),
                    Term::blank(),
                    None,
                ))
                .perform(&operator)
                .try_vec()
                .await?;
            for claim in claims {
                let dialog_artifacts::Value::Entity(block) = claim.is else {
                    anyhow::bail!("a block is an entity");
                };
                entries.push((key.to_owned(), block.to_string()));
            }
        }
        assert_eq!(
            entries,
            vec![
                ("N".to_owned(), "id:block/1".to_owned()),
                ("N5".to_owned(), "id:block/2".to_owned()),
            ],
            "each entry is a fact under domain/key"
        );
        Ok(())
    }

    /// `{key: _}` retracts one entry of a keyed collection.
    ///
    /// The blank has to survive as a TRUE blank: a query's `_` mints an
    /// auto-named variable so the matched value projects back, but a
    /// retraction's `_` is the marker that says "dissociate whatever is
    /// here". Minting a variable instead makes the mutation reference
    /// an unbound one, the commit is refused, and the entry stays —
    /// which is a deleted notebook block reappearing on the next
    /// render.
    #[dialog_common::test]
    async fn it_retracts_a_collection_entry_by_key() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let docs = [
            r#"concept!: &notebook
  description: "A notebook of ordered blocks"
  with:
    title:
      description: "The notebook's title"
      the: xyz.test.notebook/title
      as: text
    block:
      description: "The notebook's blocks, in document order"
      the: xyz.test.notebook
      as: {[position]: entity}
"#,
            r#"notebook!:
  this: id:nb
  title: "Scratch"
  block: {N: id:block/1}
"#,
            r#"notebook!:
  this: id:nb
  block: {N5: id:block/2}
"#,
            // The retraction under test.
            r#"notebook!:
  this: id:nb
  block: {N: _}
"#,
        ];
        for doc in docs {
            let parsed = parse(doc);
            assert!(
                parsed.diagnostics.is_empty(),
                "parse diagnostics for {doc:?}: {:?}",
                parsed.diagnostics
            );
            let syntax = parsed.syntax.expect("syntax");
            syntax
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate failed for {doc:?}: {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit failed for {doc:?}: {e}"))?;
        }

        let nb: dialog_artifacts::Entity = "id:nb".parse()?;
        let mut counts = Vec::new();
        for key in ["N", "N5"] {
            let the: dialog_artifacts::Attribute = format!("xyz.test.notebook/{key}").parse()?;
            let claims: Vec<dialog_query::Claim> = branch
                .query()
                .select(dialog_query::AttributeQuery::new(
                    Term::Constant(dialog_artifacts::Value::Symbol(the)),
                    Term::<dialog_artifacts::Entity>::from(nb.clone()),
                    Term::<dialog_query::Any>::var("block"),
                    Term::blank(),
                    None,
                ))
                .perform(&operator)
                .try_vec()
                .await?;
            counts.push(claims.len());
        }

        assert_eq!(counts[0], 0, "the retracted entry is gone");
        assert_eq!(counts[1], 1, "its neighbour is untouched");
        Ok(())
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
            .assert(the!("db.name/referent").of(id_demo).is(target.clone()))
            .commit()
            .perform(&operator)
            .await?;

        // Query id:demo's referent into ?demo, then assert it as
        // id:demo-copy's referent — a different, fresh entity.
        let doc = r#"name:
  this: id:demo
  entity: ?demo

name!:
  this: id:demo-copy
  entity: ?demo
"#;
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit: {e}"))?;

        // id:demo-copy now carries the copied referent.
        let id_copy: dialog_artifacts::Entity = "id:demo-copy".parse()?;
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("db.name/referent"))
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
        let doc = r#"rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
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
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install rule): {e}"))?;
        evaluated
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install rule): {e}"))?;

        // Sanity: the rule's dialog.rule/source claim landed.
        let effect_source_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.rule/source"))
                    .of(Term::<dialog_artifacts::Entity>::var("effect"))
                    .is(Term::<Vec<u8>>::var("source")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            effect_source_claims.len(),
            1,
            "expected exactly one installed rule; saw {effect_source_claims:?}"
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

        branch
            .transaction()
            .dispatch(transients)
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
        let doc = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"

rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        // Sanity: the transient marker landed on ping's
        // descriptor entity.
        let ping_descriptor = one_text_field("io.gozala.ping", "tag");
        let ping_entity = ping_descriptor.this();
        let marker_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.concept/transient"))
                    .of(Term::from(ping_entity.clone()))
                    .is(Term::from(true)),
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
                    .is(Term::from(true)),
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

        branch
            .transaction()
            .dispatch(transients)
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
        let setup = r#"concept!: &counter
  with:
    count:
      the: xyz.tonk.counter/count
      as: unsigned-integer
      cardinality: one
      description: "count"

concept!: &increment
  transient:
  with:
    by:
      the: xyz.tonk.command/increment
      as: unsigned-integer
      cardinality: one
      description: "by"

rule!:
  assert!: counter
  when:
    - assert: increment
      where: { this: ?this, by: ?n }
    - assert: counter
      where: { this: ?this, count: ?m }
    - assert: math/sum
      where: { of: ?n, with: ?m, is: ?count }

counter!: &counter-demo
  count: 0
"#;
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
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (setup): {e}"))?
            .commit()
            .perform(&operator)
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
                Term::<dialog_query::attribute::The>::from(the!("db.name/referent"))
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

        branch
            .transaction()
            .dispatch(transients)
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

    /// Regression for the counter type-mismatch: a transient
    /// `increment` submitted *through notation* (not raw
    /// `Changes`) must drive the `math/sum` rule, and — crucially —
    /// `math/sum`'s written-back `count` must itself be an
    /// unsigned integer so a *second* increment feeds it back into
    /// `math/sum` without `TypeMismatch { expected: UnsignedInt,
    /// actual: SignedInt }`.
    ///
    /// The earlier `it_induces_unsigned_sum_from_transient_increment`
    /// injects the increment as a raw `1u128` `Changes` entry and
    /// runs one round — it never exercises the notation literal
    /// `by: 1` (which parses signed-first) nor the round-2
    /// read-back of `math/sum`'s own output. This test does both:
    /// it is the path the live `/evaluate` bug actually hit.
    #[dialog_common::test]
    async fn it_chains_unsigned_sum_across_two_notation_increments() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit 1: concepts + the summing rule + a counter seeded
        // at 0. The literal `count: 0` goes through the analyzer's
        // schema-directed coercion → an unsigned 0.
        let setup = r#"concept!: &counter
  with:
    count:
      the: xyz.tonk.counter/count
      as: unsigned-integer
      cardinality: one
      description: "count"

concept!: &increment
  transient:
  with:
    by:
      the: xyz.tonk.command/increment
      as: unsigned-integer
      cardinality: one
      description: "by"

rule!:
  assert!: counter
  when:
    - assert: increment
      where: { this: ?this, by: ?n }
    - assert: counter
      where: { this: ?this, count: ?m }
    - assert: math/sum
      where: { of: ?n, with: ?m, is: ?count }

counter!: &counter-demo
  count: 0
"#;
        let parsed = parse(setup);
        assert!(
            parsed.diagnostics.is_empty(),
            "setup parse diagnostics: {:?}",
            parsed.diagnostics
        );
        parsed
            .syntax
            .expect("setup syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (setup): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (setup): {e}"))?;

        // Two transient increments submitted through notation —
        // the literal `by: 1` parses signed-first, so this is the
        // path schema coercion must rescue. The second increment
        // reads `math/sum`'s own round-1 output (`count: 1`) back
        // as `?m`; if that output were signed, induction fails.
        let increment = r#"increment!:
  this: counter-demo
  by: 1
"#;
        for round in 1..=2 {
            let parsed = parse(increment);
            assert!(
                parsed.diagnostics.is_empty(),
                "increment parse diagnostics: {:?}",
                parsed.diagnostics
            );
            parsed
                .syntax
                .expect("increment syntax")
                .evaluate(branch.transaction())
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate (increment {round}): {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit (increment {round}): {e}"))?;
        }

        // Resolve counter-demo's entity via its name referent.
        let counter_demo: dialog_artifacts::Entity =
            "id:counter-demo".parse().expect("id:<name> entity");
        let referent: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("db.name/referent"))
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

        // After two increments of 1, the durable count is an
        // unsigned 2 — proving math/sum's output stayed unsigned
        // across the round-trip.
        let count_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.counter/count"))
                    .of(Term::from(counter_entity))
                    .is(Term::<u128>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            count_claims.iter().any(|c| c.is == Value::UnsignedInt(2)),
            "counter count should be an unsigned 2 after two increments; saw {count_claims:?}"
        );

        Ok(())
    }

    /// Regression for the Dedalus-cascade bug: two semantically
    /// identical rules in the same fixpoint round must converge
    /// to one derivation, not cascade. Before the fix, sibling
    /// rules in a round saw each other's mid-round head writes
    /// through the mutating `txn`: rule A asserted `count = 1`,
    /// rule B then read that and derived `count = 2`. One
    /// `increment{by: 1}` made the counter jump by 2.
    #[dialog_common::test]
    async fn it_converges_two_identical_rules_within_a_round() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let setup = r#"concept!: &counter
  with:
    count:
      the: xyz.tonk.counter/count
      as: unsigned-integer
      cardinality: one
      description: "count"

concept!: &increment
  transient:
  with:
    by:
      the: xyz.tonk.command/increment
      as: unsigned-integer
      cardinality: one
      description: "by"

rule!:
  assert!: counter
  when:
    - assert: increment
      where: { this: ?this, by: ?n }
    - assert: counter
      where: { this: ?this, count: ?m }
    - assert: math/sum
      where: { of: ?n, with: ?m, is: ?count }

rule!:
  assert!: counter
  when:
    - assert: counter
      where: { this: ?this, count: ?m }
    - assert: increment
      where: { this: ?this, by: ?n }
    - assert: math/sum
      where: { of: ?m, with: ?n, is: ?count }

counter!: &counter-demo
  count: 0
"#;
        let parsed = parse(setup);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        parsed
            .syntax
            .expect("setup syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (setup): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (setup): {e}"))?;

        let parsed = parse("increment!:\n  this: counter-demo\n  by: 1\n");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        parsed
            .syntax
            .expect("increment syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (increment): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (increment): {e}"))?;

        let counter_demo: dialog_artifacts::Entity = "id:counter-demo".parse().expect("id:<name>");
        let referent: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("db.name/referent"))
                    .of(Term::from(counter_demo))
                    .is(Term::<dialog_artifacts::Entity>::var("e")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let counter_entity = match &referent[0].is {
            Value::Entity(e) => e.clone(),
            other => panic!("expected entity, got {other:?}"),
        };

        let count_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.counter/count"))
                    .of(Term::from(counter_entity))
                    .is(Term::<u128>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            count_claims.iter().any(|c| c.is == Value::UnsignedInt(1)),
            "two identical rules + one increment must converge to count = 1 \
             (not cascade to 2); saw {count_claims:?}"
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
        let concepts = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"
"#;
        parse(concepts)
            .syntax
            .expect("concepts syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (concepts): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (concepts): {e}"))?;

        // A document that is *only* a rule.
        let rule_doc = r#"rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        let evaluated = parse(rule_doc)
            .syntax
            .expect("rule syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (rule): {e}"))?;

        // The lifted rule must lower to one statement — a rule
        // install — so the route's commit guard (`has_statements`)
        // sees it.
        let statements = evaluated.analysis.analysis.statements();
        assert_eq!(
            statements.len(),
            1,
            "rule-only document should carry one mutation statement"
        );
        let Statement::Assert(Application::Rule { rule, .. }) = &statements[0].statement else {
            panic!(
                "rule-only document should carry Statement::Assert(Application::Rule), got {:?}",
                statements[0].statement
            );
        };
        assert_eq!(
            rule.conclusion().this(),
            one_text_field("io.gozala.pong", "tag").this(),
            "the installed rule's head concept should be pong"
        );

        Ok(())
    }

    /// `rule!: this: <entity>, assert!: ..` is rejected: rules are
    /// content-addressed, and facts pinned at any other entity would
    /// be inert to dialog's readers.
    #[dialog_common::test]
    async fn it_rejects_a_pinned_rule_install() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit the concepts the rule references.
        let concepts = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"
"#;
        parse(concepts)
            .syntax
            .expect("concepts syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (concepts): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (concepts): {e}"))?;

        // A pinned install is refused at analysis time.
        let rule_doc = r#"rule!:
  this: id:my-counter
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        let error = match parse(rule_doc)
            .syntax
            .expect("rule syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("a pinned rule install must be rejected"),
        };
        assert!(
            error.to_string().contains("content-addressed"),
            "refusal should name the content-addressing rule, got: {error}"
        );

        Ok(())
    }

    /// `rule!: this: <entity> ..: _` deletes an installed rule end
    /// to end through the notation path: parse → analyze →
    /// the analyzer resolves the stored rule off the branch →
    /// transaction.retract → commit. After the commit the
    /// `dialog.rule/source` claim at the rule's content-derived
    /// entity must be gone (the canonical encoding makes the
    /// dissociate byte-exact) and the rule must stop firing.
    #[dialog_common::test]
    async fn it_retracts_an_installed_rule_via_notation() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit the concepts the rule references.
        let concepts = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"
"#;
        parse(concepts)
            .syntax
            .expect("concepts syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (concepts): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (concepts): {e}"))?;

        // Install; the rule lands at its content-derived entity.
        let install_doc = r#"rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        parse(install_doc)
            .syntax
            .expect("install syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        // Discover the installed rule's content-derived entity so the
        // retract notation can name it.
        let installed: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.rule/source"))
                    .of(Term::<Entity>::var("rule"))
                    .is(Term::<Vec<u8>>::var("source")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            installed.len(),
            1,
            "the rule should be installed pre-retract"
        );
        let chosen = installed[0].of.clone();
        let source_query = dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(the!("dialog.rule/source"))
                .of(Term::<Entity>::from(chosen.clone()))
                .is(Term::<Vec<u8>>::var("source")),
        );

        // Retract via the notation deletion form. `..: _` is the
        // sentinel for "delete the named rule."
        let retract_doc = format!(
            r#"rule!:
  this: {chosen}
  ..: _
"#
        );
        parse(&retract_doc)
            .syntax
            .expect("retract syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (retract): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (retract): {e}"))?;

        // The source claim must be gone — the dissociate ran with
        // the stored bytes, byte-for-byte. This is the assertion
        // that originally failed under task #83 before #87 landed.
        let post: Vec<dialog_query::Claim> = branch
            .query()
            .select(source_query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            post.is_empty(),
            "dialog.rule/source at {chosen} must be empty after retract, saw {post:?}"
        );

        Ok(())
    }

    /// End-to-end deduction: an installed deductive rule makes a query
    /// for its conclusion concept return instances *derived* from
    /// premise facts (no stored conclusion facts exist), and retracting
    /// the rule makes those derived instances disappear. The premise
    /// facts never change — only the rule's presence does.
    ///
    /// Rule: `pong` is derived from `ping`. A single durable `ping`
    /// instance is on the branch; `pong` is never asserted. Querying
    /// `pong` yields one conclusion while the rule is installed and zero
    /// after it is retracted.
    #[dialog_common::test]
    async fn it_deduces_concepts_until_the_rule_is_retracted() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let ping = one_text_field("io.gozala.ping", "tag");
        let pong = one_text_field("io.gozala.pong", "tag");

        // Concepts + attributes on the branch (both durable; the
        // premise concept is a normal stored one here).
        let mut setup = branch.transaction();
        setup = install_attribute_facts(setup, &ping);
        setup = install_attribute_facts(setup, &pong);
        setup = install_named_concept(setup, "ping", &ping, /*transient=*/ false);
        setup = install_named_concept(setup, "pong", &pong, /*transient=*/ false);
        // One durable `ping` instance — the premise the rule derives from.
        let subject: dialog_artifacts::Entity = "did:key:zDeducedSubject".parse()?;
        setup = setup.assert(
            the!("io.gozala.ping/tag")
                .of(subject.clone())
                .is("hi".to_string()),
        );
        setup.commit().perform(&operator).await?;

        // A query for `pong` instances, all fields free.
        let query_pong = || async {
            let mut terms = Parameters::new();
            terms.insert("this".to_string(), Term::var("this"));
            terms.insert("tag".to_string(), Term::var("tag"));
            let conclusions: Vec<ConceptConclusion> = branch
                .query()
                .select(ConceptQuery {
                    terms,
                    predicate: pong.clone(),
                })
                .perform(&operator)
                .try_vec()
                .await?;
            anyhow::Ok(conclusions)
        };

        // Before the rule: no stored pong facts, no rule — zero pong.
        assert!(
            query_pong().await?.is_empty(),
            "pong should be empty before the rule is installed"
        );

        // Install the deductive rule (`assert:` no-bang); it lands at
        // its content-derived entity.
        let install_doc = r#"rule!:
  assert: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        parse(install_doc)
            .syntax
            .expect("install syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install rule): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install rule): {e}"))?;

        // With the rule installed, querying `pong` derives one instance
        // from the `ping` fact — even though no pong fact was written.
        let deduced = query_pong().await?;
        assert_eq!(
            deduced.len(),
            1,
            "the installed rule should deduce one pong from the ping fact; saw {deduced:?}"
        );

        // Discover the rule's content-derived entity, then retract it.
        let installed: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the!("dialog.rule/source"))
                    .of(Term::<dialog_artifacts::Entity>::var("rule"))
                    .is(Term::<Vec<u8>>::var("source")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(installed.len(), 1, "the deductive rule should be stored");
        let rule_entity = installed[0].of.clone();
        let retract_doc = format!(
            r#"rule!:
  this: {rule_entity}
  ..: _
"#
        );
        parse(&retract_doc)
            .syntax
            .expect("retract syntax")
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (retract rule): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (retract rule): {e}"))?;

        // The ping fact is untouched, but with the rule gone the
        // deduction stops — querying `pong` is empty again.
        assert!(
            query_pong().await?.is_empty(),
            "pong deduction must stop once the rule is retracted"
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
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("evaluate ({label}): {e}"))?
                .commit()
                .perform(&operator)
                .await
                .map_err(|e| anyhow::anyhow!("commit ({label}): {e}"))?;
            Ok(())
        };

        // Commit 1: the transient concept + the durable concept.
        commit_doc(
            r#"concept!: &person-entered
  transient:
  with:
    name:
      the: xyz.tonk.env/name
      as: text
      cardinality: one
      description: "name"
    age:
      the: xyz.tonk.env/age
      as: unsigned-integer
      cardinality: one
      description: "age"

attribute!: &person-name
  description: The person's name
  the: xyz.tonk.person/name
  as: text
  cardinality: one

attribute!: &person-age
  description: The person's age
  the: xyz.tonk.person/age
  as: unsigned-integer
  cardinality: one

concept!: &person
  description: "A person"
  with:
    name: person-name
    age: person-age
"#,
            "concepts",
        )
        .await?;

        // Commit 2: the rule — a separate document.
        commit_doc(
            r#"rule!:
  assert!: person
  when:
    - assert: person-entered
      where: { this: ?this, name: ?name, age: ?age }
"#,
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
        // db.effect/* facts, not held in memory. This is what
        // a separate /evaluate request does.
        let branch = repo.branch("main").open().perform(&operator).await?;

        instance
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (instance): {e}"))?
            .commit()
            .perform(&operator)
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

        let install = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"

rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#;
        let syntax = parse(install).syntax.expect("install syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
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
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (instance): {e}"))?
            .commit()
            .perform(&operator)
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

    /// Same chain in a single commit: install the concepts + the
    /// rule + assert a transient instance, all in one document. The
    /// chain's induce pass (run inside `Evaluate::perform` since the
    /// transaction-takes-induce refactor) fires the rule against
    /// the just-applied transient and lands the durable head before
    /// commit. The transient should not persist; the head should.
    #[dialog_common::test]
    async fn it_drives_fixpoint_from_one_document_with_rule_plus_transient() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // One document declares both concepts (ping transient, pong
        // durable), installs the inductive rule, and asserts a ping
        // instance. The whole thing should commit as a single
        // observable step: pong lands, ping is gone.
        let doc = r#"concept!: &ping
  transient:
  with:
    tag:
      the: io.gozala.ping/tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: io.gozala.pong/tag
      as: text
      cardinality: one
      description: "tag"

rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }

ping!:
  this: did:key:zSingleCommitSubject
  tag: "hi"
"#;

        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit: {e}"))?;

        let subject: dialog_artifacts::Entity = "did:key:zSingleCommitSubject".parse()?;

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
            "rule should have produced a durable pong in the same commit; saw {pong_claims:?}"
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
            "transient ping must not persist past the commit; saw {ping_claims:?}"
        );

        Ok(())
    }

    /// Closing a *background* sheet (one that is not the workspace's
    /// active sheet) must leave `active` pointing at the same sheet —
    /// removing a background tab never steals focus.
    ///
    /// Mirrors the `core.yaml` workspace model: a `workspace` concept
    /// (with `active` + `sheet`), the `workspace/active-sheet` and
    /// `workspace/sheet-member` projections, the transient
    /// `close-sheet` command, and the two close rules (retract the
    /// membership; reassign `active` only when the *active* sheet
    /// closes). The reassign rule is gated on `active: ?sheet`, so a
    /// background close must not match it.
    #[dialog_common::test]
    async fn it_keeps_active_when_a_background_sheet_closes() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Declare the workspace model + close rules, then seed a
        // workspace owning three sheets with `a` active, asserted at
        // `about:blank` across multiple statements — mirroring exactly
        // how core.yaml seeds the demo workspace.
        let install_doc = r#"
concept!: &workspace
  with:
    name:
      the: xyz.tonk.workspace/name
      as: text
      cardinality: one
      description: "name"
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/active-sheet
  with:
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"

concept!: &workspace/sheet-member
  with:
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/activate-sheet
  transient:
  with:
    sheet:
      the: dom.event.detail/sheet
      as: entity
      description: "sheet"

concept!: &workspace/close-sheet
  transient:
  with:
    sheet:
      the: dom.event.detail/closed
      as: entity
      description: "sheet"
    next:
      the: dom.event.detail/next
      as: entity
      description: "next"

rule!:
  assert!: workspace/active-sheet
  when:
    - assert: workspace/activate-sheet
      where: { sheet: ?active }
    - assert: workspace
      where: { this: ?this, sheet: ?active }

rule!:
  retract!: workspace/sheet-member
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet }
    - assert: workspace
      where: { this: ?this, sheet: ?sheet }

rule!:
  assert!: workspace/active-sheet
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet, next: ?active }
    - assert: workspace
      where: { this: ?this, active: ?sheet, sheet: ?active }

workspace!:
  this: about:blank
  name: "W"
  active: did:key:zSheetA
  sheet: did:key:zSheetA

workspace!:
  this: about:blank
  sheet: did:key:zSheetB

workspace!:
  this: about:blank
  sheet: did:key:zSheetC
"#;

        let parsed = parse(install_doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "install parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        let workspace: dialog_artifacts::Entity = "about:blank".parse()?;
        let sheet_b: dialog_artifacts::Entity = "did:key:zSheetB".parse()?;
        let sheet_c: dialog_artifacts::Entity = "did:key:zSheetC".parse()?;

        // Sanity: active is sheet A.
        let active_before: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            active_before.len(),
            1,
            "exactly one active before close; saw {active_before:?}"
        );

        // Submit a transient close-sheet for the BACKGROUND sheet C
        // (active is A). next = B (C's neighbour, as the binder would
        // compute). The retract rule should drop C's membership; the
        // reassign rule must NOT fire (active != C), so active stays A.
        let close_entity: dialog_artifacts::Entity = "did:key:zCloseBgCmd".parse()?;
        let mut transients = Changes::new();
        the!("dom.event.detail/closed")
            .of(close_entity.clone())
            .is(sheet_c.clone())
            .assert(&mut transients);
        the!("dom.event.detail/next")
            .of(close_entity.clone())
            .is(sheet_b.clone())
            .assert(&mut transients);

        branch
            .transaction()
            .dispatch(transients)
            .commit()
            .perform(&operator)
            .await?;

        // Sheet C's membership is gone; A and B remain.
        let members: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/sheet"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let member_set: Vec<String> = members.iter().map(|c| format!("{:?}", c.is)).collect();
        assert!(
            member_set.iter().any(|m| m.contains("zSheetA")),
            "sheet A should remain a member; saw {member_set:?}"
        );
        assert!(
            !member_set.iter().any(|m| m.contains("zSheetC")),
            "sheet C's membership should be retracted; saw {member_set:?}"
        );

        // Active must still be sheet A — closing a background tab does
        // not move focus.
        let active_after: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let active_set: Vec<String> = active_after.iter().map(|c| format!("{:?}", c.is)).collect();
        assert_eq!(
            active_set.len(),
            1,
            "active must remain cardinality-one; saw {active_set:?}"
        );
        assert!(
            active_set[0].contains("zSheetA"),
            "background close must keep active on sheet A; saw {active_set:?}"
        );

        Ok(())
    }

    /// Closing the *active* sheet moves `active` to the neighbour the
    /// command carries (`next`). The companion to
    /// [`it_keeps_active_when_a_background_sheet_closes`]: here the
    /// reassign rule *must* fire, because the closed sheet is the
    /// active one. Shares the same workspace model.
    #[dialog_common::test]
    async fn it_moves_active_to_the_neighbour_when_the_active_sheet_closes() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let install_doc = r#"
concept!: &workspace
  with:
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/active-sheet
  with:
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"

concept!: &workspace/sheet-member
  with:
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/close-sheet
  transient:
  with:
    sheet:
      the: dom.event.detail/closed
      as: entity
      description: "sheet"
    next:
      the: dom.event.detail/next
      as: entity
      description: "next"

rule!:
  retract!: workspace/sheet-member
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet }
    - assert: workspace
      where: { this: ?this, sheet: ?sheet }

rule!:
  assert!: workspace/active-sheet
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet, next: ?active }
    - assert: workspace
      where: { this: ?this, active: ?sheet, sheet: ?active }

workspace!:
  this: did:key:zCloseActiveWorkspace
  active: did:key:zSheetA
  sheet: did:key:zSheetA

workspace!:
  this: did:key:zCloseActiveWorkspace
  sheet: did:key:zSheetB
"#;

        let parsed = parse(install_doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "install parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        let workspace: dialog_artifacts::Entity = "did:key:zCloseActiveWorkspace".parse()?;
        let sheet_a: dialog_artifacts::Entity = "did:key:zSheetA".parse()?;
        let sheet_b: dialog_artifacts::Entity = "did:key:zSheetB".parse()?;

        // Close the ACTIVE sheet A; next = B (its neighbour).
        let close_entity: dialog_artifacts::Entity = "did:key:zCloseActiveCmd".parse()?;
        let mut transients = Changes::new();
        the!("dom.event.detail/closed")
            .of(close_entity.clone())
            .is(sheet_a.clone())
            .assert(&mut transients);
        the!("dom.event.detail/next")
            .of(close_entity.clone())
            .is(sheet_b.clone())
            .assert(&mut transients);

        branch
            .transaction()
            .dispatch(transients)
            .commit()
            .perform(&operator)
            .await?;

        // Active must now be sheet B (the neighbour).
        let active_after: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let active_set: Vec<String> = active_after.iter().map(|c| format!("{:?}", c.is)).collect();
        assert_eq!(
            active_set.len(),
            1,
            "active must remain cardinality-one; saw {active_set:?}"
        );
        assert!(
            active_set[0].contains("zSheetB"),
            "closing the active sheet must move active to the neighbour B; saw {active_set:?}"
        );

        Ok(())
    }

    /// Repro of the live focus bug: seed a workspace, *create* a new
    /// sheet via the `create-sheet` command (the "Adds new sheet" rule
    /// re-asserts the whole `workspace` to add it), then close that new
    /// sheet while a *different* sheet is active. Closing the new
    /// (background) sheet must not disturb `active`.
    ///
    /// Mirrors `core.yaml`: the create-sheet command + its two rules
    /// (mint the sheet, add it to the workspace) plus the close rules.
    #[dialog_common::test]
    async fn it_keeps_active_after_creating_then_closing_a_background_sheet() -> anyhow::Result<()>
    {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let install_doc = r#"
concept!: &workspace
  with:
    name:
      the: xyz.tonk.workspace/name
      as: text
      cardinality: one
      description: "name"
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/sheet
  with:
    title:
      the: xyz.tonk.artifact/title
      as: text
      cardinality: one
      description: "title"
    order:
      the: xyz.tonk.sheet/order
      as: text
      cardinality: one
      description: "order"

concept!: &workspace/active-sheet
  with:
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"

concept!: &workspace/sheet-member
  with:
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/create-sheet
  transient:
  with:
    name:
      the: dom.event.detail/name
      as: text
      description: "name"
    order:
      the: dom.event.detail/order
      as: text
      description: "order"

concept!: &workspace/close-sheet
  transient:
  with:
    sheet:
      the: dom.event.detail/closed
      as: entity
      description: "sheet"
    next:
      the: dom.event.detail/next
      as: entity
      description: "next"

rule!:
  assert!: workspace/sheet
  when:
    - assert: workspace/create-sheet
      where: { this: ?this, name: ?title, order: ?order }

rule!:
  assert!: workspace
  when:
    - assert: workspace/create-sheet
      where: { this: ?sheet, name: ?title, order: ?order }
    - assert: workspace
      where: { this: ?this, name: ?name, active: ?active }

rule!:
  retract!: workspace/sheet-member
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet }
    - assert: workspace
      where: { this: ?this, sheet: ?sheet }

rule!:
  assert!: workspace/active-sheet
  when:
    - assert: workspace/close-sheet
      where: { sheet: ?sheet, next: ?active }
    - assert: workspace
      where: { this: ?this, active: ?sheet, sheet: ?active }

workspace!:
  this: did:key:zCreateCloseWorkspace
  name: "W"
  active: did:key:zSheetA
  sheet: did:key:zSheetA

workspace!:
  this: did:key:zCreateCloseWorkspace
  sheet: did:key:zSheetB
"#;

        let parsed = parse(install_doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "install parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        let workspace: dialog_artifacts::Entity = "did:key:zCreateCloseWorkspace".parse()?;

        // Fire create-sheet (the new sheet entity = the command entity).
        let new_sheet: dialog_artifacts::Entity = "did:key:zCreatedSheet".parse()?;
        let mut create = Changes::new();
        the!("dom.event.detail/name")
            .of(new_sheet.clone())
            .is("New".to_string())
            .assert(&mut create);
        the!("dom.event.detail/order")
            .of(new_sheet.clone())
            .is("e".to_string())
            .assert(&mut create);

        branch
            .transaction()
            .dispatch(create)
            .commit()
            .perform(&operator)
            .await?;

        // The new sheet is now a member, active is still A.
        let active_mid: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let active_mid_set: Vec<String> =
            active_mid.iter().map(|c| format!("{:?}", c.is)).collect();
        assert_eq!(
            active_mid_set.len(),
            1,
            "after create, exactly one active; saw {active_mid_set:?}"
        );
        assert!(
            active_mid_set[0].contains("zSheetA"),
            "creating a sheet must not change active; saw {active_mid_set:?}"
        );

        // Now close the NEW (background) sheet; active is A, next = B.
        let sheet_b: dialog_artifacts::Entity = "did:key:zSheetB".parse()?;
        let close_entity: dialog_artifacts::Entity = "did:key:zCreateCloseCmd".parse()?;
        let mut close = Changes::new();
        the!("dom.event.detail/closed")
            .of(close_entity.clone())
            .is(new_sheet.clone())
            .assert(&mut close);
        the!("dom.event.detail/next")
            .of(close_entity.clone())
            .is(sheet_b.clone())
            .assert(&mut close);
        branch
            .transaction()
            .dispatch(close)
            .commit()
            .perform(&operator)
            .await?;

        // Active must STILL be A — closing the new background sheet must
        // not move focus.
        let active_after: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let active_set: Vec<String> = active_after.iter().map(|c| format!("{:?}", c.is)).collect();
        assert_eq!(
            active_set.len(),
            1,
            "active must remain cardinality-one after closing the new sheet; saw {active_set:?}"
        );
        assert!(
            active_set[0].contains("zSheetA"),
            "closing the new background sheet must keep active on A; saw {active_set:?}"
        );

        Ok(())
    }

    /// The real `core.yaml` create flow: a `create-sheet` command mints
    /// a self-describing empty sheet (entity = the sheet itself, model =
    /// empty-artifact), titles its empty-artifact entity with the typed
    /// name, and auto-activates it. Mirrors the three create rules in
    /// the standard library.
    #[dialog_common::test]
    async fn it_creates_a_self_describing_empty_sheet_and_activates_it() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let install_doc = r#"
concept!: &workspace
  with:
    name:
      the: xyz.tonk.workspace/name
      as: text
      cardinality: one
      description: "name"
    active:
      the: xyz.tonk.workspace/active
      as: entity
      cardinality: one
      description: "active"
    sheet:
      the: xyz.tonk.workspace/sheet
      as: entity
      cardinality: many
      description: "sheet"

concept!: &workspace/sheet
  with:
    title:
      the: xyz.tonk.artifact/title
      as: text
      cardinality: one
      description: "title"
    entity:
      the: xyz.tonk.artifact/entity
      as: entity
      cardinality: one
      description: "entity"
    model:
      the: xyz.tonk.artifact/model
      as: entity
      cardinality: one
      description: "model"
    order:
      the: xyz.tonk.sheet/order
      as: text
      cardinality: one
      description: "order"

concept!: &empty-artifact
  this: tonk:empty-artifact
  with:
    title:
      the: xyz.tonk.artifact/title
      as: text
      cardinality: one
      description: "title"

concept!: &workspace/create-sheet
  transient:
  with:
    name:
      the: dom.event.detail/name
      as: text
      description: "name"
    order:
      the: dom.event.detail/order
      as: text
      description: "order"

rule!:
  assert!: workspace/sheet
  when:
    - assert: workspace/create-sheet
      where: { this: ?this, name: ?title, order: ?order }
    - assert: ==
      where: { this: ?entity, is: ?this }
    - assert: ==
      where: { this: ?model, is: tonk:empty-artifact }

rule!:
  assert!: empty-artifact
  when:
    - assert: workspace/create-sheet
      where: { this: ?this, name: ?title, order: ?order }

rule!:
  assert!: workspace
  when:
    - assert: workspace/create-sheet
      where: { this: ?sheet, name: ?title, order: ?order }
    - assert: workspace
      where: { this: ?this, name: ?name }
    - assert: ==
      where: { this: ?this, is: about:blank }
    - assert: ==
      where: { this: ?active, is: ?sheet }

workspace!:
  this: about:blank
  name: "W"
  active: did:key:zSheetA
  sheet: did:key:zSheetA
"#;

        let parsed = parse(install_doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "install parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate (install): {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit (install): {e}"))?;

        // Fire create-sheet. The new sheet entity = the command entity.
        let new_sheet: dialog_artifacts::Entity = "did:key:zNewSheet".parse()?;
        let mut create = Changes::new();
        the!("dom.event.detail/name")
            .of(new_sheet.clone())
            .is("Notes".to_string())
            .assert(&mut create);
        the!("dom.event.detail/order")
            .of(new_sheet.clone())
            .is("bz".to_string())
            .assert(&mut create);

        branch
            .transaction()
            .dispatch(create)
            .commit()
            .perform(&operator)
            .await?;

        // The sheet's title is the typed name.
        let title: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.artifact/title"))
                    .of(Term::from(new_sheet.clone()))
                    .is(Term::<String>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            title
                .iter()
                .any(|c| format!("{:?}", c.is).contains("Notes")),
            "the new sheet/empty-artifact should be titled \"Notes\"; saw {title:?}"
        );

        // The sheet's entity points at itself, and its model is the
        // empty-artifact concept.
        let entity: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.artifact/entity"))
                    .of(Term::from(new_sheet.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            entity
                .iter()
                .any(|c| format!("{:?}", c.is).contains("zNewSheet")),
            "the sheet's entity should be self-referential; saw {entity:?}"
        );
        let model: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.artifact/model"))
                    .of(Term::from(new_sheet.clone()))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            model
                .iter()
                .any(|c| format!("{:?}", c.is).contains("tonk:empty-artifact")),
            "the sheet's model should be tonk:empty-artifact; saw {model:?}"
        );

        // The workspace auto-activated the new sheet.
        let workspace: dialog_artifacts::Entity = "about:blank".parse()?;
        let active: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::from(the!("xyz.tonk.workspace/active"))
                    .of(Term::from(workspace))
                    .is(Term::<dialog_artifacts::Entity>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        let active_set: Vec<String> = active.iter().map(|c| format!("{:?}", c.is)).collect();
        assert_eq!(
            active_set.len(),
            1,
            "active must be cardinality-one; saw {active_set:?}"
        );
        assert!(
            active_set[0].contains("zNewSheet"),
            "creating must auto-activate the new sheet; saw {active_set:?}"
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

    /// End-to-end: a `concept!` with a `maybe:` block lands the
    /// optional marker (`db.concept.optional/{field}`) on the
    /// branch for the optional field only — the required `with:`
    /// field carries none. Proves the notation → analyzer →
    /// storage round-trip for optionality.
    #[dialog_common::test]
    async fn it_persists_optional_marker_for_maybe_field() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let doc = r#"concept!: &person
  with:
    name:
      the: io.gozala.person/name
      as: text
      cardinality: one
      description: "name"
  maybe:
    nickname:
      the: io.gozala.person/nickname
      as: text
      cardinality: one
      description: "nickname"
"#;
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");

        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("commit: {e}"))?;

        // The descriptor entity is content-derived; rebuild it the
        // same way (required name + optional nickname) to find it.
        let descriptor = ConceptDescriptor::try_from(vec![
            (
                "name".to_string(),
                dialog_query::ConceptFieldDescriptor::required(AttributeDescriptor::new(
                    the!("io.gozala.person/name"),
                    "name",
                    DialogCardinality::One,
                    Some(Type::String),
                )),
            ),
            (
                "nickname".to_string(),
                dialog_query::ConceptFieldDescriptor::optional(AttributeDescriptor::new(
                    the!("io.gozala.person/nickname"),
                    "nickname",
                    DialogCardinality::One,
                    Some(Type::String),
                )),
            ),
        ])?;
        let entity = descriptor.this();

        let nickname_the: dialog_query::attribute::The =
            "db.concept.optional/nickname".parse().unwrap();
        let nickname_markers: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(nickname_the)
                    .of(Term::from(entity.clone()))
                    .is(Term::<bool>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(
            nickname_markers.len(),
            1,
            "expected an optional marker for `nickname`; saw {nickname_markers:?}"
        );
        assert_eq!(nickname_markers[0].is, Value::Boolean(true));

        let name_the: dialog_query::attribute::The = "db.concept.optional/name".parse().unwrap();
        let name_markers: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(name_the)
                    .of(Term::from(entity.clone()))
                    .is(Term::<bool>::var("v")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            name_markers.is_empty(),
            "required field `name` must carry no optional marker; saw {name_markers:?}"
        );
        Ok(())
    }

    /// Headline behavior, end to end: a `concept:` query returns
    /// entities that LACK an optional field (with the field omitted
    /// from the result), alongside entities that HAVE it (field
    /// present). Set-widening must not drop the field-less entity.
    ///
    /// NOTE: this passes only because the optional field (`nickname`)
    /// sorts *after* the required field (`name`), so the required
    /// field leads the scan. The mirror case where the optional
    /// field sorts first is currently broken by a dialog-db planner
    /// bug — see
    /// [`dialog_repro_optional_field_sorted_first_drops_rows`] and
    /// the ignored [`it_set_widens_body_derived_entities_with_bare_query`].
    #[dialog_common::test]
    async fn it_set_widens_optional_field_in_query() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Declare the concept and assert two people in one commit:
        // alice has a nickname, bob does not.
        let setup = r#"concept!: &person
  with:
    name:
      the: io.gozala.person/name
      as: text
      cardinality: one
      description: "name"
  maybe:
    nickname:
      the: io.gozala.person/nickname
      as: text
      cardinality: one
      description: "nickname"

person!:
  this: id:alice
  name: "Alice"
  nickname: "Al"

person!:
  this: id:bob
  name: "Bob"
"#;
        let parsed = parse(setup);
        assert!(
            parsed.diagnostics.is_empty(),
            "setup parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup commit: {e}"))?;

        // Query every person.
        let query_doc = r#"person:
  this: ?p
  name: ?name
  nickname: ?nick
"#;
        let parsed = parse(query_doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "query parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let query_syntax = parsed.syntax.expect("query syntax");
        let evaluated = query_syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("query evaluate: {e}"))?;

        let results: Vec<&QueryResult> = evaluated
            .matches
            .iter()
            .flat_map(|b| b.results.iter())
            .collect();

        // Both people come back — the optional field must not filter
        // bob out.
        assert_eq!(
            results.len(),
            2,
            "expected both people (set-widening must not drop the nickname-less one); saw {results:?}"
        );

        let alice = results
            .iter()
            .find(|r| r.fields.get("name") == Some(&serde_json::json!("Alice")))
            .expect("alice row present");
        let bob = results
            .iter()
            .find(|r| r.fields.get("name") == Some(&serde_json::json!("Bob")))
            .expect("bob row present");

        // Alice has the optional field; bob omits it entirely.
        assert_eq!(
            alice.fields.get("nickname"),
            Some(&serde_json::json!("Al")),
            "alice's nickname must be present"
        );
        assert!(
            !bob.fields.contains_key("nickname"),
            "bob lacks the optional field, so it must be omitted; saw {:?}",
            bob.fields
        );
        Ok(())
    }

    /// Reproduces the reported failure end-to-end through notation:
    /// a concept with required `name` + optional `age`, two people
    /// (one without `age`), queried with bare `person:`. The
    /// `age`-less person must still appear.
    ///
    /// This previously failed (only the person *with* `age` returned)
    /// because `age` sorts before `name`, so the optional field led
    /// the unbound scan. Fixed by the dialog-db query-engine rework
    /// on `feat/narrowing-diagnostics`; kept as a regression guard.
    #[dialog_common::test]
    async fn it_set_widens_body_derived_entities_with_bare_query() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let setup = r#"concept!: &person
  description: A person
  with:
    name:
      description: Name of the person
      the: xyz.tonk.person/name
      as: text
  maybe:
    age:
      description: Age of the person
      the: xyz.tonk.person/age
      as: unsigned-integer

person!:
  name: Alice

person!:
  name: Bob
  age: 4
"#;
        let parsed = parse(setup);
        assert!(
            parsed.diagnostics.is_empty(),
            "setup parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup commit: {e}"))?;

        let parsed = parse("person:\n");
        assert!(
            parsed.diagnostics.is_empty(),
            "query parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let query_syntax = parsed.syntax.expect("query syntax");
        let evaluated = query_syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("query evaluate: {e}"))?;

        let results: Vec<&QueryResult> = evaluated
            .matches
            .iter()
            .flat_map(|b| b.results.iter())
            .collect();

        let names: Vec<_> = results
            .iter()
            .filter_map(|r| r.fields.get("name").and_then(|v| v.as_str()))
            .collect();
        assert!(
            names.contains(&"Alice"),
            "Alice (no optional `age`) must appear; saw {results:?}"
        );
        assert!(names.contains(&"Bob"), "Bob must appear; saw {results:?}");
        assert_eq!(results.len(), 2, "expected both people; saw {results:?}");
        Ok(())
    }

    /// Regression guard at the **dialog level** for set-widening
    /// when the optional field sorts alphabetically *before* every
    /// required field. Uses only `dialog_query` + `dialog_repository`
    /// APIs — no tonk notation, analyzer, or reconstruction.
    ///
    /// The concept has `bio` (optional, sorts first) and `name`
    /// (required). A `this`-unbound query must return both alice
    /// (`bio` Absent) and bob (`bio` Present). This previously
    /// returned only bob, because the planner led the unbound scan
    /// with the optional `bio` premise. Fixed by the dialog-db
    /// query-engine rework on `feat/narrowing-diagnostics`.
    #[dialog_common::test]
    async fn it_set_widens_optional_field_sorted_before_required() -> anyhow::Result<()> {
        use dialog_query::concept::descriptor::ConceptConclusion;
        use dialog_query::concept::query::ConceptQuery;
        use dialog_query::{Output as _, Parameters, Term};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let alice: dialog_artifacts::Entity = "id:alice".parse()?;
        let bob: dialog_artifacts::Entity = "id:bob".parse()?;

        // alice has only name; bob has name + bio.
        branch
            .transaction()
            .assert(the!("person/name").of(alice).is("Alice".to_string()))
            .assert(the!("person/name").of(bob.clone()).is("Bob".to_string()))
            .assert(the!("person/bio").of(bob).is("Hi".to_string()))
            .commit()
            .perform(&operator)
            .await?;

        // `bio` (optional) sorts before `name` (required).
        let descriptor = ConceptDescriptor::try_from(vec![
            (
                "bio".to_string(),
                dialog_query::ConceptFieldDescriptor::optional(AttributeDescriptor::new(
                    the!("person/bio"),
                    "",
                    DialogCardinality::One,
                    Some(Type::String),
                )),
            ),
            (
                "name".to_string(),
                dialog_query::ConceptFieldDescriptor::required(AttributeDescriptor::new(
                    the!("person/name"),
                    "",
                    DialogCardinality::One,
                    Some(Type::String),
                )),
            ),
        ])?;

        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::var("person"));
        terms.insert("name".to_string(), Term::var("name"));
        terms.insert("bio".to_string(), Term::var("bio"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(ConceptQuery {
                terms,
                predicate: descriptor,
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(
            conclusions.len(),
            2,
            "expected alice (bio Absent) + bob (bio Present); got {} — \
             the optional lead premise dropped alice",
            conclusions.len()
        );
        Ok(())
    }

    /// User-reported repro, end-to-end through notation: a concept
    /// with a `text` field, asserting a bare integer (`age: 3`)
    /// into it. The evaluate path must fail with a type error
    /// rather than committing a `SignedInt` fact that the strictly
    /// typed `person:` concept query can never match (which made
    /// the entity silently vanish from its own concept). The
    /// analyzer-level diagnostic and "untyped accepts any" cases
    /// are covered in `tonk-analyzer`; this guards the wiring
    /// through `evaluate`.
    #[dialog_common::test]
    async fn it_errors_evaluating_integer_into_text_field() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let setup = r#"concept!: &person
  description: Person
  with:
    age:
      description: Age of the person
      the: xyz.tonk.person/age
      as: text
"#;
        let parsed = parse(setup);
        let syntax = parsed.syntax.expect("syntax");
        syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup evaluate: {e}"))?
            .commit()
            .perform(&operator)
            .await
            .map_err(|e| anyhow::anyhow!("setup commit: {e}"))?;

        let parsed = parse("person!:\n  age: 3\n");
        let query_syntax = parsed.syntax.expect("assert syntax");
        let result = query_syntax
            .evaluate(branch.transaction())
            .perform(&operator)
            .await;
        let err = result.err().expect("integer into a text field must error");
        assert!(
            err.to_string().to_lowercase().contains("text")
                || err.to_string().to_lowercase().contains("type"),
            "expected a type-mismatch error mentioning the text type, got: {err}"
        );
        Ok(())
    }
}
