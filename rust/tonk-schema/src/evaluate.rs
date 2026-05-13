//! Shared analyze → query → plan → commit driver for asserted-notation.
//!
//! Both the worker's `POST /evaluate` route and the slide CLI run the
//! same pipeline against an open dialog branch:
//!
//! 1. Analyze the parsed [`Syntax`] into an [`Analysis`].
//! 2. Run each query expression as an `Application`; hash-join their
//!    frames on shared user-named variables.
//! 3. For each joined frame, plan every mutation `Statement` against
//!    `analysis.variables ∪ frame`.
//! 4. Commit the assert/retract pairs to the branch.
//! 5. Re-run the queries against post-commit state so the response
//!    carries before/after match views.
//!
//! Callers wrap the returned [`EvaluateOutcome`] with whatever
//! envelope they need: the worker emits `Json(response)` and triggers
//! a subscription re-poll when `committed`; slide renders the
//! response as YAML or JSON.

use std::collections::BTreeMap;

use async_trait::async_trait;
use dialog_artifacts::{Entity, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::{ConceptDescriptor, ConceptQuery, Output as _, Parameters, Term};
use dialog_repository::{Branch, RemoteSite, Revision};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_notation::Syntax;

use crate::analyzer;
use crate::concept::{
    AttributeByEntity, AttributeByName, Concept as ConceptLookup, lookup_named_entity,
};
use crate::transact::{
    Analysis, Application, ApplicationPlan, Planner as _, QueryAnalysis, Statement,
};

// ---------------------------------------------------------------- //
// Public response types                                            //
// ---------------------------------------------------------------- //

/// Full result of [`run`] — the on-the-wire shape the worker
/// returns and the structured shape slide consumes.
///
/// Carries the branch revision before and after the commit (one
/// before/after pair — every match's mutations land in the same
/// dialog transaction), the per-source-expression query matches
/// in both pre- and post-commit shape, and a commit summary.
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

/// What [`run`] returns — the response plus a flag the caller can
/// use to decide whether downstream side-effects (e.g. the
/// worker's subscription re-poll) need to fire.
#[derive(Debug, Clone)]
pub struct EvaluateOutcome {
    /// Response payload.
    pub response: EvaluateResponse,
    /// `true` iff a dialog commit was attempted (i.e. the
    /// document carried at least one mutation `Statement`). The
    /// worker uses this to decide whether to re-poll branch
    /// subscriptions.
    pub committed: bool,
}

// ---------------------------------------------------------------- //
// Errors                                                           //
// ---------------------------------------------------------------- //

/// Failure modes for [`run`]. Callers map these onto whatever
/// envelope they expose (HTTP status, CLI exit code).
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
    /// The dialog commit itself failed.
    #[error("{0}")]
    Commit(String),
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
// Branch-backed Resolver                                           //
// ---------------------------------------------------------------- //

/// [`Resolver`] backed by an open dialog branch — looks up
/// concepts and attributes via [`crate::concept`]'s builder
/// family. Exposed publicly so callers don't have to reimplement
/// the same four lookups every time they want to drive
/// [`analyzer::analyze`].
pub struct BranchResolver<'a, Env> {
    /// Open branch the lookups query against.
    pub branch: &'a Branch,
    /// Env capable of running dialog queries.
    pub env: &'a Env,
}

// `BranchResolver` no longer implements `Resolver` directly —
// the blanket `impl<T: BranchIntrospection> Resolver for T` in
// `analyzer::resolver` provides it via the introspection
// implementation below.

