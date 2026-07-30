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
mod query;
mod repository;
pub mod share;
mod sync;

pub use account::{
    AccountConvergenceReport, AccountDevice, AccountDisplayNameRequest, AccountDisplayNameResponse,
    AccountLinkRequest, AccountRepositoryEstablishRequest, AccountStatus, RevocationProjection,
    RevokeDeviceAcknowledgement, RevokeDeviceRequest,
};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use deployment::DeploymentConfig;
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    ACCOUNT_REQUIRED, AccountRequired, CreateSpaceRequest, CreateSpaceResponse, PendingIntent,
    RootStatus, SaveRootRequest,
};
pub use invite::{
    CreateInviteRequest, CreateInviteResponse, InvitationKind, InvitationSummary,
    RevokeInvitationAcknowledgement,
};
pub use join::{
    JoinFailureKind, JoinRequest, JoinResponse, MembershipResponse, VisitRequest, VisitResponse,
};
pub use profile::{ProfileInfo, SpaceEntry};
pub use query::Query;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration,
};
pub use sync::{
    Comparison, SyncDisposition, SyncResponse, SyncState, SyncStatusResponse, classify,
};
