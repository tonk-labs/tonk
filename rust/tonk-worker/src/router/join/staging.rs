//! One join attempt's throwaway proof and repository store.
//!
//! A join has to establish three things before it may change anything
//! the user can see: that the candidate delegation chain composes into
//! authority over the invited subject, that the remote still honours
//! that authority, and that the content it hands back is usable. All
//! three need somewhere to put the candidate certificate and the fetched
//! blocks, and putting them in the durable stores is precisely the
//! half-installed state the join is trying to avoid.
//!
//! So the attempt runs against `Storage<VolatileSpace>`: an in-memory
//! pool holding only what the proof walk needs — the existing
//! `root -> device` grant, the candidate chain, and this attempt's own
//! `profile -> operator` session — plus the staged repository's blocks.
//! The operator is freshly derived, so its audience is this attempt's
//! alone and nothing it signs outlives the pool. Dropping [`Staging`]
//! drops all of it, which is what makes a failed attempt leave nothing
//! behind.

use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_effects::storage::{LocationExt as _, Storage as StorageLocation};
use dialog_operator::Operator;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Branch, Repository, SiteAddress};
use dialog_storage::provider::storage::{Storage, VolatileSpace};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use tonk_schema::prelude::DidExt as _;

use super::{DEFAULT_BRANCH, DEFAULT_REMOTE, JoinFailure};
use crate::worker::TonkState;

/// Location the active profile's signer is mounted at inside the staging
/// pool. Every attempt builds its own pool, so the name never collides.
const STAGING_PROFILE: &str = "join-staging";

/// The operator a staged join signs with: derived from the same device
/// profile, keyed to this attempt, and bounded by its own session.
pub(crate) type StagedOperator = Operator<VolatileSpace>;

/// A join attempt's volatile proof and repository storage.
pub(crate) struct Staging {
    operator: StagedOperator,
}

/// Nothing here is safe to render: the operator's audience is derived
/// from the device key and the pool holds candidate bearer certificates.
impl std::fmt::Debug for Staging {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Staging")
    }
}

impl Staging {
    /// Mount the active profile in a fresh volatile pool and open a
    /// bounded session over it.
    ///
    /// The profile is the same device identity the durable store uses —
    /// a staged join has to compose onto the real `root -> device` grant
    /// — but its certificate and repository storage is this pool alone.
    pub(crate) async fn open(tonk: &TonkState) -> Result<Self, JoinFailure> {
        let storage = Storage::<VolatileSpace>::volatile();
        StorageLocation::profile(STAGING_PROFILE)
            .create(Credential::Signer(tonk.profile.signer().clone()))
            .perform(&storage)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to mount the staging profile: {error}"))
            })?;

        let session = crate::session::open(&tonk.profile, &storage)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to open a staging session: {error}"))
            })?;

        Ok(Self {
            operator: session.operator,
        })
    }

    /// The operator every staged read, commit, and fetch runs through.
    pub(crate) fn operator(&self) -> &StagedOperator {
        &self.operator
    }

    /// Retain a delegation chain in the staged certificate store only.
    pub(crate) async fn retain(
        &self,
        tonk: &TonkState,
        chain: DelegationChain,
    ) -> Result<(), JoinFailure> {
        tonk.profile
            .access()
            .save(UcanDelegation(chain))
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to stage a delegation: {error}"))
            })
    }

    /// Mount a verifier-only replica for `subject` in the staged pool and
    /// wire the invite's remote onto its content branch.
    ///
    /// Mirrors the durable mount's dialog-level wiring — same subject
    /// DID, same `origin` remote, same `main` upstream — so a pull here
    /// exercises exactly the authority the durable replica would use.
    /// None of the meta facts the durable mount writes are needed: this
    /// branch exists to be pulled, inspected, and thrown away.
    pub(crate) async fn mount(
        &self,
        tonk: &TonkState,
        subject: &Did,
        remote_url: Option<&str>,
    ) -> Result<Branch, JoinFailure> {
        let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|error| {
            JoinFailure::malformed(format!("subject is not a valid Ed25519 did:key: {error:?}"))
        })?;

        let space = Subject::from(tonk.profile.did()).attenuate(Space::new(subject.repo_key()));
        let credential = space
            .create(Credential::from(verifier))
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to mount the staged replica: {error}"))
            })?;
        let repository = Repository::from(credential);

        let branch = repository
            .branch(DEFAULT_BRANCH)
            .open()
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!(
                    "failed to open the staged content branch: {error}"
                ))
            })?;

        let Some(url) = remote_url else {
            return Ok(branch);
        };

        let remote = repository
            .remote(DEFAULT_REMOTE)
            .create(SiteAddress::from(UcanAddress::new(url)))
            .subject(subject.clone())
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to attach the staged remote: {error}"))
            })?;
        let target = remote
            .branch(DEFAULT_BRANCH)
            .open()
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to open the staged upstream: {error}"))
            })?;
        branch
            .set_upstream(&target)
            .perform(&self.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to track the staged upstream: {error}"))
            })?;

        Ok(branch)
    }
}
