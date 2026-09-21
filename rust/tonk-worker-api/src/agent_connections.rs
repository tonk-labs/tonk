//! Public projections of ordinary agent invitation grant groups.
#![allow(missing_docs)]
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionTarget {
    pub cid: String,
    pub acknowledged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub id: String,
    pub repo: String,
    pub subject: String,
    pub recipient: String,
    pub label: String,
    pub scope: String,
    pub expires_at: u64,
    pub status: String,
    pub confirmed: bool,
    pub targets: Vec<AgentConnectionTarget>,
}

/// Returned only to the transient handoff adapter; never store or log this URL.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionInviteResponse {
    pub url: String,
    pub connection: AgentConnectionSummary,
}
impl std::fmt::Debug for AgentConnectionInviteResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConnectionInviteResponse")
            .field("url", &"[REDACTED]")
            .field("connection", &self.connection)
            .finish()
    }
}
