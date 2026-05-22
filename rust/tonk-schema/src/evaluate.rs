//! Shared analyze → query → mutation pipeline for asserted-notation.
//!
//! The public surface is [`TransactionEvaluateExt::evaluate`], which
//! returns an [`Evaluate`] chain (mirroring dialog's
//! `Branch::commit(...)` pattern). Callers reach the post-mutation
//! transaction via `.perform(branch, env).await`:
//!
//! ```ignore
//! use tonk_schema::evaluate::TransactionEvaluateExt;
//!
//! let evaluated = branch.transaction()
//!     .evaluate(&syntax)
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
//! let result = branch.transaction()
//!     .evaluate(&syntax)
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

use crate::analyzer;
use crate::transact::{
    Analysis, Application, ApplicationPlan, Planner as _, QueryAnalysis, Statement,
};

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
// Public entry point — txn.evaluate(syntax).perform(branch, env)   //
// ---------------------------------------------------------------- //

/// Extension trait that adds [`Self::evaluate`] to dialog's
/// [`Transaction`]. Imported at call sites to use the chain.
pub trait TransactionEvaluateExt<'a> {
    /// Stage an evaluation of `syntax` against this transaction.
    /// Returns a chain handle; call `.perform(branch, env)` to
    /// execute.
    fn evaluate<'s>(self, syntax: &'s Syntax) -> Evaluate<'a, 's>;
}

impl<'a> TransactionEvaluateExt<'a> for Transaction<'a> {
    fn evaluate<'s>(self, syntax: &'s Syntax) -> Evaluate<'a, 's> {
        Evaluate { txn: self, syntax }
    }
}

/// Chain handle for an evaluation. Holds the transaction and
/// syntax until `.perform(branch, env)` consumes them.
pub struct Evaluate<'a, 's> {
    txn: Transaction<'a>,
    syntax: &'s Syntax,
}

impl<'a, 's> Evaluate<'a, 's> {
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
        let Evaluate { mut txn, syntax } = self;

        let resolver = analyzer::SourceResolver::new(branch, env);
        let analysis = analyzer::analyze(syntax, &resolver)
            .await
            .map_err(EvaluateError::Analyze)?;

        // ---- Build base bindings frame from analysis-derived vars ----
        let mut base = Parameters::new();
        for (name, entity) in &analysis.variables {
            base.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
        }

        // ---- Per-expression queries + post-join ----
        let pre_results = match &analysis.query {
            Some(q) => Some(run_query(q, branch, env).await?),
            None => None,
        };
        let pre_matches: Vec<Parameters> = match &pre_results {
            Some(r) if !r.joined.is_empty() => r.joined.clone(),
            _ => vec![Parameters::new()],
        };

        // ---- Commit-summary seed: published declarations + analysis variables ----
        let mut commits = CommitSummary::default();
        for (key, entity) in &analysis.declarations {
            commits.entities.insert(key.clone(), entity.to_string());
        }
        for (key, entity) in &analysis.variables {
            commits
                .entities
                .insert(format!("?{key}"), entity.to_string());
        }

        // ---- Plan + apply mutations to the caller's transaction ----
        let mut claim_count = 0usize;
        // Retraction targets resolved by querying the branch
        // up-front so we don't interleave reads with mutation
        // accumulation against the transaction.
        let mut retract_claims: Vec<RawClaim> = Vec::new();
        // Transient-concept assertions, accumulated so the caller
        // can hand them to `induce` as the effects-fixpoint seed.
        // An assertion's concept entity is in
        // `analysis.mutate.transient` when the concept was
        // declared `transient:`.
        let mut transients = Changes::new();
        if !analysis.mutate.statements.is_empty() {
            for match_frame in &pre_matches {
                let mut frame = base.clone();
                for (k, v) in match_frame.iter() {
                    frame.insert(k.clone(), v.clone());
                }
                for statement in &analysis.mutate.statements {
                    let plan = statement
                        .application()
                        .clone()
                        .plan(&frame)
                        .map_err(|e| EvaluateError::Plan(format!("plan failed: {e}")))?;
                    match statement {
                        Statement::Assert(_) => {
                            claim_count += count_emitted_claims(&plan);
                            // A transient-concept assertion also
                            // seeds the transient bucket so the
                            // effects fixpoint fires on it and it's
                            // swept before the durable commit.
                            if analysis
                                .mutate
                                .transient
                                .contains(&plan.statement.predicate.this())
                            {
                                crate::effects::accumulate_head_facts(
                                    &plan.statement,
                                    &mut transients,
                                );
                            }
                            txn = txn.assert(plan);
                        }
                        Statement::Retract(_) => {
                            let resolved = resolve_retraction_targets(plan, branch, env).await?;
                            claim_count += resolved.len();
                            retract_claims.extend(resolved);
                        }
                    }
                }
            }
            for claim in retract_claims {
                txn = txn.retract(claim);
            }
        }
        commits.claims = claim_count;