// ---------------------------------------------------------------- //
// Branch-backed BranchIntrospection                                //
// ---------------------------------------------------------------- //
//
// Same lookups as the `Resolver` impl above plus enumeration —
// `list_concepts` and `list_named_entities`. Both rely on
// branch-side helpers in `crate::concept`. The blanket
// `Resolver`-from-`BranchIntrospection` impl below means a
// downstream type only needs to implement this trait once and
// gets the legacy `Resolver` interface for free.

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<'a, Env: EvaluateEnv> tonk_introspect::BranchIntrospection for BranchResolver<'a, Env> {
    async fn lookup_concept(
        &self,
        name: &str,
    ) -> Result<Option<tonk_introspect::ResolvedConcept>, tonk_introspect::IntrospectionError> {
        let resolved = ConceptLookup::by_name(name)
            .resolve(self.branch, self.env)
            .await
            .map_err(|e| tonk_introspect::IntrospectionError::new(e.to_string()))?;
        Ok(resolved.map(|c| tonk_introspect::ResolvedConcept {
            entity: c.entity,
            descriptor: c.descriptor,
        }))
    }

    async fn lookup_attribute(
        &self,
        name: &str,
    ) -> Result<Option<tonk_introspect::ResolvedAttribute>, tonk_introspect::IntrospectionError>
    {
        let resolved = AttributeByName::new(name)
            .resolve(self.branch, self.env)
            .await
            .map_err(|e| tonk_introspect::IntrospectionError::new(e.to_string()))?;
        Ok(resolved.map(|a| tonk_introspect::ResolvedAttribute {
            entity: a.entity,
            descriptor: a.descriptor,
        }))
    }

    async fn lookup_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<tonk_introspect::ResolvedAttribute>, tonk_introspect::IntrospectionError>
    {
        let resolved = AttributeByEntity::new(entity.clone())
            .resolve(self.branch, self.env)
            .await
            .map_err(|e| tonk_introspect::IntrospectionError::new(e.to_string()))?;
        Ok(resolved.map(|a| tonk_introspect::ResolvedAttribute {
            entity: a.entity,
            descriptor: a.descriptor,
        }))
    }

    async fn lookup_named_entity(
        &self,
        name: &str,
    ) -> Result<Option<Entity>, tonk_introspect::IntrospectionError> {
        lookup_named_entity(name, self.branch, self.env)
            .await
            .map_err(|e| {
                tonk_introspect::IntrospectionError::new(format!("name lookup failed: {e:?}"))
            })
    }

    /// Enumerate every concept on the branch. Built-ins from
    /// [`crate::builtin::concept_registry`] always lead the list;
    /// branch-published concepts (entities carrying the
    /// `dialog.meta/concept = db:concept` marker) follow with
    /// their reconstructed descriptors. Filtering by published
    /// name happens at the call site — the introspection trait
    /// returns the full set so the consumer can decide what to
    /// surface (e.g. completion may want both built-in and
    /// branch concepts; docs generation may want only branch).
    async fn list_concepts(
        &self,
    ) -> Result<Vec<tonk_introspect::ResolvedConcept>, tonk_introspect::IntrospectionError> {
        use crate::builtin::concept_registry;
        use dialog_query::Output as _;

        let mut out: Vec<tonk_introspect::ResolvedConcept> = Vec::new();

        for (_name, resolved) in concept_registry().iter() {
            out.push(tonk_introspect::ResolvedConcept {
                entity: resolved.entity.clone(),
                descriptor: resolved.descriptor.clone(),
            });
        }

        // Find every entity carrying the concept marker —
        // `(?of, dialog.meta/concept, db:concept)`.
        let marker_attr: dialog_query::attribute::The = "dialog.meta/concept"
            .parse()
            .expect("dialog.meta/concept is a valid attribute URI");
        let marker_target: Entity = "db:concept"
            .parse()
            .expect("`db:concept` is a valid entity URI");
        let claims = self
            .branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(marker_attr)
                    .of(Term::<Entity>::var("__list_concepts_of"))
                    .is(Term::from(marker_target)),
            ))
            .perform(self.env)
            .try_vec()
            .await
            .map_err(|e| {
                tonk_introspect::IntrospectionError::new(format!(
                    "concept marker query failed: {e:?}",
                ))
            })?;

        for claim in claims {
            let entity = claim.of.clone();
            // Reuse the existing branch-side builder rather
            // than reaching into `concept.rs` private helpers.
            let resolved = ConceptLookup::by_entity(entity.clone())
                .resolve(self.branch, self.env)
                .await
                .map_err(|e| {
                    tonk_introspect::IntrospectionError::new(format!(
                        "descriptor reconstruction failed: {e:?}",
                    ))
                })?;
            if let Some(c) = resolved {
                out.push(tonk_introspect::ResolvedConcept {
                    entity: c.entity,
                    descriptor: c.descriptor,
                });
            }
        }

        Ok(out)
    }

    /// Enumerate every published name on the branch — `(id:<n>,
    /// dialog.name/referent, ?target)` claims projected into
    /// `(name, target)` pairs. Reuses the [`crate::meta::Name`]
    /// concept's derived query for the actual fetch.
    async fn list_named_entities(
        &self,
    ) -> Result<Vec<tonk_introspect::NamedEntity>, tonk_introspect::IntrospectionError> {
        use crate::meta::Name;
        use dialog_query::{Output as _, Query};

        let rows: Vec<Name> = self
            .branch
            .query()
            .select(Query::<Name> {
                this: Term::<Entity>::var("__list_names_this"),
                entity: Term::<Entity>::var("__list_names_target"),
            })
            .perform(self.env)
            .try_vec()
            .await
            .map_err(|e| {
                tonk_introspect::IntrospectionError::new(format!("name enumeration failed: {e:?}",))
            })?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let uri = row.this.to_string();
            // `id:<name>` is the user-published-name shape;
            // skip anything else (built-in `db:<name>` URIs,
            // direct DIDs that happen to carry a referent
            // claim, etc.) — those aren't reachable through the
            // bare-symbol notation, so suggesting them as
            // completion targets would mislead.
            let Some(name) = uri.strip_prefix("id:") else {
                continue;
            };
            out.push(tonk_introspect::NamedEntity {
                name: name.to_owned(),
                entity: row.entity.0,
            });
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- //
// Public entry point                                               //
// ---------------------------------------------------------------- //

/// Drive analyze → query → plan → commit → re-query against an
/// open dialog branch. Captures the branch revision before and
/// after the commit so the response carries a snapshot pair.
///
/// `transact` controls whether mutation statements actually
/// commit. With `transact = false` the planner runs (so
/// plan-time errors still surface) but the dialog transaction
/// is dropped instead of committed — used by the editor's
/// auto-evaluate to project what *would* happen after a
/// keystroke without applying it.
///
/// Caller is responsible for parsing the source text into
/// [`Syntax`] (callers want different parse-diagnostic surfaces
/// — HTTP 400 body vs. CLI `<source>:<line>:<col>:` lines — and
/// each owns its own surfacing logic).
pub async fn run<Env: EvaluateEnv>(
    syntax: &Syntax,
    branch: &Branch,
    env: &Env,
    transact: bool,
) -> Result<EvaluateOutcome, EvaluateError> {
    let resolver = BranchResolver { branch, env };
    let analysis = analyzer::analyze(syntax, &resolver)
        .await
        .map_err(EvaluateError::Analyze)?;

    let revision_before = branch.revision();
    let (response, committed) = run_pipeline(&analysis, branch, env, transact).await?;
    let revision_after = branch.revision();

    Ok(EvaluateOutcome {
        response: EvaluateResponse {
            revision_before,
            revision_after,
            ..response
        },
        committed,
    })
}

// ---------------------------------------------------------------- //
// Pipeline                                                         //
// ---------------------------------------------------------------- //

/// Drive the analyze → run → plan → commit pipeline. Returns the
/// matches + commit summary plus a `committed` flag (true iff a
/// dialog commit was attempted); the caller fills in the
/// before/after revisions.
async fn run_pipeline<Env: EvaluateEnv>(
    analysis: &Analysis,
    branch: &Branch,
    env: &Env,
    transact: bool,
) -> Result<(EvaluateResponse, bool), EvaluateError> {
    // ---- Build base bindings frame from analysis-derived vars ----
    let mut base = Parameters::new();
    for (name, entity) in &analysis.variables {
        base.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
    }

    // ---- Per-expression queries + post-join ----
    // Each expression runs its own ConceptQuery. The driver
    // hash-joins their frames on shared user-named variables so
    // disjoint expressions cross-product (independent results)
    // and connected expressions equi-join (filtered intersection).
    //
    // Disjoint queries used to fail because a single unified
    // ConceptQuery has only one `this` slot; merging two
    // expressions collapsed both entities into one.
    let pre_results = match &analysis.query {
        Some(q) => Some(run_query(q, branch, env).await?),
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
    // We only build a transaction at all when there are
    // mutation statements *and* the caller wants them
    // committed. `committed` reports whether a commit was
    // actually attempted — `transact = false` callers (the
    // editor's auto-evaluate) get the rendered matches without
    // any branch state change.
    let committed = transact && !analysis.mutate.statements.is_empty();
    if committed {
        let mut tx = branch.transaction();
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
                    .map_err(|e| EvaluateError::Plan(format!("plan failed: {e}")))?;
                match statement {
                    Statement::Assert(_) => {
                        claim_count += count_emitted_claims(&plan);
                        tx = tx.assert(plan);
                    }
                    Statement::Retract(_) => {
                        // Resolve blank fields by querying the
                        // branch for their current values, then
                        // dissociate each match.
                        let resolved = resolve_retraction_targets(plan, branch, env).await?;
                        claim_count += resolved.len();
                        retract_claims.extend(resolved);
                    }
                }
            }
        }
        for claim in retract_claims {
            tx = tx.retract(claim);
        }
        tx.commit()
            .perform(env)
            .await
            .map_err(|e| EvaluateError::Commit(format!("commit failed: {e}")))?;
        commits.claims = claim_count;
    }

    // Render the pre-commit matches now (before we run the
    // post-commit query) so the response carries both shapes
    // and the editor can show a before/after comparison.
    let matches_before = render_match_blocks(analysis, pre_results.as_ref());

    // ---- Re-run per-expression queries against post-commit state ----
    // For pure-query documents the post-state equals the
    // pre-state, so reuse `pre_results` to skip the round-trip.
    let post_results = if analysis.mutate.statements.is_empty() {
        pre_results
    } else {
        match &analysis.query {
            Some(q) => Some(run_query(q, branch, env).await?),
            None => pre_results,
        }
    };
    let matches_after = render_match_blocks(analysis, post_results.as_ref());

    Ok((
        EvaluateResponse {
            revision_before: None,
            revision_after: None,
            matches_before,
            matches_after,
            commits,
        },
        committed,
    ))
}

/// Per-expression query results plus the joined frames for
/// mutation planning.
///
/// Each expression runs its own [`ConceptQuery`] independently;
/// the driver hash-joins frames on shared user-named variables.
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

    // For each expression, collect the user-named variables it
    // binds. We project the joined frame onto these to dedupe.
    // Labels come from `analysis.query.labels` — populated by
    // the analyzer for both explicit query expressions and the
    // implicit queries it synthesizes for assertions, so the
    // assertion path's result block is titled by the head name
    // (`person`) instead of the legacy `?` fallback.
    let mut blocks = Vec::with_capacity(query.queries.len());
    for (i, application) in query.queries.iter().enumerate() {
        let label = query
            .labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| "?".to_owned());
        let descriptor = match application {
            Application::Concept { query: q, .. } => q.predicate.clone(),
            Application::Domain { application: d, .. } => ConceptQuery::from(d.clone()).predicate,
        };

        // Variable names this expression contributes to the join.
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
