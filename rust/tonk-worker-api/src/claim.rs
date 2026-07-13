//! Claim-route wire DTOs.

use serde::{Deserialize, Serialize};

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
