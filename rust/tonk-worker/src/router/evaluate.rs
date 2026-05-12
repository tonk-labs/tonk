//! Evaluate route — accepts an asserted-notation document and
//! drives the unified analyze → query → plan → commit pipeline.
//!
//! Pipeline:
//! 1. Parse the body into a [`Syntax`].
//! 2. Analyze it into a [`tonk_schema::transact::Analysis`].
//! 3. If the document carries a query, run the unified
//!    [`ConceptQuery`] derived from it; otherwise use a single
//!    empty binding frame (so pure-mutation docs commit once).
//! 4. For each match, plan every mutation [`Statement`] against
//!    `analysis.variables ∪ match`, then assert / retract via
//!    [`ApplicationPlan`].
//! 5. Capture the branch revision before and after the commit
//!    so the response carries a snapshot pair.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use async_trait::async_trait;
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{Entity, Value};
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::{ConceptDescriptor, ConceptQuery, Output as _, Parameters, Term};
use dialog_repository::{Branch, Revision};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_notation::{Parsed, Syntax, parse};
use tonk_schema::{
    analyzer::{self, ResolvedAttribute, ResolvedConcept, Resolver, ResolverError},
    concept::{AttributeByEntity, AttributeByName, Concept as ConceptLookup, lookup_named_entity},
    transact::{Analysis, ApplicationPlan, Planner as _, Statement},
};

use super::AppState;
use crate::{TonkWorkerError, worker::DefaultOperator};

/// Path parameters for the evaluate route.
#[derive(Debug, Deserialize)]
pub struct EvaluatePath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Path parameters for the profile-side evaluate route. The
/// profile is a singleton — no `repo` segment.
#[derive(Debug, Deserialize)]
pub struct ProfileEvaluatePath {
    /// The branch name.
    pub branch: String,
}

/// Query-string parameters for the evaluate route. Today the only
/// option is `transact`, which lets the caller suppress the
/// commit step so auto-fire evaluates (e.g. when the editor
/// settles after an edit) can project what *would* happen
/// without applying mutations the user hasn't confirmed.
#[derive(Debug, Deserialize)]
pub struct EvaluateQuery {
    /// When `false`, run analysis and queries but do not commit
    /// any mutation. The response carries the same shape as a
    /// real evaluate (`matches_before`, etc.) — `commits.claims`
    /// will be `0` and `revision_after == revision_before`.
    ///
    /// Defaults to `true` (full commit) so existing callers keep
    /// today's behavior. Accepts `true`/`false`, `1`/`0`,
    /// `yes`/`no`.
    #[serde(default = "default_true", deserialize_with = "deserialize_bool")]
    pub transact: bool,
}

impl Default for EvaluateQuery {
    fn default() -> Self {
        Self { transact: true }
    }
}

fn default_true() -> bool {
    true
}

/// Deserialize a query-string boolean from the loose forms a
/// browser query string might carry — `true`, `false`, `1`,
/// `0`, `yes`, `no`. Anything else falls back to `true` so a
/// stray value can't accidentally suppress the commit.
fn deserialize_bool<'de, D: serde::Deserializer<'de>>(de: D) -> Result<bool, D::Error> {
    let raw = String::deserialize(de)?;
    Ok(!matches!(
        raw.to_ascii_lowercase().as_str(),
        "false" | "0" | "no"
    ))
}

/// Response from a successful evaluate.
///
/// Carries the branch revision before and after the commit
/// (one before/after pair — every match's mutations land in the
/// same dialog transaction), the per-source-expression query
/// matches in both pre- and post-commit shape, and a commit
/// summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluateResponse {
    /// Revision of the branch before the commit, if any. `None`
    /// when the branch had no prior commits.
    pub revision_before: Option<Revision>,
    /// Revision of the branch after the commit. Same as
    /// `revision_before` when the document carried no mutations.
    pub revision_after: Option<Revision>,
    /// Per-source-expression query matches as they looked
    /// *before* the commit. Same shape as `matches_after`. For
    /// pure-query documents this equals `matches_after`.
    pub matches_before: Vec<QueryMatchBlock>,
    /// Per-source-expression query matches as they look *after*
    /// the commit — what the user just produced.
    pub matches_after: Vec<QueryMatchBlock>,
    /// Commit summary — number of EAV claims written/retracted
    /// and the entities the commit touched.
    pub commits: CommitSummary,
}

