//! Shared contracts for Tonk's root-owned account repository.
//!
//! This crate deliberately exposes only the account-specific constants,
//! lifecycle outcomes, and remote initialization primitive. Higher-level
//! mounting and projection policy remains with the worker and CLI adapters.

/// Customer registration contracts for the access service.
pub mod customer;
/// Retaining space authority into the account repository.
pub mod delegations;
mod descriptor;
/// Canonical device-signed account attachment detach intents.
pub mod detach;
/// Work deferred until the account confirms its email.
pub mod pending;
/// Provider-neutral account space backup artifacts.
pub mod prefix;
mod provider;
pub mod subscription;

pub use descriptor::{AccountRepositoryDescriptorV1, DescriptorError};
pub use provider::{AccountProviderError, AccountProviderRecord};

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::blob::{Import as BlobImport, Read as BlobRead};
use dialog_effects::memory::{Publish as MemoryPublish, Resolve};
use dialog_repository::{
    Branch, FetchRemoteBranchError, PublishError, PublishRemoteBranchError, PushError,
    RemoteBranch, RemoteSite, ResolveError, Revision,
};
use thiserror::Error;

/// The account repository's sole branch in descriptor version 1.
pub const MAIN_BRANCH: &str = "main";
/// The account repository's sole remote in descriptor version 1.
pub const ORIGIN_REMOTE: &str = "origin";
/// Replica kind used for the hidden account system repository.
pub const ACCOUNT_REPLICA_KIND: &str = "tonk:account";
/// Credential site holding the local provider attachment and the account
/// repository descriptor it owns.
pub const ACCOUNT_PROVIDER_CREDENTIAL_SITE: &str = "tonk-account-provider-v1";
/// Credential site holding what this device knows about its account's
/// customer registration with the access service.
pub const CUSTOMER_CREDENTIAL_SITE: &str = "tonk-customer-v1";
/// Credential site holding the trusted descriptor hash.
pub const TRUSTED_BASE_CREDENTIAL_SITE: &str = "tonk-account-trusted-base-v1";
/// Credential site holding work that cannot run until the account
/// confirms its email: provisioning calls and the custody-cell publish.
/// See [`pending`] and `plan/account-activation-gate.md` §5.
pub const PENDING_WORK_CREDENTIAL_SITE: &str = "tonk-pending-work-v1";

/// What the account service says when an account predates the repository
/// descriptor and has not established one yet.
///
/// Shared so the service and the browser agree on it: a device that has never
/// linked cannot be told to visit account setup by the `/settings` landing logic
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

/// Establish `genesis` as `origin/main`, only if no revision exists there.
///
/// A push, not a bare cell publish: the genesis tree's blocks and blobs
/// upload BEFORE the head cell points at them (the same invariant every
/// sync push keeps), and the winner's local branch comes out tracking the
/// upstream it just established, so its next push fast-forwards. The
/// atomic create-if-absent semantics ride the push's own CAS: a fresh
/// branch's sync base is empty, so the publish expects an absent cell and
/// loses cleanly to any racer. The loser fetches and returns the exact
/// winning revision rather than treating the refusal as an availability
/// error.
pub async fn publish_genesis_if_absent<Env>(
    branch: &Branch,
    remote: &RemoteBranch,
    env: &Env,
) -> Result<CreateGenesis, RemoteError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<MemoryPublish>
        + Provider<BlobRead>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Put>>
        + Provider<Fork<RemoteSite, Resolve>>
        + Provider<Fork<RemoteSite, MemoryPublish>>
        + Provider<Fork<RemoteSite, BlobImport>>
        + Provider<Fork<RemoteSite, BlobRead>>
        + ConditionalSync
        + 'static,
{
    // Probe first so retries are idempotent: an established remote wins
    // without this caller attempting anything.
    match probe_remote_main(remote, env).await? {
        RemotePresence::Present(winner) => return Ok(CreateGenesis::Loser(winner)),
        RemotePresence::Absent => {}
    }

    match branch.push().perform(env).await {
        Ok(Some(published)) => Ok(CreateGenesis::Winner(published)),
        Ok(None) => Err(RemoteError::Malformed(
            "no local revision to establish as genesis".to_string(),
        )),
        Err(
            PushError::NonFastForward { .. }
            | PushError::Publish(PublishError::VersionMismatch { .. })
            | PushError::PublishRemoteBranch(PublishRemoteBranchError::Publish(
                PublishError::VersionMismatch { .. },
            )),
        ) => match probe_remote_main(remote, env).await? {
            RemotePresence::Present(winner) => Ok(CreateGenesis::Loser(winner)),
            RemotePresence::Absent => Err(RemoteError::Malformed(
                "genesis push lost the race, but the winning revision is absent".to_string(),
            )),
        },
        Err(error) => Err(map_push_error(error)),
    }
}

