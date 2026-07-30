//! Shared contracts for Tonk's root-owned account repository.
//!
//! This crate deliberately exposes only the account-specific constants,
//! lifecycle outcomes, and remote initialization primitive. Higher-level
//! mounting and projection policy remains with the worker and CLI adapters.

mod descriptor;
mod link;

pub use descriptor::{AccountRepositoryDescriptorV1, DescriptorError};
pub use link::{AccountLinkError, AccountLinkRecord};

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::memory::{Publish as MemoryPublish, Resolve};
use dialog_repository::{
    FetchRemoteBranchError, PublishError, PublishRemoteBranchError, RemoteBranch, RemoteSite,
    ResolveError, Revision,
};
use thiserror::Error;

/// The account repository's sole branch in descriptor version 1.
pub const MAIN_BRANCH: &str = "main";
/// The account repository's sole remote in descriptor version 1.
pub const ORIGIN_REMOTE: &str = "origin";
/// Replica kind used for the hidden account system repository.
pub const ACCOUNT_REPLICA_KIND: &str = "tonk:account";
/// Existing credential site holding the versioned local account-link record.
pub const ACCOUNT_LINK_CREDENTIAL_SITE: &str = "tonk-account-link-v1";
/// Credential site holding the trusted descriptor hash.
pub const TRUSTED_BASE_CREDENTIAL_SITE: &str = "tonk-account-trusted-base-v1";

/// What the account service says when an account predates the repository
/// descriptor and has not established one yet.
///
/// Shared so the service and the browser agree on it: a device that has never
/// linked cannot be told to visit account setup by the `/account` landing logic
/// (that path needs an existing local link), so the browser recognizes this
/// conflict on the wire and explains the recovery instead of surfacing a raw
/// status line.
pub const UNESTABLISHED_ACCOUNT_CONFLICT: &str =
    "account repository is not established; finish account-state setup first";

/// Whether the configured remote has an established `main` revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemotePresence {
    /// The remote explicitly reported that the revision cell does not exist.
    Absent,
    /// The remote returned a concrete established revision.
    Present(Revision),
}

/// Result of publishing canonical empty genesis with create-if-absent semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateGenesis {
    /// This caller atomically created the remote revision cell.
    Winner(Revision),
    /// Another caller already established this exact remote revision.
    Loser(Revision),
}

/// Durable local account-state lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountStateStatus {
    /// No valid account repository descriptor is stored locally.
    Unconfigured,
    /// A descriptor exists, but no trusted remote base has been adopted.
    Unhydrated,
    /// The trusted-base marker matches the current descriptor.
    Ready,
}

/// Remote failures that never authorize account repository initialization.
#[derive(Debug, Error)]
pub enum RemoteError {
    /// The provider rejected the presented authority.
    #[error("remote authorization failed: {0}")]
    Unauthorized(String),
    /// The provider could not be reached.
    #[error("remote is unavailable: {0}")]
    Unavailable(String),
    /// The provider returned bytes that could not be decoded.
    #[error("remote returned malformed data: {0}")]
    Malformed(String),
    /// A lower layer collapsed the remote failure into an opaque error.
    #[error("remote operation failed: {0}")]
    Other(String),
}

/// Probe `origin/main`, distinguishing only a confirmed missing revision cell
/// from every remote failure.
pub async fn probe_remote_main<Env>(
    branch: &RemoteBranch,
    env: &Env,
) -> Result<RemotePresence, RemoteError>
where
    Env: Provider<Fork<RemoteSite, Resolve>> + Provider<MemoryPublish> + ConditionalSync,
{
    match branch.fetch().perform(env).await {
        Ok(Some(revision)) => Ok(RemotePresence::Present(revision)),
        Ok(None) => Ok(RemotePresence::Absent),
        Err(error) => Err(map_fetch_error(error)),
    }
}

/// Publish `genesis` only when `origin/main` is absent.
///
/// The operation probes first so retries are idempotent. If another caller
/// wins between the probe and conditional publish, this fetches and returns
/// the exact winning revision rather than treating the CAS failure as an
/// availability error.
pub async fn publish_genesis_if_absent<Env>(
    branch: &RemoteBranch,
    genesis: Revision,
    env: &Env,
) -> Result<CreateGenesis, RemoteError>
where
    Env: Provider<Fork<RemoteSite, Resolve>>
        + Provider<Fork<RemoteSite, MemoryPublish>>
        + Provider<MemoryPublish>
        + ConditionalSync,
{
    match probe_remote_main(branch, env).await? {
        RemotePresence::Present(winner) => return Ok(CreateGenesis::Loser(winner)),
        RemotePresence::Absent => {}
    }

    match branch.publish(genesis.clone()).perform(env).await {
        Ok(()) => Ok(CreateGenesis::Winner(genesis)),
        Err(PublishRemoteBranchError::Publish(PublishError::VersionMismatch { .. })) => {
            match probe_remote_main(branch, env).await? {
                RemotePresence::Present(winner) => Ok(CreateGenesis::Loser(winner)),
                RemotePresence::Absent => Err(RemoteError::Malformed(
                    "conditional publish lost, but the winning revision is absent".to_string(),
                )),
            }
        }
        Err(error) => Err(map_publish_error(error)),
    }
}

fn map_fetch_error(error: FetchRemoteBranchError) -> RemoteError {
    match error {
        FetchRemoteBranchError::Resolve(error) => map_resolve_error(error),
        FetchRemoteBranchError::Publish(error) => map_publish_leaf(error),
    }
}

fn map_publish_error(error: PublishRemoteBranchError) -> RemoteError {
    match error {
        PublishRemoteBranchError::Publish(error) => map_publish_leaf(error),
        PublishRemoteBranchError::MissingEdition => {
            RemoteError::Malformed("published remote revision has no edition".to_string())
        }
    }
}

fn map_resolve_error(error: ResolveError) -> RemoteError {
    match error {
        ResolveError::Authorization(message) => RemoteError::Unauthorized(message),
        ResolveError::Io(error) => RemoteError::Unavailable(error.to_string()),
        ResolveError::Decode(message) => RemoteError::Malformed(message),
        other => RemoteError::Other(other.to_string()),
    }
}

fn map_publish_leaf(error: PublishError) -> RemoteError {
    match error {
        PublishError::Authorization(message) => RemoteError::Unauthorized(message),
        PublishError::Io(error) => RemoteError::Unavailable(error.to_string()),
        PublishError::Encode(message) => RemoteError::Malformed(message),
        other => RemoteError::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_keeps_account_lifecycle_states_distinct() {
        assert_ne!(
            AccountStateStatus::Unconfigured,
            AccountStateStatus::Unhydrated
        );
        assert_ne!(AccountStateStatus::Unhydrated, AccountStateStatus::Ready);
    }

    #[test]
    fn it_preserves_concrete_remote_error_classes() {
        assert!(matches!(
            map_resolve_error(ResolveError::Authorization("denied".to_string())),
            RemoteError::Unauthorized(_)
        ));
        assert!(matches!(
            map_resolve_error(ResolveError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "offline",
            ))),
            RemoteError::Unavailable(_)
        ));
        assert!(matches!(
            map_resolve_error(ResolveError::Decode("bad revision".to_string())),
            RemoteError::Malformed(_)
        ));
        assert!(matches!(
            map_resolve_error(ResolveError::Storage("opaque status".to_string())),
            RemoteError::Other(_)
        ));
    }
}
