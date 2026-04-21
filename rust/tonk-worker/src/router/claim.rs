//! Claim assertion, retraction, and query routes.
//!
//! These endpoints provide low-level access to the artifact store through
//! the dialog-repository transaction and query APIs.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum::extract::Query as AxumQuery;
use axum_wasm_macros::wasm_compat;
use base64::Engine;
use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Statement, Update, Value};
use dialog_repository::RepositoryExt as _;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for claim assertion/retraction.
#[derive(Debug, Deserialize)]
pub struct AssertPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
    /// The entity identifier.
    pub entity: String,
    /// The attribute namespace.
    pub attr_ns: String,
    /// The attribute name.
    pub attr_name: String,
}

/// Query parameters for claim queries.
#[derive(Debug, Deserialize)]
pub struct ClaimQuery {
    /// The attribute to query (e.g., "namespace/name").
    pub the: Option<String>,
    /// The entity to query.
    pub of: Option<String>,
}

/// Path parameters for select endpoint.
#[derive(Debug, Deserialize)]
pub struct SelectPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response for claim assertion/retraction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssertResponse {
    /// Whether the operation succeeded.
    pub success: bool,
    /// The entity that was asserted/retracted.
    pub entity: String,
    /// The attribute that was asserted/retracted.
    pub attribute: String,
}

/// A single claim in the query response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimResponse {
    /// The attribute.
    pub the: String,
    /// The entity.
    pub of: String,
    /// The value.
    pub is: serde_json::Value,
}

/// Response for claim query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResponse {
    /// The claims that matched the query.
    pub claims: Vec<ClaimResponse>,
}

/// A raw claim with dynamic attribute/entity/value for use with the transaction API.
///
/// This implements `Statement` by forwarding to `Update::associate`/`dissociate`,
/// allowing us to use runtime-determined attribute names and `Value` directly.
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

/// Parse a value from the request body based on content type.
fn parse_value(content_type: Option<&str>, body: &[u8]) -> Result<Value, TonkWorkerError> {
    match content_type {
        Some(ct) if ct.starts_with("text/plain") => {
            let text = String::from_utf8(body.to_vec())
                .map_err(|e| TonkWorkerError::Internal(format!("Invalid UTF-8: {}", e)))?;
            Ok(Value::String(text))
        }
        Some(ct) if ct.starts_with("application/json") => {
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|e| TonkWorkerError::Internal(format!("Invalid JSON: {}", e)))?;
            json_to_value(json)
        }
        Some(ct) => {
            log!("Unknown content type '{}', storing as bytes", ct);
            Ok(Value::Bytes(body.to_vec()))
        }
        None => Ok(Value::Bytes(body.to_vec())),
    }
}

/// Convert a JSON value to a dialog-db Value.
fn json_to_value(json: serde_json::Value) -> Result<Value, TonkWorkerError> {
    match json {
        serde_json::Value::Null => Ok(Value::String("null".to_string())),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Ok(Value::UnsignedInt(i as u128))
                } else {
                    Ok(Value::SignedInt(i as i128))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err(TonkWorkerError::Internal("Invalid number".to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s)),
        // For arrays and objects, store as JSON string
        other => Ok(Value::String(other.to_string())),
    }
}

/// Convert a dialog-db Value to a JSON value.
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::SignedInt(i) => {
            if *i >= i64::MIN as i128 && *i <= i64::MAX as i128 {
                serde_json::Value::Number((*i as i64).into())
            } else {
                serde_json::Value::String(i.to_string())
            }
        }
        Value::UnsignedInt(u) => {
            if *u <= u64::MAX as u128 {
                serde_json::Value::Number((*u as u64).into())
            } else {
                serde_json::Value::String(u.to_string())
            }
        }
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Bytes(b) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(b);
            serde_json::Value::String(encoded)
        }
        Value::Entity(e) => serde_json::Value::String(e.to_string()),
        Value::Record(r) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(r);
            serde_json::Value::String(encoded)
        }
        Value::Symbol(s) => serde_json::Value::String(s.to_string()),
    }
}

/// Handles claim assertion requests.
///
/// POST /api/repository/{repo}/branch/{branch}/claim/assert/{entity}/{attr_ns}/{attr_name}
#[wasm_compat]
pub async fn assert_claim(
    State(state): State<AppState>,
    Path(path): Path<AssertPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AssertResponse>, TonkWorkerError> {
    let attribute_str = format!("{}/{}", path.attr_ns, path.attr_name);
    log!(
        "Asserting claim: repo={}, branch={}, entity={}, attribute={}",
        path.repo,
        path.branch,
        path.entity,
        attribute_str
    );

    // Parse entity
    let entity: Entity = path.entity.parse().map_err(|e| {
        TonkWorkerError::Internal(format!("Invalid entity '{}': {}", path.entity, e))
    })?;

    // Parse attribute
    let attribute: Attribute = attribute_str
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("Invalid attribute: {}", e)))?;

    // Get content type and parse value
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok());
    let value = parse_value(content_type, &body)?;

    // Build and commit the assertion using the transaction API
    let tonk_state = state.write().await;

    let repo = tonk_state
        .profile
        .repository(&path.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to load repository '{}': {}", path.repo, e))
        })?;

    let branch = repo
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", path.branch, e))
        })?;

    let claim = RawClaim {
        the: attribute,
        of: entity,
        is: value,
    };

    branch
        .transaction()
        .assert(claim)
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            log!("Failed to assert claim: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to assert claim: {}", e))
        })?;

    log!("Claim asserted successfully");

    Ok(Json(AssertResponse {
        success: true,
        entity: path.entity,
        attribute: attribute_str,
    }))
}