fn map_push_error(error: PushError) -> RemoteError {
    match error {
        PushError::Publish(error) => map_publish_leaf(error),
        PushError::Resolve(error) => map_resolve_error(error),
        other => RemoteError::Other(other.to_string()),
    }
}

fn map_fetch_error(error: FetchRemoteBranchError) -> RemoteError {
    match error {
        FetchRemoteBranchError::Resolve(error) => map_resolve_error(error),
        FetchRemoteBranchError::Publish(error) => map_publish_leaf(error),
    }
}

fn map_resolve_error(error: ResolveError) -> RemoteError {
    match error {
        ResolveError::Authorization(error) => RemoteError::Unauthorized(error.to_string()),
        ResolveError::Io(error) => RemoteError::Unavailable(error.to_string()),
        ResolveError::Decode(message) => RemoteError::Malformed(message),
        other => RemoteError::Other(other.to_string()),
    }
}

fn map_publish_leaf(error: PublishError) -> RemoteError {
    match error {
        PublishError::Authorization(error) => RemoteError::Unauthorized(error.to_string()),
        PublishError::Io(error) => RemoteError::Unavailable(error.to_string()),
        PublishError::Encode(message) => RemoteError::Malformed(message),
        other => RemoteError::Other(other.to_string()),
    }
}

/// What a build can tell about data before opening it.
///
/// Deliberately a value rather than a `Result`: "cannot tell" is a real
/// answer with its own handling, and collapsing it into an error would make
/// a damaged record indistinguishable from an old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readability {
    /// Written by a build compatible with this one.
    Current,
    /// Written before the format this build reads. Migration applies.
    Legacy,
    /// Neither: damaged, or not a revision. No migration fixes this.
    Unknown,
}

/// Judge data by the bytes of its revision record.
///
/// Takes bytes rather than a path so it holds on every target: a service
/// worker has no filesystem, and reads the same record out of its own
/// storage. Whoever can produce the bytes can ask the question.
///
/// `None` for the bytes means the record is absent, which for a site that
/// has committed nothing is ordinary rather than suspicious — a space with no
/// revision has no data to migrate.
pub fn readability(revision: Option<&[u8]>) -> Readability {
    let Some(bytes) = revision else {
        return Readability::Current;
    };
    match revision_is_current(bytes) {
        Some(true) => Readability::Current,
        Some(false) => Readability::Legacy,
        None => Readability::Unknown,
    }
}

/// Path of a branch's revision record, relative to a site directory.
///
/// Per branch, not per site: branches are migrated independently, so a space
/// can have `main` on the current format while a meta or feature branch is
/// still legacy. Asking about a site as a whole would answer for whichever
/// branch happened to be checked and stay silent about the rest.
///
/// This hardcodes dialog's storage layout, which is the weak part: dialog's
/// own `Resolve` effect returns the same bytes as `Edition<Vec<u8>>` and
/// would be layout-independent, but the cell it names
/// (`Branch::induction_cell`) is `pub(crate)`. Reaching the record that way
/// needs a small dialog-side accessor, and is worth asking for — the bytes
/// are already there, only the address is private.
///
/// Callers that can produce the bytes some other way should: only
/// [`readability`] is the contract, and it takes bytes precisely so a
/// service worker reading its own storage never needs this function.
pub fn revision_path(repository: &str, branch: &str) -> String {
    format!("{repository}/memory/branch/{branch}/revision")
}

/// Field the current revision shape carries and the pre-upgrade one does
/// not. Its absence is what the typed decoder reports when it refuses old
/// data, so probing for it asks the same question directly.
const REVISION_BRANCH_FIELD: &str = "branch";

