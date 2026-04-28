//! Transaction route — accepts an asserted-notation document
//! (JSON or YAML) and commits all derived facts in a single
//! transaction.
//!
//! The body is parsed by `tonk_notation` into a typed
//! [`Syntax`][tonk_notation::Syntax] tree, then handed to
//! `tonk_schema::interpret` along with a [`BranchResolver`] that
//! looks up bookmark references against the open branch via
//! typed `Named` / `AttributeFacts` concept queries. The
//! resulting [`Claim`][tonk_schema::interpret::Claim]s are
//! committed atomically.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use async_trait::async_trait;
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{Attribute, Entity, Statement, Update, Value};
use dialog_query::{AttributeDescriptor, Output as _, Query, Term};
use dialog_repository::{Branch, RepositoryExt as _};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_notation::{Parsed, parse, parse_json};
use tonk_schema::{
    interpret::{self, Claim, ResolvedAttribute, Resolver, ResolverError},
    meta::{AttributeFacts, Name, Named},
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

/// Response from a successful transaction.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactResponse {
    /// Number of EAV claims committed.
    pub claims: usize,
    /// Resolved entity URI for every named subject in the document.
    pub bookmarks: BTreeMap<String, String>,
}

/// A claim ready for dialog's transaction API. Forwards the
/// runtime-determined `(the, of, is)` triple to
/// [`Update::associate`] / [`Update::dissociate`].
struct RawClaim {
    the: Attribute,
    of: Entity,
    is: Value,
}

impl From<Claim> for RawClaim {
    fn from(claim: Claim) -> Self {
        Self {
            the: claim.the,
            of: claim.of,
            is: claim.is,
        }
    }
}

impl Statement for RawClaim {
    fn assert(self, update: &mut impl Update) {
        update.associate(self.the, self.of, self.is);
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

/// `POST /api/repository/{repo}/branch/{branch}/transact`
///
/// Body: an asserted-notation document, encoded as JSON
/// (`Content-Type: application/json`) or YAML (`Content-Type:
/// application/yaml` or `text/yaml`). Empty documents are valid
/// and produce a no-op (zero claims).
#[wasm_compat]
pub async fn transact(
    State(state): State<AppState>,
    Path(path): Path<TransactPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let parsed =
        if content_type.starts_with("application/yaml") || content_type.starts_with("text/yaml") {
            parse(text)
        } else {
            parse_json(text)
        };

    let syntax = surface_parse_diagnostics(parsed)?;

    log!(
        "Transacting {} statement(s) on repo={}, branch={}",
        syntax.statements.len(),
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

    let transaction_data = interpret::interpret(&syntax, &resolver)
        .await
        .map_err(|e| {
            log!("Interpreter rejected transaction: {e}");
            TonkWorkerError::Router(e.to_string())
        })?;

    let bookmarks: BTreeMap<String, String> = transaction_data
        .bookmarks
        .iter()
        .map(|(name, entity)| (name.clone(), entity.to_string()))
        .collect();
    let claim_count = transaction_data.claims.len();

    if transaction_data.claims.is_empty() {
        return Ok(Json(TransactResponse {
            claims: 0,
            bookmarks,
        }));
    }

    let mut transaction = branch.transaction();
    for claim in transaction_data.claims {
        transaction = transaction.assert(RawClaim::from(claim));
    }

    transaction
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            log!("Transaction commit failed: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to commit transaction: {}", e))
        })?;

    Ok(Json(TransactResponse {
        claims: claim_count,
        bookmarks,
    }))
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

/// [`Resolver`] that looks up bookmark names against the open
/// branch using dialog-query's typed concept queries.
struct BranchResolver<'a> {
    branch: &'a Branch,
    operator: &'a DefaultOperator,
}

impl<'a> BranchResolver<'a> {
    /// Run the typed `AttributeFacts` query against `entity` and
    /// reconstruct a [`ResolvedAttribute`]. Shared by both
    /// resolver methods — the difference is just how `entity`
    /// gets discovered (by name lookup vs. supplied directly).
    async fn fetch_attribute(
        &self,
        entity: Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        let facts: Vec<AttributeFacts> = self
            .branch
            .query()
            .select(Query::<AttributeFacts> {
                this: Term::from(entity.clone()),
                id: Term::var("id"),
                r#type: Term::var("type"),
                cardinality: Term::var("cardinality"),
                description: Term::var("description"),
            })
            .perform(self.operator)
            .try_vec()
            .await
            .map_err(|e| ResolverError::new(format!("AttributeFacts query failed: {e:?}")))?;

        let Some(facts) = facts.into_iter().next() else {
            return Ok(None);
        };

        let descriptor = build_descriptor(&facts).map_err(ResolverError::new)?;
        Ok(Some(ResolvedAttribute { entity, descriptor }))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<'a> Resolver for BranchResolver<'a> {
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        // Step 1 — find the named entity. Two `Named` rows with
        // different entities for the same name shouldn't happen
        // under cardinality-one semantics, but be defensive: take
        // the first.
        let named: Vec<Named> = self
            .branch
            .query()
            .select(Query::<Named> {
                this: Term::var("this"),
                name: Term::from(Name(name.to_owned())),
            })
            .perform(self.operator)
            .try_vec()
            .await
            .map_err(|e| ResolverError::new(format!("Named query failed: {e:?}")))?;

        let Some(found) = named.into_iter().next() else {
            return Ok(None);
        };

        // Step 2 — fetch the attribute's full fact set against
        // the resolved entity. Returning `None` here means the
        // entity exists with `dialog.meta/name` but isn't an
        // attribute (no `dialog.attribute/*` claims). The
        // interpreter surfaces that as `UnknownBookmark` with a
        // clearer message, and a non-attribute resolved-by-name
        // is exactly what the user would have seen as a typed
        // mismatch anyway.
        self.fetch_attribute(found.this).await
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        // Direct fact-set query — no `Named` lookup needed.
        // Behaves like `resolve_attribute` for the second half
        // of the pipeline.
        self.fetch_attribute(entity.clone()).await
    }
}

/// Reconstruct a [`AttributeDescriptor`] from its branch facts.
///
/// Round-trips through serde — same trick the interpreter uses
/// for the *write* side, so we don't have to mirror dialog's
/// internal `Type` ↔ string mapping.
fn build_descriptor(facts: &AttributeFacts) -> Result<AttributeDescriptor, String> {
    let mut shape = serde_json::Map::new();
    shape.insert(
        "the".to_owned(),
        serde_json::Value::String(facts.id.0.clone()),
    );
    if !facts.r#type.0.is_empty() {
        shape.insert(
            "as".to_owned(),
            serde_json::Value::String(facts.r#type.0.clone()),
        );
    }
    if !facts.cardinality.0.is_empty() {
        shape.insert(
            "cardinality".to_owned(),
            serde_json::Value::String(facts.cardinality.0.clone()),
        );
    }
    if !facts.description.0.is_empty() {
        shape.insert(
            "description".to_owned(),
            serde_json::Value::String(facts.description.0.clone()),
        );
    }
    serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| format!("could not reconstruct AttributeDescriptor: {e}"))
}
