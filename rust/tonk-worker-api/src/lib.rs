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
mod analytics;
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
pub use analytics::{ANALYTICS_MESSAGE, AnalyticsEvent, AnalyticsMessage};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use deployment::DeploymentConfig;
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    AccountCreation, CREATE_ACCOUNT_REQUEST, CUSTODY_REQUEST, CustodyIntent, DeviceLink,
    ENCRYPTION_KEY_REQUEST, Enrollment, LINK_ACCOUNT, LinkAccountRequest, PasskeyAddition,
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

/// The `space/create` claim, in the shape the seeded descriptor decodes.
///
/// Defined here so every dispatcher builds the same transient: the Hub's
/// form, the FAB's `new` row, and the browser tests that drive creation
/// the way the app does.
///
/// `name` alone. No `remote`: where a space syncs is resolved worker-side
/// from the account's own registration, and a page that supplied one made
/// every create look like a deliberate choice of this server — which wired
/// spaces created before anyone registered to a service that refuses to
/// serve them. No `template` either: template seeding went with the
/// template libraries, and a field the form does not carry fails to
/// resolve and aborts the whole command.
///
/// The inline `with:` block must stay identical to the descriptor in
/// `profile.yaml`, or the transient mints a different entity and no
/// handler fires.
pub fn create_space_claim_json(name: &str) -> Value {
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "A request to create a new space.",
                        "with": {
                            "name": { "the": "dom.event.current-target.elements.name/value", "as": "Text" }
                        }
                    }
                },
                "parameters": { "name": name }
            }
        }]
    })
}
