//! Inspect routes for querying branch, remote, and archive information.

pub mod archive;
pub mod branch;
pub mod remote;

pub use branch::BranchStatusResponse;
pub use remote::{RemoteBranchStatusResponse, RemoteStatusResponse};