/// Whether a revision record was written by a build this one can read.
///
/// Decodes to a generic map rather than the current struct. The old record
/// is valid CBOR — `missing field \`branch\`` is serde reporting a *shape*
/// mismatch, not a parse failure — so its keys are readable even though its
/// shape is not, and the key set answers the question without depending on
/// the wording of an error.
///
/// Measured against a real pre-upgrade fixture:
///
/// ```text
/// old (7 fields): tree cause issuer moment period subject authority
/// new (6 fields): tree branch issuer context edition signature
/// ```
///
/// `None` means the bytes are not a CBOR map at all — damaged, or something
/// other than a revision. That is not the same as "old", so it is not
/// reported as one; the caller decides what to do with an unreadable record.
pub fn revision_is_current(bytes: &[u8]) -> Option<bool> {
    // `IgnoredAny` for the values: only the key set is being asked about,
    // and the values carry types this crate has no reason to model.
    let record: std::collections::BTreeMap<String, serde::de::IgnoredAny> =
        serde_ipld_dagcbor::from_slice(bytes).ok()?;
    Some(record.contains_key(REVISION_BRANCH_FIELD))
}

/// The on-disk format this build writes.
///
/// Bumped when data this build writes can no longer be read by the previous
/// one — the dialog upgrade that motivated all of this would have been a
/// bump from 0 to 1.
pub const SITE_FORMAT: u32 = 1;

/// File recording [`SITE_FORMAT`], written beside a site's data.
///
/// Beside the data rather than in the CLI's space registry, because the
/// registry is one adapter's bookkeeping: a site created by the worker, or
/// copied between machines by hand, carries no registry entry but still has
/// this file. It also answers before anything opens a branch, which matters
/// because opening the branch is exactly what fails on old data.
pub const SITE_FORMAT_FILE: &str = "format.json";

/// What a site's format file records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SiteFormat {
    /// The format the data beside this file is written in.
    pub format: u32,
}

impl SiteFormat {
    /// The format this build writes.
    pub fn current() -> Self {
        Self {
            format: SITE_FORMAT,
        }
    }

    /// Whether this build can read data recorded at this format.
    pub fn is_readable(&self) -> bool {
        self.format == SITE_FORMAT
    }
}

/// Read a site's recorded format, if it has one.
///
/// `None` means the file is absent, which dates the site to before this
/// marker existed — the pre-upgrade population. That is a real answer, not a
/// missing one, so it is not defaulted to the current format: doing so would
/// claim a compatibility nobody verified.
pub fn read_site_format(bytes: Option<&[u8]>) -> Option<SiteFormat> {
    serde_json::from_slice(bytes?).ok()
}

/// Whether an error means the data predates the current on-disk format.
///
/// A space written before the dialog upgrade fails when its branch is
/// opened, deep inside block decoding:
///
/// ```text
/// Failed to decode a block: Msg("missing field `branch`")
/// ```
///
/// The revision block is still CBOR, so it parses — its *shape* changed, and
/// serde reports the field it wanted. Nothing structured distinguishes that
/// from an ordinary decode failure, so this matches the signature instead,
/// which is why it is one function with one test against a real pre-upgrade
/// fixture rather than a check scattered across call sites.
///
/// Deliberately narrow: a corrupt block or an unrelated schema change should
/// keep reporting itself, not be mistaken for something a migration fixes.
///
/// This is the *fallback*, for sites written before [`SITE_FORMAT_FILE`]
/// existed and which therefore cannot announce themselves. Sites this build
/// creates carry that file, so the next incompatible change is detected by
/// comparing a number instead of matching a message dialog may reword.
pub fn is_legacy_format(error: &str) -> bool {
    error.contains("Failed to decode a block") && error.contains("missing field `branch`")
}

/// What to tell someone holding data this build cannot read.
///
/// The old binary is the only thing that can read the old format, so the
/// remedy is to install it, export, and import — not to retry.
///
/// `tonk` used to drive that itself, downloading `v0.6.7` and exporting
/// through it. The command is gone; the failure it answered is not, so
/// this spells the steps out instead of naming a command that no longer
/// exists. Detecting the format is what keeps this a sentence rather than
/// `missing field 'branch'`, which is why [`is_legacy_format`] stays.
pub const LEGACY_FORMAT_REMEDY: &str = "\
this space was written by an older tonk and cannot be opened by this one.

