//! Fact assertion and query routes.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum::extract::Query as AxumQuery;
use axum_wasm_macros::wasm_compat;
use base64::Engine;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{Attribute, Entity, Relation, Value};

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for fact assertion.
#[derive(Debug, Deserialize)]
pub struct AssertPath {
    /// The entity identifier.
    pub entity: String,
    /// The attribute namespace.
    pub attribute_ns: String,
    /// The attribute name.
    pub attribute_name: String,
}

/// Query parameters for fact queries.
#[derive(Debug, Deserialize)]
pub struct FactQuery {
    /// The attribute to query (e.g., "namespace/name").
    pub the: Option<String>,
    /// The entity to query.
    pub of: Option<String>,
    /// The value to query.
    pub is: Option<String>,
}

/// Response for fact assertion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssertResponse {
    /// Whether the assertion succeeded.
    pub success: bool,
    /// The entity that was asserted.
    pub entity: String,
    /// The attribute that was asserted.
    pub attribute: String,
}

/// A single fact in the query response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactResponse {
    /// The attribute.
    pub the: String,
    /// The entity.
    pub of: String,
    /// The value.
    pub is: serde_json::Value,
}

/// Response for fact query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResponse {
    /// The facts that matched the query.
    pub facts: Vec<FactResponse>,
}

/// Parse a value from the request body based on content type.
fn parse_value(content_type: Option<&str>, body: &[u8]) -> Result<Value, TonkWorkerError> {
    match content_type {
        Some(ct) if ct.starts_with("text/plain") => {
            // Store as string
            let text = String::from_utf8(body.to_vec())
                .map_err(|e| TonkWorkerError::Internal(format!("Invalid UTF-8: {}", e)))?;
            Ok(Value::String(text))
        }
        Some(ct) if ct.starts_with("application/json") => {
            // Try to parse JSON and convert scalars
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|e| TonkWorkerError::Internal(format!("Invalid JSON: {}", e)))?;
            json_to_value(json)
        }
        Some(ct) => {
            // Unknown content type - store as bytes
            log!("Unknown content type '{}', storing as bytes", ct);
            Ok(Value::Bytes(body.to_vec()))
        }
        None => {
            // No content type - store as bytes
            Ok(Value::Bytes(body.to_vec()))
        }
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
            // i128 doesn't fit in JSON number, convert to i64 if possible
            if *i >= i64::MIN as i128 && *i <= i64::MAX as i128 {
                serde_json::Value::Number((*i as i64).into())
            } else {
                serde_json::Value::String(i.to_string())
            }
        }
        Value::UnsignedInt(u) => {
            // u128 doesn't fit in JSON number, convert to u64 if possible
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
            // Encode bytes as base64
            let encoded = base64::engine::general_purpose::STANDARD.encode(b);
            serde_json::Value::String(encoded)
        }
        Value::Entity(e) => serde_json::Value::String(e.to_string()),
        Value::Record(r) => {
            // Encode record bytes as base64
            let encoded = base64::engine::general_purpose::STANDARD.encode(r);
            serde_json::Value::String(encoded)
        }
        Value::Symbol(s) => serde_json::Value::String(s.to_string()),
    }
}

/// Handles fact assertion requests.
///
/// POST /api/fact/assert/:entity/:attribute-ns/:attribute-name
#[wasm_compat]
pub async fn assert_fact(
    State(state): State<AppState>,
    Path(path): Path<AssertPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AssertResponse>, TonkWorkerError> {
    let attribute_str = format!("{}/{}", path.attribute_ns, path.attribute_name);
    log!(
        "Asserting fact: entity={}, attribute={}",
        path.entity,
        attribute_str
    );

    // Parse the entity identifier
    let entity: Entity = path.entity.parse().map_err(|e| {
        TonkWorkerError::Internal(format!("Invalid entity '{}': {}", path.entity, e))
    })?;

    // Parse the attribute
    let attribute: Attribute = attribute_str
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("Invalid attribute: {}", e)))?;

    // Get content type
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok());

    // Parse the value
    let value = parse_value(content_type, &body)?;

    // Create the relation
    let relation = Relation::new(attribute, entity, value);

    // Transact the relation
    {
        let mut tonk_state = state.write().await;
        tonk_state.space.transact([relation]).await.map_err(|e| {
            log!("Failed to assert fact: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to assert fact: {}", e))
        })?;
    }

    log!("Fact asserted successfully");

    Ok(Json(AssertResponse {
        success: true,
        entity: path.entity,
        attribute: attribute_str,
    }))
}