        // ---- Install effects ----
        // Each rule!: expression lifted in the analyzer's
        // Phase 3b lands here as an `Effect`. The
        // `Effect: Statement` impl writes the
        // `dialog.effect/*` facts that the reactor's `induce`
        // loop reads on every subsequent commit.
        //
        // Each effect emits a constant set of claims (marker +
        // source + conclusion + polarity + one `on:` entry per
        // attribute the body reads) — bump the claim count by
        // a flat 4 + premise-attribute count so the response's
        // `commits.claims` reflects what's about to land
        // durably.
        for effect in analysis.effects.iter().cloned() {
            commits.claims += 4 + effect.on_entities().len();
            txn = txn.assert(effect);
        }

        let matches = render_match_blocks(&analysis, pre_results.as_ref());

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
    /// The analyzer's output. Callers re-run its queries
    /// against the transaction overlay (or the post-commit
    /// branch) to compute post-mutation matches.
    pub analysis: Analysis,
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
        // pure-mutation docs (no `analysis.query`), the after
        // block is empty.
        let post_results = match &analysis.query {
            Some(q) => Some(run_query(q, branch, env).await?),
            None => None,
        };
        let matches_after = render_match_blocks(&analysis, post_results.as_ref());

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
    /// The analyzer's output, in case the caller wants further
    /// queries against the analysis.
    pub analysis: Analysis,
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
    /// `QueryAnalysis::synthesized`. Each snapshot runs
    /// standalone — never joined — and is rendered as its own
    /// match block.
    synthesized_per_expression: Vec<Vec<Parameters>>,
}

/// Run each expression's [`Application`] independently and join
/// their frames on shared user-named variables.
///
/// `Application` impls `dialog_query::Application` and dispatches
/// internally to the right [`crate::concept::QueryPlan`] (built-in
/// or branch concept), so this loop is uniform across head kinds.
async fn run_query<Env: EvaluateEnv>(
    query: &QueryAnalysis,
    branch: &Branch,
    env: &Env,
) -> Result<QueryResults, EvaluateError> {
    let mut per_expression = Vec::with_capacity(query.queries.len());
    for application in &query.queries {
        let frames = collect_matches(application.clone(), branch, env).await?;
        per_expression.push(frames);
    }
    let joined = natural_join(&per_expression);

    // Synthesized snapshots run standalone — never joined into
    // `joined`, so an empty snapshot can't zero mutation
    // planning's binding set.
    let mut synthesized_per_expression = Vec::with_capacity(query.synthesized.len());
    for application in &query.synthesized {
        let frames = collect_matches(application.clone(), branch, env).await?;
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
        .select(application)
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
    analysis: &Analysis,
    results: Option<&QueryResults>,
) -> Vec<QueryMatchBlock> {
    let Some(query) = &analysis.query else {
        return Vec::new();
    };
    let Some(results) = results else {
        return Vec::new();
    };

    let mut blocks = Vec::with_capacity(query.queries.len() + query.synthesized.len());

    // User-written queries: each block draws from the joined
    // frames so connected queries display the filtered
    // intersection.
    for (i, application) in query.queries.iter().enumerate() {
        let label = query
            .labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| "?".to_owned());
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
        blocks.push(render_block(label, application, source_frames));
    }

    // Synthesized snapshot queries: each runs standalone (no
    // join), rendered from its own per-expression frames.
    for (i, application) in query.synthesized.iter().enumerate() {
        let label = query
            .synthesized_labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| "?".to_owned());
        let source_frames: &[Parameters] = results
            .synthesized_per_expression
            .get(i)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        blocks.push(render_block(label, application, source_frames));
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
    use crate::concept::{AnonymousConcept, TransientConcept};
    use dialog_artifacts::Statement;
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
    /// `QueryAnalysis::synthesized`, not `queries` — or it zeroes
    /// the join and the assert's `?var` goes unbound.
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

        branch
            .transaction()
            .evaluate(&syntax)
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
        let evaluated = branch
            .transaction()
            .evaluate(&syntax)
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

        branch
            .transaction()
            .evaluate(&syntax)
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
}
