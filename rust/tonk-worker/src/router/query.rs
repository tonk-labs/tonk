//! Query route — accepts an asserted-notation query document
//! and returns matching facts.
//!
//! v0 scope: a single query expression per document. The body is
//! parsed by [`tonk_notation`] into a [`Syntax`][tonk_notation::Syntax]
//! tree, analyzed by [`tonk_schema::interpret::analyze`] (which
//! resolves the head concept against the branch via
//! [`BranchResolver`]), and executed via dialog-query's runtime
//! [`ConceptDescriptor::apply`] path. Results come back as
//! [`ConceptConclusion`]s and are rendered as a sequence of
//! entity-keyed YAML blocks.
//!
//! Assertions and retractions return 501 — transactions land in
//! a follow-up.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
};
use async_trait::async_trait;
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{Entity, Value};
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::{ConceptDescriptor, Output as _, Parameters, Proposition, Term};
use dialog_repository::{Branch, RepositoryExt as _};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_notation::{Parsed, Scalar, parse};
use tonk_schema::{
    concept::{AttributeByEntity, Concept as ConceptLookup},
    interpret::{
        self, Analysis, ParameterValue, ResolvedAttribute, ResolvedConcept, Resolver, ResolverError,
    },
};

use super::AppState;
use crate::{TonkWorkerError, worker::DefaultOperator};

/// Path parameters for the query route.
#[derive(Debug, Deserialize)]
pub struct QueryPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response from a successful query.
///
/// One [`QueryResult`] per matching entity. Field values are
/// serialised as JSON-compatible primitives (strings, numbers,
/// bools); entity references serialise as their canonical
/// `did:key:…` URI strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResultEnvelope {
    /// Number of matching entities.
    pub count: usize,
    /// One block per matching entity.
    pub results: Vec<QueryResult>,
}

/// One match — an entity plus its bound field values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    /// Canonical entity URI for the match.
    pub this: String,
    /// Field name → bound value, in the concept's defined order.
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// `POST /api/repository/{repo}/branch/{branch}/query`
///
/// Body: an asserted-notation document containing exactly one
/// query expression. Returns matching entities and their bound
/// fields.
#[wasm_compat]
pub async fn query(
    State(state): State<AppState>,
    Path(path): Path<QueryPath>,
    body: Bytes,
) -> Result<Json<QueryResultEnvelope>, TonkWorkerError> {
    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;

    log!(
        "Querying {} expression(s) on repo={}, branch={}",
        syntax.expressions.len(),
        path.repo,
        path.branch,
    );

    let tonk_state = state.read().await;

    let repo = tonk_state
        .profile
        .repository(&path.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", path.repo, e))
        })?;

    let branch = repo
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", path.branch, e))
        })?;

    let resolver = BranchResolver {
        branch: &branch,
        operator: &tonk_state.operator,
    };

    let analysis = interpret::analyze(&syntax, &resolver).await.map_err(|e| {
        log!("Analyzer rejected query: {e}");
        TonkWorkerError::Router(e.to_string())
    })?;

    let plan = match analysis {
        Analysis::Query(q) => q,
        Analysis::Transaction(_) => {
            return Err(TonkWorkerError::Router(
                "this endpoint accepts queries only — \
                 the document parsed as an assertion (`!` on the head). \
                 Use the /transact endpoint for writes."
                    .to_owned(),
            ));
        }
    };

    let envelope = run_query(&plan, &branch, &tonk_state.operator).await?;

    Ok(Json(envelope))
}

/// Build dialog-query parameters from the analyzer's plan, run
/// the descriptor query, and shape results into a
/// [`QueryResultEnvelope`].
async fn run_query(
    plan: &interpret::QueryPlan,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<QueryResultEnvelope, TonkWorkerError> {
    // Build dialog-query Parameters from the analyzer's per-field
    // bindings, plus a side-map (field name → Term we passed in)
    // we'll use at render time to recover bindings without
    // needing access to ConceptConclusion's private `terms`.
    //
    // Variables become named Term::var() so result bindings are
    // recoverable; blanks become Term::blank(); literals become
    // Term::Constant via per-type conversions.
    let mut parameters = Parameters::new();
    let mut field_terms: BTreeMap<String, Term<dialog_query::Any>> = BTreeMap::new();
    for binding in &plan.parameters {
        let term: Term<dialog_query::Any> = match &binding.value {
            ParameterValue::Variable(name) => Term::<dialog_query::Any>::var(name),
            ParameterValue::Blank => Term::<dialog_query::Any>::blank(),
            ParameterValue::Literal(scalar) => scalar_to_term(scalar)?,
        };
        parameters.insert(binding.name.clone(), term.clone());
        field_terms.insert(binding.name.clone(), term);
    }

    // dialog-query requires a `this` parameter naming the entity
    // being matched. Use the head's variable name when the user
    // gave one (`person ?alice` → `Term::var("alice")`); otherwise
    // generate a stable name so result extraction can find it.
    let this_var_name = plan
        .head_variable
        .clone()
        .unwrap_or_else(|| "this".to_owned());
    parameters.insert(
        "this".to_owned(),
        Term::<dialog_query::Any>::var(&this_var_name),
    );

    // `apply()` returns a `Proposition::Concept(ConceptQuery)`.
    // We don't want the wider Proposition type — `select()` needs
    // an `Application`, and `ConceptQuery` is one. Pattern-match
    // through to the inner query.
    let proposition = plan.descriptor.apply(parameters).map_err(|e| {
        TonkWorkerError::Router(format!("query parameters do not match concept: {e}"))
    })?;
    let concept_query: ConceptQuery = match proposition {
        Proposition::Concept(q) => q,
        _ => {
            return Err(TonkWorkerError::Internal(
                "concept descriptor produced a non-Concept proposition".into(),
            ));
        }
    };

    let conclusions: Vec<ConceptConclusion> = branch
        .query()
        .select(concept_query)
        .perform(operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("query execution failed: {e:?}")))?;

    let mut results = Vec::with_capacity(conclusions.len());
    for conclusion in conclusions {
        results.push(render_conclusion(
            &plan.descriptor,
            &field_terms,
            conclusion,
        )?);
    }

    Ok(QueryResultEnvelope {
        count: results.len(),
        results,
    })
}