/// Handles fact query requests.
///
/// GET /api/fact/query?the=namespace/name&of=entity&is=value
#[wasm_compat]
pub async fn query_facts(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<FactQuery>,
) -> Result<Json<QueryResponse>, TonkWorkerError> {
    use futures_util::TryStreamExt;
    use tonk_space::Fact as FactType;

    log!(
        "Querying facts: the={:?}, of={:?}, is={:?}",
        query.the,
        query.of,
        query.is
    );

    // At least one constraint is required
    if query.the.is_none() && query.of.is_none() && query.is.is_none() {
        return Err(TonkWorkerError::Internal(
            "At least one of 'the', 'of', or 'is' must be specified".to_string(),
        ));
    }

    let tonk_state = state.read().await;

    // Build the query using the Fact selector
    let mut selector = FactType::<Value>::select();

    if let Some(attr) = &query.the {
        // Validate attribute format (must be namespace/name)
        if !attr.contains('/') {
            return Err(TonkWorkerError::Internal(format!(
                "Invalid attribute '{}': must be in 'namespace/name' format",
                attr
            )));
        }
        selector = selector.the(attr.as_str());
    }

    if let Some(entity_str) = &query.of {
        let entity: Entity =
            entity_str
                .parse()
                .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                    TonkWorkerError::Internal(format!("Invalid entity '{}': {}", entity_str, e))
                })?;
        selector = selector.of(entity);
    }

    if let Some(value_str) = &query.is {
        // Try to parse as JSON first, fall back to string
        let value = if let Ok(json) = serde_json::from_str::<serde_json::Value>(value_str) {
            json_to_value(json)?
        } else {
            Value::String(value_str.to_string())
        };
        selector = selector.is(value);
    }

    let compiled = selector
        .compile()
        .map_err(|e| TonkWorkerError::Internal(format!("Query compilation error: {}", e)))?;

    let facts: Vec<FactType<Value>> = compiled
        .query(&tonk_state.space)
        .try_collect()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Query execution error: {}", e)))?;

    // Convert to response format
    let fact_responses: Vec<FactResponse> = facts
        .iter()
        .filter_map(|f| match f {
            FactType::Assertion { the, of, is, .. } => Some(FactResponse {
                the: the.to_string(),
                of: of.to_string(),
                is: value_to_json(is),
            }),
            _ => None, // Skip retractions in the response
        })
        .collect();

    log!("Found {} facts", fact_responses.len());

    Ok(Json(QueryResponse {
        facts: fact_responses,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_space_with_delegation;
    use super::*;
    use crate::api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_asserts_and_queries_fact() {
        let (space, operator, delegation) = test_space_with_delegation().await;
        let app = api_router(space, operator, delegation);

        // Assert a fact
        let request = Request::builder()
            .uri("/api/fact/assert/test:entity/test/name")
            .method("POST")
            .header("content-type", "text/plain")
            .body(Body::from("Test Name"))
            .expect("Failed to build request");

        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        // Query the fact
        let request = Request::builder()
            .uri("/api/fact/query?the=test/name&of=test:entity")
            .method("GET")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let query_response: QueryResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert_eq!(query_response.facts.len(), 1);
        assert_eq!(query_response.facts[0].is, serde_json::json!("Test Name"));
    }
}
