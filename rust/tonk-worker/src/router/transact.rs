//! Transaction route — accepts an asserted-notation document
//! containing one assertion and commits the derived facts.
//!
//! v1 scope:
//! - Single expression per document, must be an assertion.
//! - Head binding is `Anonymous` (a fresh entity is minted) or
//!   `Uri` (the user supplies an explicit `did:key:…`).
//! - Body fields are literal scalars; variables, blanks,
//!   references, and nested mappings are rejected by the
//!   analyzer.
//! - Concept heads validate against the resolved descriptor;
//!   claim heads accept any field name and synthesize
//!   `<domain>/<field>` attributes.
//!
//! Retractions and bookmark-name binding (which derives an
//! entity from a bookmark name and asserts the binding) land in
//! a follow-up.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use async_trait::async_trait;
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{Attribute, Entity, Statement, Update, Value};
use dialog_repository::{Branch, RepositoryExt as _};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_notation::{Parsed, parse};
use tonk_schema::{
    concept::{AttributeByEntity, AttributeByName, Concept as ConceptLookup},
    interpret::{
        self, Analysis, ClaimAssertion, ResolvedAttribute, ResolvedConcept, Resolver,
        ResolverError, TransactionPlan,
    },
};

use super::AppState;
use crate::{TonkWorkerError, worker::DefaultOperator};

/// Path parameters for the transact route.
#[derive(Debug, Deserialize)]
pub struct TransactPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response shape from a successful transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactResponse {
    /// Number of EAV claims committed.
    pub claims: usize,
    /// Subject → entity URI for every head the transaction
    /// touched. v1 always has exactly one entry (single
    /// expression per document); the map shape leaves room for
    /// multi-statement transactions later.
    pub entities: BTreeMap<String, String>,
}

/// One claim ready for dialog's transaction API. Wraps a
/// `(the, of, is)` triple and forwards the
/// associate / dissociate calls verbatim.
struct RawClaim {
    the: Attribute,
    of: Entity,
    is: Value,
}

impl Statement for RawClaim {
    fn assert(self, update: &mut impl Update) {
        update.associate(self.the, self.of, self.is);
    }
    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

impl From<ClaimAssertion> for RawClaim {
    fn from(assertion: ClaimAssertion) -> Self {
        Self {
            the: assertion.the,
            of: assertion.of,
            is: assertion.is,
        }
    }
}

/// `POST /api/repository/{repo}/branch/{branch}/transact`
///
/// Body: asserted-notation document containing exactly one
/// assertion expression. Returns the committed claim count and
/// the resolved entity URI for the head.
#[wasm_compat]
pub async fn transact(
    State(state): State<AppState>,
    Path(path): Path<TransactPath>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;

    log!(
        "Transacting {} expression(s) on repo={}, branch={}",
        syntax.expressions.len(),
        path.repo,
        path.branch,
    );

    let tonk_state = state.write().await;

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
        log!("Analyzer rejected transaction: {e}");
        TonkWorkerError::Router(e.to_string())
    })?;

    let plan = match analysis {
        Analysis::Transaction(t) => t,
        Analysis::Query(_) => {
            return Err(TonkWorkerError::Router(
                "this endpoint accepts assertions only — \
                 the document parsed as a query (no `!` on the head). \
                 Use the /query endpoint for reads."
                    .to_owned(),
            ));
        }
    };

    let response = commit_transaction(plan, &branch, &tonk_state.operator).await?;

    Ok(Json(response))
}

/// Build a dialog transaction from the analyzer's plan, commit
/// it, and shape the response.
///
/// Retractions go through a query-then-dissociate path: for
/// each `(of, attributes)` target, we query the branch for the
/// current `(the, of, is)` triples matching `(the, of, *)` and
/// emit a dissociate per match. Dialog requires the value to
/// dissociate; we have to materialize it from the branch.
async fn commit_transaction(
    plan: TransactionPlan,
    branch: &Branch,
    operator: &DefaultOperator,
) -> Result<TransactResponse, TonkWorkerError> {
    use dialog_query::{Output as _, Term};

    let head_entity_uri = plan.head_entity.to_string();
    let head_label = plan.head_label.clone();

    // Resolve retraction targets to concrete (the, of, is)
    // triples by querying the branch first. We use
    // `DynamicAttributeQuery::new` directly because it takes
    // `is: Term<Any>` — the value-side `Term<T>` flavours
    // accepted by `the!().of().is()` constrain the value to a
    // specific Scalar type, which is wrong for retraction
    // (any value matches).
    let mut retraction_claims: Vec<RawClaim> = Vec::new();
    for target in &plan.retractions {
        for the in &target.attributes {
            let the_term: dialog_query::attribute::The = the.clone().into();
            let query = dialog_query::AttributeQuery::new(
                Term::from(the_term),
                Term::from(target.of.clone()),
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
                        "retraction query failed for ({the:?}, {of}): {e:?}",
                        of = target.of
                    ))
                })?;
            for claim in claims {
                retraction_claims.push(RawClaim {
                    the: claim.the.into(),
                    of: target.of.clone(),
                    is: claim.is,
                });
            }
        }
    }

    let assertion_count = plan.assertions.len();
    let retraction_count = retraction_claims.len();
    let claim_count = assertion_count + retraction_count;

    let mut tx = branch.transaction();
    for assertion in plan.assertions {
        tx = tx.assert(RawClaim::from(assertion));
    }
    for claim in retraction_claims {
        tx = tx.retract(claim);
    }

    tx.commit().perform(operator).await.map_err(|e| {
        log!("Transaction commit failed: {:?}", e);
        TonkWorkerError::Internal(format!("commit failed: {e:?}"))
    })?;

    let mut entities = BTreeMap::new();
    entities.insert(head_label, head_entity_uri);

    Ok(TransactResponse {
        claims: claim_count,
        entities,
    })
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
/// the [`tonk_schema::concept`] builder family. Mirrors the
/// resolver in the query route — both endpoints need the same
/// concept-by-name path.
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

    async fn resolve_variable(
        &self,
        _name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        // Document-scope variables don't outlive a document;
        // the in-document `DocumentResolver` handles them.
        Ok(None)
    }

    async fn resolve_entity_variable(&self, _name: &str) -> Result<Option<Entity>, ResolverError> {
        Ok(None)
    }
}