/// Convert a notation scalar into a `Term<Any>` constant.
///
/// Routes every integer through `i64` since dialog-query exposes
/// `From<i64>` but not the wider widths; overflow is surfaced as
/// an explicit error rather than silent truncation.
fn scalar_to_term(scalar: &Scalar) -> Result<Term<dialog_query::Any>, TonkWorkerError> {
    let term = match scalar {
        Scalar::String(s) => Term::<String>::from(s.clone()).into(),
        Scalar::Boolean(b) => Term::<bool>::from(*b).into(),
        Scalar::Integer(i) => {
            let v = i64::try_from(*i).map_err(|_| {
                TonkWorkerError::Router(format!("integer literal {i} doesn't fit in i64"))
            })?;
            Term::<i64>::from(v).into()
        }
        Scalar::UnsignedInteger(u) => {
            let v = i64::try_from(*u).map_err(|_| {
                TonkWorkerError::Router(format!("unsigned integer literal {u} doesn't fit in i64"))
            })?;
            Term::<i64>::from(v).into()
        }
        Scalar::Float(f) => Term::<f64>::from(*f).into(),
        Scalar::Null => {
            return Err(TonkWorkerError::Router(
                "null literals aren't supported as query parameters".into(),
            ));
        }
    };
    Ok(term)
}

/// Render one [`ConceptConclusion`] as a [`QueryResult`].
///
/// Walks the descriptor's `with` map in order. For each field,
/// looks up the corresponding term (variable or constant) in
/// the side-map and asks the conclusion's underlying [`Match`]
/// for the resolved value. Blank-bound fields and fields the
/// engine couldn't unify are silently omitted rather than
/// emitting explicit nulls.
fn render_conclusion(
    descriptor: &ConceptDescriptor,
    field_terms: &BTreeMap<String, Term<dialog_query::Any>>,
    conclusion: ConceptConclusion,
) -> Result<QueryResult, TonkWorkerError> {
    let entity = conclusion.entity().to_string();
    let mut fields = BTreeMap::new();
    let source = conclusion.source();
    for (field_name, _attribute) in descriptor.with().iter() {
        let Some(term) = field_terms.get(field_name) else {
            continue;
        };
        if let Ok(value) = source.lookup(term) {
            fields.insert(field_name.to_owned(), value_to_json(&value));
        }
    }
    Ok(QueryResult {
        this: entity,
        fields,
    })
}

/// Convert a dialog `Value` into a JSON value for the response.
/// Best-effort: strings round-trip directly, numerics become JSON
/// numbers, entities become their URI strings.
fn value_to_json(value: &Value) -> serde_json::Value {
    // `Value` serialises through serde — round-trip is the path
    // of least drift since dialog already defines what each
    // variant looks like as JSON.
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

/// Project [`Parsed`] onto a successful syntax or a 400 error
/// carrying the diagnostic messages.
fn surface_parse_diagnostics(parsed: Parsed) -> Result<tonk_notation::Syntax, TonkWorkerError> {
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

/// [`Resolver`] that looks up names against the open branch via
/// the [`tonk_schema::concept`] builder family. The route owns
/// the `Branch` + operator references; this trait implementation
/// is just a thin adapter that wraps each lookup's return type
/// into the analyzer's [`ResolvedConcept`] / [`ResolvedAttribute`].
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
        _name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        // Bookmark-name → attribute lookup isn't part of the v0
        // query path (concept fields don't yet resolve bookmark
        // references); the transaction follow-up will need this,
        // and a sibling builder
        // `tonk_schema::concept::AttributeByName` will land then.
        Ok(None)
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
}