Only tonk v0.6.7 can read that format. To recover the data:

  1. install v0.6.7 (github.com/tonk-labs/tonk/releases/tag/v0.6.7)
  2. export each branch with it: tonk export --branch <name> --out <file>
  3. import each file here: tonk import <file> --branch <name>

Branches migrate separately, so repeat steps 2 and 3 for each one.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Branches migrate independently, so the path names one rather than
    /// standing for a whole site. A space can have `main` current while a
    /// meta branch is still legacy, and a site-wide answer would hide that.
    #[test]
    fn it_addresses_a_revision_per_branch() {
        assert_eq!(
            revision_path("main", "main"),
            "main/memory/branch/main/revision"
        );
        assert_eq!(
            revision_path("main", "meta"),
            "main/memory/branch/meta/revision",
            "a second branch has its own record and its own verdict"
        );
    }

    /// The probe against real revisions from both builds: one written by
    /// `v0.6.7`, one by this build. Committed bytes rather than
    /// hand-assembled maps, because the question is what dialog actually
    /// wrote, which no amount of reasoning about the struct can answer.
    #[test]
    fn it_tells_revisions_apart_by_their_key_set() {
        let legacy = include_bytes!("../tests/fixtures/revision-legacy.cbor");
        let current = include_bytes!("../tests/fixtures/revision-current.cbor");

        assert_eq!(
            revision_is_current(legacy),
            Some(false),
            "a pre-upgrade revision carries no `branch` field"
        );
        assert_eq!(
            revision_is_current(current),
            Some(true),
            "this build's revision does"
        );
    }

    /// Bytes that are not a revision are not reported as an old one: the
    /// caller has a damaged record, which no migration fixes.
    #[test]
    fn it_declines_to_judge_bytes_that_are_not_a_revision() {
        assert_eq!(revision_is_current(b"not cbor at all"), None);
        assert_eq!(revision_is_current(&[]), None);
    }

    /// A site records the format it was written in, and a build reading a
    /// different number knows without opening anything.
    #[test]
    fn it_reads_a_recorded_site_format() {
        let stamped = serde_json::to_vec(&SiteFormat::current()).unwrap();
        let read = read_site_format(Some(&stamped)).expect("a stamped site announces itself");
        assert!(read.is_readable());

        let future = serde_json::to_vec(&SiteFormat { format: 99 }).unwrap();
        assert!(
            !read_site_format(Some(&future)).unwrap().is_readable(),
            "a format this build does not write is not one it can read"
        );
    }

    /// An absent marker is an answer, not a default. Treating it as the
    /// current format would claim a compatibility nobody verified — and the
    /// sites without a marker are exactly the ones that need migrating.
    #[test]
    fn it_treats_an_unmarked_site_as_unknown() {
        assert!(read_site_format(None).is_none());
        assert!(read_site_format(Some(b"not json")).is_none());
    }

    /// The signature of a pre-upgrade space, verbatim from opening the
    /// committed `v0.6.7` fixture with this build.
    #[test]
    fn it_recognizes_the_legacy_format_signature() {
        let observed = "acquire branch: branch \"main\" on repository \"main\" \
             not found: Decode error: Failed to decode a block: \
             Msg(\"missing field `branch`\")";
        assert!(is_legacy_format(observed));
    }

    /// Narrow on purpose: a corrupt block or an unrelated schema change is
    /// not something a migration fixes, and saying so would send someone
    /// down a path that cannot help them.
    #[test]
    fn it_does_not_claim_unrelated_decode_failures() {
        assert!(!is_legacy_format("Failed to decode a block: Msg(\"eof\")"));
        assert!(!is_legacy_format("missing field `branch` in some config"));
        assert!(!is_legacy_format("branch \"main\" not found"));
    }

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
            map_resolve_error(ResolveError::Authorization(
                dialog_capability::AuthorizeError::Malformed {
                    detail: "denied".to_string(),
                }
            )),
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
