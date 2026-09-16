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

/// A current browser replica and the result of proving the exact build bundle.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLinkSpace {
    pub repo: String,
    pub subject: String,
    pub name: String,
    pub can_delegate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLinkSpaces {
    pub max_spaces: usize,
    pub account: String,
    pub snapshot: String,
    pub spaces: Vec<TerminalLinkSpace>,
    pub grant_lifetime_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLinkApproveRequest {
    /// Hex-encoded complete canonical signed public request.
    pub request: String,
    pub snapshot: String,
    /// Explicit current subjects, including when the UI selected all current.
    pub subjects: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLinkApprovalReceipt {
    pub request_id: String,
    pub recorded: bool,
    pub connections: Vec<AgentConnectionSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLinkDeclineRequest {
    pub request: String,
    pub snapshot: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalConnectionSummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_additions: Vec<TerminalPendingAddition>,
    pub request_id: String,
    pub recipient: String,
    pub label: String,
    pub account: String,
    /// Historical mailbox acknowledgement, not current access authorization.
    pub delivery_status: String,
    pub spaces: Vec<AgentConnectionSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalConnectionAddRequest {
    pub snapshot: String,
    pub subjects: Vec<String>,
    /// Stable across retries of this explicit add action; a new action uses a new ID.
    pub operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalConnectionAddReceipt {
    pub delivery_id: String,
    pub recorded: bool,
    pub connections: Vec<AgentConnectionSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalConnectionRevokeRequest {
    /// Omitted means every grant group currently issued for this terminal.
    #[serde(default)]
    pub group_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalPendingAddition {
    pub operation_id: String,
    pub delivery_id: String,
    pub subjects: Vec<String>,
}