/// Matches for one source-expression query, projected back into
/// the user's view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryMatchBlock {
    /// Display label for the source expression
    /// (`person:\n  this: ?alice` → `"person"`).
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

/// `POST /api/repository/{repo}/branch/{branch}/evaluate`
///
/// Body: an asserted-notation document — any mix of queries and
/// mutations. Returns query matches and a commit summary in one
/// response.
///
/// Query string:
/// - `transact=false` — analyze + run queries but skip the
///   commit. Used by the editor's auto-evaluate (on idle) to
///   project query results without applying mutations the user
///   hasn't confirmed. `commits.claims` will be `0` and
///   `revision_after == revision_before`. Defaults to `true`.
#[wasm_compat]
pub async fn evaluate(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    axum::extract::Query(query): axum::extract::Query<EvaluateQuery>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate repo={}, branch={}", path.repo, path.branch);
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);
    evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
}

/// `POST /api/profile/branch/{branch}/evaluate`
///
/// Profile-side counterpart to [`evaluate`]. The profile is its
/// own repository but lives outside the named-repo namespace, so
/// the route surface is parallel to the repository routes rather
/// than nested under one. Same body / query-string / response
/// contract.
#[wasm_compat]
pub async fn evaluate_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileEvaluatePath>,
    axum::extract::Query(query): axum::extract::Query<EvaluateQuery>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate profile branch={}", path.branch);
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
    evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
}

/// Shared body for [`evaluate`] and [`evaluate_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is
/// the only difference between the two routes.
async fn evaluate_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: crate::reactor::BranchReference<'a>,
    body: Bytes,
    query: EvaluateQuery,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;

    log!("Evaluating {} expression(s)", syntax.expressions.len());

    // Acquire the cached branch via the reactor so the same
    // handle is reused across requests and subscription polling
    // sees commits emitted below.
    let branch = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let resolver = BranchResolver {
        branch: branch.handle(),
        operator: &tonk_state.operator,
    };

    let analysis = analyzer::analyze(&syntax, &resolver).await.map_err(|e| {
        log!("Analyzer rejected document: {e}");
        // Preserve structure (code + range) so the editor can
        // attach the diagnostic to the offending source span
        // instead of rendering a banner.
        TonkWorkerError::from(e)
    })?;

    let revision_before = branch.handle().revision();
    let response = run(
        &analysis,
        &syntax,
        branch.handle(),
        tonk_branch,
        &tonk_state.operator,
        query.transact,
    )
    .await?;
    let revision_after = branch.handle().revision();

    Ok(Json(EvaluateResponse {
        revision_before,
        revision_after,
        ..response
    }))
}

