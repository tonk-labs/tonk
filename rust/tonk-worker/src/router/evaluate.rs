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
    concept::{AttributeByEntity, AttributeByName, Concept as ConceptLookup},
    interpret::{self, ResolvedAttribute, ResolvedConcept, Resolver, ResolverError},
    meta::{Name, Named},
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

/// `POST /api/repository/{repo}/branch/{branch}/evaluate`
///
/// Body: an asserted-notation document — any mix of queries and
/// mutations. Returns query matches and a commit summary in one
/// response.
#[wasm_compat]
pub async fn evaluate(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;

    log!(
        "Evaluating {} expression(s) on repo={}, branch={}",
        syntax.expressions.len(),
        path.repo,
        path.branch,
    );

    let tonk_state = state.write().await;

    // Acquire the cached branch via the reactor so the same
    // handle is reused across requests and subscription polling
    // sees commits emitted below.
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);
    let branch = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let resolver = BranchResolver {
        branch: branch.handle(),
        operator: &tonk_state.operator,
    };

    let analysis = interpret::analyze(&syntax, &resolver).await.map_err(|e| {
        log!("Analyzer rejected document: {e}");
        TonkWorkerError::Router(e.to_string())
    })?;

    let revision_before = branch.handle().revision();
    let response = run(
        &analysis,
        &syntax,
        branch.handle(),
        tonk_branch,
        &tonk_state.operator,
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
async fn run<'a>(
    analysis: &Analysis,
    syntax: &Syntax,
    branch: &Branch,
    tonk_branch: crate::reactor::BranchReference<'a>,
    operator: &DefaultOperator,
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
                        // Cardinality-one fields need prior
                        // values dissociated so the new value
                        // replaces rather than accumulates with
                        // them. Dialog's storage layer is
                        // additive — `associate_unique` only
                        // de-dupes within the current batch, not
                        // against committed state. So query the
                        // branch first.
                        let supersedes =
                            resolve_supersession_targets(&plan, branch, operator).await?;
                        claim_count += count_emitted_claims(&plan);
                        retract_claims.extend(supersedes);
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
        tx.commit().perform(operator).await.map_err(|e| {
            log!("Transaction commit failed: {:?}", e);
            TonkWorkerError::Internal(format!("commit failed: {e}"))
        })?;
        commits.claims = claim_count;
    }

    // Render the pre-commit matches now (before we run the
    // post-commit query) so the response carries both shapes
    // and the editor can show a before/after comparison.
    let matches_before = render_match_blocks(analysis, syntax, pre_results.as_ref());

    // ---- Re-run per-expression queries against post-commit state ----
    // For pure-query documents the post-state equals the
    // pre-state, so reuse `pre_results` to skip the round-trip.
    let post_results = if analysis.mutate.statements.is_empty() {
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

/// Run one [`ConceptQuery`] per expression and join their frames
/// on shared user-named variables.
async fn run_query(
    query: &tonk_schema::transact::QueryAnalysis,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<QueryResults, TonkWorkerError> {
    let mut per_expression = Vec::with_capacity(query.queries.len());
    for cq in query.expression_queries() {
        let frames = collect_matches(cq, branch, operator).await?;
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

/// For an *assert* plan: find prior values of every
/// cardinality-one field on the plan's `this` entity and emit
/// them as `RawClaim`s ready for `tx.retract`. Dialog's storage
/// is additive — without this, re-asserting `age = 30` on Alice
/// leaves `age = 28` and `age = 30` both present, and the
/// engine returns whichever it finds first.
///
/// Cardinality-many fields are skipped (the whole point is
/// multiple values per entity).
async fn resolve_supersession_targets(
    plan: &ApplicationPlan,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<Vec<RawClaim>, TonkWorkerError> {
    use dialog_query::Cardinality;

    let Some(this_term) = plan.statement.terms.get("this") else {
        return Ok(Vec::new());
    };
    let this_entity = match this_term {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    for (field_name, attribute) in plan.statement.predicate.with().iter() {
        if attribute.cardinality() != Cardinality::One {
            continue;
        }
        // Only supersede when the new assert *would* write a
        // concrete value — leaving a blank is an "unset" /
        // "skip" signal, not "retract whatever's there".
        let new_value = match plan.statement.terms.get(field_name) {
            Some(Term::Constant(value)) => value.clone(),
            _ => continue,
        };

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
                    "supersession query failed for ({:?}, {of}): {e:?}",
                    attribute.the(),
                    of = this_entity
                ))
            })?;
        for claim in claims {
            // Skip retracting the value we're about to write —
            // re-asserting the same value is a no-op, and
            // emitting a retract+assert pair for the same
            // (the, of, is) would be churn.
            if claim.is == new_value {
                continue;
            }
            out.push(RawClaim {
                the: claim.the.into(),
                of: this_entity.clone(),
                is: claim.is,
            });
        }
    }
    Ok(out)
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

/// Run the unified [`ConceptQuery`] against the branch and
/// collect every match frame as a [`Parameters`] by extracting
/// every bound variable from each [`ConceptConclusion`].
async fn collect_matches(
    query: ConceptQuery,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<Vec<Parameters>, TonkWorkerError> {
    // Capture the variable names present in `query.terms` so we
    // can ask the conclusion for their bindings.
    let mut variable_names: Vec<String> = Vec::new();
    for (_, term) in query.terms.iter() {
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
        .select(tonk_schema::concept::QueryPlan::from(query))
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
            labels.push(q.head.name_source.clone());
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

    /// Find any branch entity carrying `dialog.meta/name = name`
    /// via the typed [`Named`] concept query. Returns the first
    /// match — name uniqueness is not a schema-level invariant
    /// (two distinct concepts could share a display name), but in
    /// practice the meta branch enforces it via cardinality-one.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolverError> {
        use dialog_query::{Output as _, Query};
        let rows: Vec<Named> = self
            .branch
            .query()
            .select(Query::<Named> {
                this: Term::var("this"),
                name: Term::from(Name(name.to_owned())),
            })
            .perform(self.operator)
            .try_vec()
            .await
            .map_err(|e| ResolverError::new(format!("Named lookup failed: {e:?}")))?;
        Ok(rows.into_iter().next().map(|n| n.this))
    }
}
