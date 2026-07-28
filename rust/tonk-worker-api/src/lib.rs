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
mod evaluate;
mod identify;
mod identity;
mod join;
mod profile;
mod query;
mod repository;
mod sync;

pub use account::{AccountDevice, AccountLinkRequest, AccountStatus, RevokeDeviceRequest};
pub use claim::{ClaimResponse, QueryResponse};
pub use conclusion::{Conclusion, Frame};
pub use evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};
pub use identify::IdentifyResponse;
pub use identity::{
    CreateSpaceRequest, CreateSpaceResponse, IdentityIntent, IdentityRequired, RootStatus,
    SaveRootRequest,
};
pub use join::{JoinRequest, JoinResponse};
pub use profile::{ProfileInfo, SpaceEntry};
pub use query::Query;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration,
};
pub use sync::{Comparison, SyncResponse, SyncState, SyncStatusResponse, classify};