/// Drive the analyze → run → plan → commit pipeline. Returns
/// the matches + commit summary; the caller fills in the
/// before/after revisions.
///
/// `transact` controls the commit step: when `true` (the normal
/// case) mutations land on the branch and `matches_after`
/// reflects the post-commit state. When `false` the planning
/// runs (so any plan-level error still surfaces) but the
/// transaction is dropped instead of committed, and
/// `matches_after` reuses the pre-commit results — the route is
/// effectively "what would happen if I ran this?".
async fn run<'a>(
    analysis: &Analysis,
    syntax: &Syntax,
    branch: &Branch,
    tonk_branch: crate::reactor::BranchReference<'a>,
    operator: &DefaultOperator,
    transact: bool,
) -> Result<EvaluateResponse, TonkWorkerError> {
    // ---- Build base bindings frame from analysis-derived vars ----
    let mut base = Parameters::new();
    for (name, entity) in &analysis.variables {
        base.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
    }

    // ---- Per-expression queries + post-join ----
    // Each expression runs its own ConceptQuery. The worker
    // hash-joins their frames on shared user-named variables so
    // disjoint expressions cross-product (independent results)
    // and connected expressions equi-join (filtered intersection).
    //
    // Disjoint queries used to fail because a single unified
    // ConceptQuery has only one `this` slot; merging two
    // expressions collapsed both entities into one.
    let pre_results = match &analysis.query {
        Some(q) => Some(run_query(q, branch, operator).await?),
        None => None,
    };
    let pre_matches: Vec<Parameters> = match &pre_results {
        Some(r) if !r.joined.is_empty() => r.joined.clone(),
        _ => vec![Parameters::new()],
    };

    // ---- Plan + commit each mutation per match frame ----
    let mut commits = CommitSummary::default();
    for (key, entity) in &analysis.declarations {
        commits.entities.insert(key.clone(), entity.to_string());
    }
    for (key, entity) in &analysis.variables {
        commits
            .entities
            .insert(format!("?{key}"), entity.to_string());
    }
    if !analysis.mutate.statements.is_empty() {
        let mut tx = tonk_branch.transaction();
        let mut claim_count = 0usize;
        // Retraction targets resolved by querying the branch
        // *before* the transaction commits — collected here and
        // tx.retract'd in one shot below so we don't interleave
        // queries with transaction building.
        let mut retract_claims: Vec<RawClaim> = Vec::new();
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
                    .map_err(|e| TonkWorkerError::Internal(format!("plan failed: {e}")))?;
                match statement {
                    Statement::Assert(_) => {
                        claim_count += count_emitted_claims(&plan);
                        tx = tx.assert(plan);
                    }
                    Statement::Retract(_) => {
                        // Resolve blank fields by querying the
                        // branch for their current values, then
                        // dissociate each match.
                        let resolved = resolve_retraction_targets(plan, branch, operator).await?;
                        claim_count += resolved.len();
                        retract_claims.extend(resolved);
                    }
                }
            }
        }
        for claim in retract_claims {
            tx = tx.retract(claim);
        }
        if transact {
            tx.commit().perform(operator).await.map_err(|e| {
                log!("Transaction commit failed: {:?}", e);
                TonkWorkerError::Internal(format!("commit failed: {e}"))
            })?;
            commits.claims = claim_count;
        } else {
            // Drop the assembled transaction without committing.
            // We still ran the plan above so any plan-level error
            // surfaced; this just stops the writes from landing.
            drop(tx);
        }
    }

    // Render the pre-commit matches now (before we run the
    // post-commit query) so the response carries both shapes
    // and the editor can show a before/after comparison.
    let matches_before = render_match_blocks(analysis, syntax, pre_results.as_ref());

    // ---- Re-run per-expression queries against post-commit state ----
    // Skip the post-state query in two cases:
    //   1. Pure-query documents — the post-state equals the
    //      pre-state by definition.
    //   2. `transact == false` — we deliberately did not commit,
    //      so the branch state is unchanged and the post-results
    //      mirror the pre-results.
    let post_results = if analysis.mutate.statements.is_empty() || !transact {
        pre_results
    } else {
        match &analysis.query {
            Some(q) => Some(run_query(q, branch, operator).await?),
            None => pre_results,
        }
    };
    let matches_after = render_match_blocks(analysis, syntax, post_results.as_ref());

    Ok(EvaluateResponse {
        revision_before: None,
        revision_after: None,
        matches_before,
        matches_after,
        commits,
    })
}

/// Per-expression query results plus the joined frames for
/// mutation planning.
///
/// Each expression runs its own [`ConceptQuery`] independently;
/// the worker hash-joins frames on shared user-named variables.
/// Disjoint expressions cross-product (no shared variable to
/// constrain on); connected expressions equi-join.
struct QueryResults {
    /// Per-expression frames, in document order. Each frame
    /// carries every user-named variable bound by that
    /// expression's query.
    per_expression: Vec<Vec<Parameters>>,
    /// The natural join of `per_expression`. Used for mutation
    /// planning (one row = one substitution into a [`Statement`]).
    /// Equivalent to the cross-product when no expressions share
    /// variables.
    joined: Vec<Parameters>,
}

