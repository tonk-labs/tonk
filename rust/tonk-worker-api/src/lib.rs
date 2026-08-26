#![warn(missing_docs)]
//! Wire DTOs for the Tonk service-worker HTTP API.
//!
//! These are the request/response shapes that cross the HTTP
//! boundary between the worker (`tonk-worker`) and its clients (the
//! `tonk-ui` page, the `tonk` CLI). They are plain serde data types
//! with no engine dependency, so a client can name and (de)serialize
//! them without linking the datalog engine that the worker itself
//! runs.
//!
//! `tonk-worker` re-exports every type defined here at the same
//! module paths it used to define them, so its handler code is
//! unchanged.

mod account;
mod claim;
mod conclusion;
mod deployment;
mod evaluate;
mod identify;
mod identity;
mod invite;
mod join;
mod profile;
mod profiles;
mod query;
mod repository;
pub mod share;
mod sync;

pub use account::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDeletionSpace,
    AccountDevice, AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest,
    AccountSpaceDeletionRequest, AccountStatus, AccountSummary, HostedSpaceDeletionResult,
    RevokeDeviceAcknowledgement, RevokeDeviceRequest,
};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use deployment::DeploymentConfig;
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    CREATE_ACCOUNT_REQUEST, ENCRYPTION_KEY_REQUEST, LINK_ACCOUNT, LinkAccountRequest,
    PasskeyMetadata, RootStatus, SaveRootRequest, WEBAUTHN, WebAuthnKind, WebAuthnRequest,
};
pub use invite::{
    CreateInviteRequest, CreateInviteResponse, InvitationKind, InvitationSummary,
    RevokeInvitationAcknowledgement,
};
pub use join::{JoinFailureKind, JoinRequest, JoinResponse};
pub use profile::{ProfileInfo, SpaceEntry};
pub use profiles::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};
pub use query::Query;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration,
};
use serde_json::{Value, json};
pub use sync::{
    Comparison, SyncDisposition, SyncResponse, SyncState, SyncStatusResponse, classify,
};

/// keeps this consistent with [`rename_repo_claim_json`].
pub fn create_space_claim_json(name: &str, remote: &str, template: &str) -> Value {
    let mut parameters = json!({ "name": name });
    if !remote.is_empty() {
        parameters["remote"] = json!(remote);
    }
    if !template.is_empty() {
        parameters["template"] = json!(template);
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "A request to create a new space from the wizard form.",
                        "with": {
                            "name":       { "the": "dom.event.current-target.elements.name/value", "as": "Text" },
                            "remote":     { "the": "dom.event.current-target.elements.remote/value", "as": "Text" },
                            "template":   { "the": "dom.event.current-target.elements.template/value", "as": "Text" }
                        }
                    }
                },
                "parameters": parameters
            }
        }]
    })
}
