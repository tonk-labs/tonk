//! Inspect routes for querying branch and site information.

pub mod branch;
pub mod site;

pub use branch::{BranchStatusResponse, UpstreamStatusResponse, branch};
pub use site::{CredentialsResponse, RemoteBranchStatusResponse, SiteStatusResponse};