/// Run each expression's [`Application`] independently and join
/// their frames on shared user-named variables.
///
/// `Application` impls `dialog_query::Application` and dispatches
/// internally to the right [`QueryPlan`] (built-in or branch
/// concept), so this loop is uniform across head kinds.
async fn run_query(
    query: &tonk_schema::transact::QueryAnalysis,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<QueryResults, TonkWorkerError> {
    let mut per_expression = Vec::with_capacity(query.queries.len());
    for application in &query.queries {
        let frames = collect_matches(application.clone(), branch, operator).await?;
        per_expression.push(frames);
    }
    let joined = natural_join(&per_expression);
    Ok(QueryResults {
        per_expression,
        joined,
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
async fn resolve_retraction_targets(
    plan: ApplicationPlan,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<Vec<RawClaim>, TonkWorkerError> {
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
            .perform(operator)
            .try_vec()
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
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
async fn collect_matches(
    application: tonk_schema::transact::Application,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<Vec<Parameters>, TonkWorkerError> {
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
        .perform(operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("query execution failed: {e:?}")))?;

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
    syntax: &Syntax,
    results: Option<&QueryResults>,
) -> Vec<QueryMatchBlock> {
    let Some(query) = &analysis.query else {
        return Vec::new();
    };
    let Some(results) = results else {
        return Vec::new();
    };

    // Source-expression labels in document order.
    let mut labels: Vec<String> = Vec::new();
    for expression in &syntax.expressions {
        if let tonk_notation::Expression::Query(q) = expression {
            labels.push(q.head.source.clone());
        }
    }

    // For each expression, collect the user-named variables it
    // binds. We project the joined frame onto these to dedupe.
    let mut blocks = Vec::with_capacity(query.queries.len());
    for (i, application) in query.queries.iter().enumerate() {
        let label = labels.get(i).cloned().unwrap_or_else(|| "?".to_owned());
        let descriptor = match application {
            tonk_schema::transact::Application::Concept { query: q, .. } => q.predicate.clone(),
            tonk_schema::transact::Application::Domain { application: d, .. } => {
                ConceptQuery::from(d.clone()).predicate
            }
        };

        // Variable names this expression contributes to the join.
        let mut my_vars: Vec<String> = Vec::new();
        let terms = match application {
            tonk_schema::transact::Application::Concept { query: q, .. } => &q.terms,
            tonk_schema::transact::Application::Domain { application: d, .. } => &d.parameters,
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
        blocks.push(QueryMatchBlock {
            label,
            results: block_results,
        });
    }
    blocks
}

fn render_one_result(
    descriptor: &ConceptDescriptor,
    application: &tonk_schema::transact::Application,
    frame: &Parameters,
) -> QueryResult {
    use tonk_schema::transact::Application;

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

/// Project [`Parsed`] onto a successful syntax or a 400 error
/// carrying the diagnostic messages.
fn surface_parse_diagnostics(parsed: Parsed) -> Result<Syntax, TonkWorkerError> {
    if !parsed.diagnostics.is_empty() {
        let messages = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(TonkWorkerError::Router(messages));
    }
    parsed
        .syntax
        .ok_or_else(|| TonkWorkerError::Router("empty document".to_owned()))
}

/// Branch-backed [`Resolver`] — looks up concepts and attributes
/// against the open branch via `tonk_schema::concept`'s builder
/// family.
struct BranchResolver<'a> {
    branch: &'a Branch,
    operator: &'a DefaultOperator,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<'a> Resolver for BranchResolver<'a> {
    async fn resolve_concept(&self, name: &str) -> Result<Option<ResolvedConcept>, ResolverError> {
        let resolved = ConceptLookup::by_name(name)
            .resolve(self.branch, self.operator)
            .await
            .map_err(|e| ResolverError::new(e.to_string()))?;
        Ok(resolved.map(|c| ResolvedConcept {
            entity: c.entity,
            descriptor: c.descriptor,
        }))
    }

    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        let resolved = AttributeByName::new(name)
            .resolve(self.branch, self.operator)
            .await
            .map_err(|e| ResolverError::new(e.to_string()))?;
        Ok(resolved.map(|a| ResolvedAttribute {
            entity: a.entity,
            descriptor: a.descriptor,
        }))
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        let resolved = AttributeByEntity::new(entity.clone())
            .resolve(self.branch, self.operator)
            .await
            .map_err(|e| ResolverError::new(e.to_string()))?;
        Ok(resolved.map(|a| ResolvedAttribute {
            entity: a.entity,
            descriptor: a.descriptor,
        }))
    }

    /// Find the entity that the user-published name `<name>`
    /// points at — `(id:<name>, dialog.meta/name, ?target)`.
    /// Cardinality-one on `dialog.meta/name` means at most one
    /// target per name, so the first (and only) match is
    /// returned.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolverError> {
        lookup_named_entity(name, self.branch, self.operator)
            .await
            .map_err(|e| ResolverError::new(format!("name lookup failed: {e:?}")))
    }
}