/// Handles claim retraction requests.
///
/// POST /api/repository/{repo}/branch/{branch}/claim/retract/{entity}/{attr_ns}/{attr_name}
#[wasm_compat]
pub async fn retract_claim(
    State(state): State<AppState>,
    Path(path): Path<AssertPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AssertResponse>, TonkWorkerError> {
    let attribute_str = format!("{}/{}", path.attr_ns, path.attr_name);
    log!(
        "Retracting claim: repo={}, branch={}, entity={}, attribute={}",
        path.repo,
        path.branch,
        path.entity,
        attribute_str
    );

    // Parse entity
    let entity: Entity = path.entity.parse().map_err(|e| {
        TonkWorkerError::Internal(format!("Invalid entity '{}': {}", path.entity, e))
    })?;

    // Get content type and parse value
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok());
    let value = parse_value(content_type, &body)?;

    // Build and commit the retraction using the transaction API
    let tonk_state = state.write().await;

    let repo = tonk_state
        .profile
        .repository(&path.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to load repository '{}': {}", path.repo, e))
        })?;

    let branch = repo
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", path.branch, e))
        })?;

    // Parse attribute
    let attribute: Attribute = attribute_str
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("Invalid attribute: {}", e)))?;

    let claim = RawClaim {
        the: attribute,
        of: entity,
        is: value,
    };

    branch
        .transaction()
        .retract(claim)
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            log!("Failed to retract claim: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to retract claim: {}", e))
        })?;

    log!("Claim retracted successfully");

    Ok(Json(AssertResponse {
        success: true,
        entity: path.entity,
        attribute: attribute_str,
    }))
}

/// Handles claim query requests.
///
/// GET /api/repository/{repo}/branch/{branch}/claim/select?the=namespace/name&of=entity
#[wasm_compat]
pub async fn select_claims(
    State(state): State<AppState>,
    Path(params): Path<SelectPath>,
    AxumQuery(query): AxumQuery<ClaimQuery>,
) -> Result<Json<QueryResponse>, TonkWorkerError> {
    log!(
        "Querying claims: repo={}, branch={}, the={:?}, of={:?}",
        params.repo,
        params.branch,
        query.the,
        query.of,
    );

    // At least one constraint is required
    if query.the.is_none() && query.of.is_none() {
        return Err(TonkWorkerError::Internal(
            "At least one of 'the' or 'of' must be specified".to_string(),
        ));
    }

    let tonk_state = state.read().await;

    let repo = tonk_state
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "Failed to load repository '{}': {}",
                params.repo, e
            ))
        })?;

    let branch = repo
        .branch(params.branch.as_str())
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to load branch '{}': {}", params.branch, e))
        })?;

    // Build the constrained artifact selector.
    // ArtifactSelector transitions from Unconstrained -> Constrained on the first constraint.
    // We need at least one constraint (validated above).
    let selector = ArtifactSelector::new();

    // Parse optional attribute
    let attribute: Option<Attribute> = match &query.the {
        Some(attr) => {
            if !attr.contains('/') {
                return Err(TonkWorkerError::Internal(format!(
                    "Invalid attribute '{}': must be in 'namespace/name' format",
                    attr
                )));
            }
            Some(attr.parse().map_err(|e| {
                TonkWorkerError::Internal(format!("Invalid attribute '{}': {}", attr, e))
            })?)
        }
        None => None,
    };

    // Parse optional entity
    let entity: Option<Entity> = match &query.of {
        Some(entity_str) => Some(entity_str.parse().map_err(|e| {
            TonkWorkerError::Internal(format!("Invalid entity '{}': {}", entity_str, e))
        })?),
        None => None,
    };

    // Build the constrained selector based on which params are present
    let constrained = match (attribute, entity) {
        (Some(attr), Some(ent)) => selector.the(attr).of(ent),
        (Some(attr), None) => selector.the(attr),
        (None, Some(ent)) => selector.of(ent),
        (None, None) => unreachable!("validated above"),
    };

    let stream = branch
        .claims()
        .select(constrained)
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Query execution error: {}", e)))?;

    tokio::pin!(stream);

    let mut claims = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(artifact) => {
                claims.push(ClaimResponse {
                    the: artifact.the.to_string(),
                    of: artifact.of.to_string(),
                    is: value_to_json(&artifact.is),
                });
            }
            Err(e) => {
                log!("Error reading artifact: {:?}", e);
            }
        }
    }

    log!("Found {} claims", claims.len());

    Ok(Json(QueryResponse { claims }))
}
