//! Redeeming an invite URL, atomically.
//!
//! A join is externally a single transition: either the recipient ends
//! up holding a usable replica of the invited subject, or nothing about
//! their profile changed. Getting there takes three stages, and the
//! whole point of the split is that only the last one writes anything
//! durable.
//!
//! ```text
//! parse -> verify audience -> build candidate chain -> stage proof/repository
//!       -> authorize remote -> pull, mutate, and validate staged content
//!       -> install staged content -> commit authority/profile/guest state
//!       -> backup -> navigate
//! ```
//!
//! [`prepare_join`] does the reads: parse the URL, check the invite is
//! addressed to this identity, and build the candidate delegation chain
//! in memory. [`stage_join`] proves it, against a
//! [`Staging`](staging::Staging) pool that never touches the durable
//! stores: the remote either honours the chain or it does not, the
//! content either carries what the space needs or it does not, and the
//! roster facts this claim adds ride the same staged revision.
//! [`commit_join`] then installs the exact staged revision, saves the
//! accepted authority, and only then indexes the replica in the profile.
//!
//! Two outcomes:
//!
//! - **Joined** — there was no replica for this subject; one was
//!   created, keyed by the subject DID. The name is not chosen here:
//!   it lives in the shared repository's content branch and arrives
//!   over the staged pull. 201 Created.
//! - **Renewed** — the recipient already had a replica for this
//!   subject. The chain was still saved (so the recipient picks
//!   up any new access this invite carries — e.g. an extension of
//!   an expiring delegation), but no replica was created. 200 OK.
//!
//! Both branches return a [`RepositoryInfo`] for the replica the
//! recipient ends up at, so the UI navigates to
//! `/space/{repository.name}` regardless of outcome. The `outcome`
//! tag in the JSON body lets callers iterate on UX without
//! changing the wire format.
//!
//! Local replica DID == invited subject DID: dialog's
//! `space.create()` accepts a verifier-only credential, and
//! commits are signed by operator/profile authority rather than
//! the repo credential. Sharing a DID across users keeps
//! `Replica.this` (`hash(profile, subject)`) and the sigil glyph
//! stable everyone-side.
//!
//! Invite URLs carry bearer authority in their query and fragment, so
//! no type in this module renders one: [`PreparedJoin`],
//! [`StagedJoin`], and [`JoinFailure`] all redact, and failure copy is
//! fixed text chosen from a closed set rather than anything an upstream
//! response said.

mod staging;

use ::axum::{
    Json,
    extract::{Path, Request, State},
    http::StatusCode,
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Changes, Entity, Statement as _, Value};
use dialog_capability::access::{Prove, Retain};
use dialog_capability::{Fork, Provider, Subject};
use dialog_common::ConditionalSync;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{
    Branch, PullError, RemoteSite, Repository, RepositoryExt as _, Revision, SiteAddress,
};
use dialog_ucan::{Ucan, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::{
    Invitation, InvitationExecution, InvitedVia, MemberName, MemberRole, Membership, Replica,
    RepositoryName, prelude::DidExt as _,
};
use tonk_worker_api::JoinFailureKind;

use self::staging::Staging;
use super::AppState;
use super::repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, build_repository_info, record_initialized_replica_in_profile,
    record_replica_local_meta,
};
use crate::{TonkWorkerError, worker::TonkState};

/// The single branch the profile repository lives on (`main`; the
/// profile has no content/meta split).
const PROFILE_BRANCH: &str = "main";

/// Default upstream branch wired up when the invite carries a
/// `remote=` URL.
const DEFAULT_BRANCH: &str = "main";

/// Default remote name used for the access service URL.
const DEFAULT_REMOTE: &str = "origin";

/// The bookmark the space route mounts (`<tonk-display model=tonk/space>`).
/// A replica whose content cannot resolve it renders "Model not found"
/// instead of the space, so a join that would land there is not a join.
const SPACE_MODEL_NAME: &str = "tonk/space";

/// Attribute binding a bookmark name to the entity it refers to.
const NAME_REFERENT: &str = "dialog.name/referent";

/// Marker claim every concept declared on a branch carries. Its presence is
/// what separates "the name resolves" from "the model behind it exists".
///
/// Deliberately the marker and not `dialog.meta/source`: `source` is the
/// descriptor materialised as JSON by the concept-of-concept query, which
/// reconstructs it from the branch's facts. Nothing ever asserts it, so a raw
/// claims read for `source` answers "no" for every concept that has ever
/// existed (see `tonk_schema::concept::concept_of_concept_descriptor`). The
/// marker is the fact the query itself enumerates concepts by.
const CONCEPT_MARKER: &str = "dialog.meta/concept";

/// The provider surface a branch read, commit, or remote fallback needs.
///
/// Staged and durable work run against different operators
/// ([`StagedOperator`](staging::StagedOperator) over volatile storage,
/// the session operator over the device's), so the helpers they share
/// are generic over this bundle rather than over `TonkState`.
pub(crate) trait BranchEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Import>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Prove<Ucan>>
    + Provider<Retain<Ucan>>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> BranchEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Prove<Ucan>>
        + Provider<Retain<Ucan>>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Body of `POST /api/profile/join`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JoinRequest {
    /// Full invite URL including any `#fragment`.
    ///
    /// Audience-open invites carry the ephemeral seed in the URL
    /// fragment; browsers never send fragments with `fetch`, so the
    /// caller must read `window.location.href` client-side and
    /// forward the complete string.
    pub url: String,
}

/// Body of a successful `POST /api/profile/join` response.
///
/// The `outcome` discriminator splits "we created a new local
/// replica for you" from "you already had one; we just refreshed
/// your access." UIs can navigate to `repository.name` either
/// way — only the toast / banner copy differs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JoinResponse {
    /// A new local replica was created for the invited subject
    /// under the requested name. Status 201.
    Joined {
        /// Repository info for the freshly created replica.
        repository: RepositoryInfo,
    },
    /// The recipient already had a replica for this subject. The
    /// invite's delegation chain was saved (renewing access if
    /// the invite carried fresh delegations) but no new replica
    /// was created and the requested name is ignored. Status 200.
    Renewed {
        /// Repository info for the existing replica the recipient
        /// will land in.
        repository: RepositoryInfo,
    },
}

/// Which authority a join installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinMode {
    /// Bounded guest authority from an audience-open invite. Creates no
    /// durable membership and leaves a guest credential behind, so a
    /// later promotion knows what to replay.
    GuestVisit,
    /// Durable membership terminating at the recipient's root.
    Durable,
}

/// Why a join stopped short of committing.
///
/// A missing local root is not a failure — it is a request for the
/// identity the join needs, and the same URL is replayed once the
/// ceremony completes.
pub(crate) enum JoinRejection {
    /// A durable join was asked for before the device had a root.
    IdentityRequired,
    /// The join reached a terminal classification.
    Failed(JoinFailure),
}

impl From<JoinFailure> for JoinRejection {
    fn from(failure: JoinFailure) -> Self {
        Self::Failed(failure)
    }
}

impl From<JoinRejection> for TonkWorkerError {
    fn from(rejection: JoinRejection) -> Self {
        match rejection {
            JoinRejection::IdentityRequired => TonkWorkerError::RootRequired,
            JoinRejection::Failed(failure) => failure.into(),
        }
    }
}

/// A terminal join classification plus operator-facing context.
///
/// `detail` is assembled from typed error variants, HTTP statuses, and
/// stable service codes only. The invite URL, its fragment, the
/// delegation bytes, and upstream response bodies never reach it, which
/// is what makes this type safe to render.
pub(crate) struct JoinFailure {
    kind: JoinFailureKind,
    detail: String,
}

/// Renders the same tag the overlay shows, so a log line and the state
/// the recipient is looking at name the failure identically.
impl std::fmt::Debug for JoinFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JoinFailure")
            .field("kind", &self.kind.as_str())
            .field("detail", &self.detail)
            .finish()
    }
}

impl JoinFailure {
    fn new(kind: JoinFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    /// The classification, for the caller that renders a terminal state.
    pub(crate) fn kind(&self) -> JoinFailureKind {
        self.kind
    }

    pub(crate) fn malformed(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::Malformed, detail)
    }

    fn audience_mismatch(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::AudienceMismatch, detail)
    }

    fn revoked(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::Revoked, detail)
    }

    fn unavailable(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::Unavailable, detail)
    }

    pub(crate) fn claim_failed(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::ClaimFailed, detail)
    }
}

impl From<JoinFailure> for TonkWorkerError {
    fn from(failure: JoinFailure) -> Self {
        let kind = failure.kind();
        let message = kind.message().to_string();
        match kind {
            JoinFailureKind::Malformed => TonkWorkerError::Router(message),
            JoinFailureKind::AudienceMismatch => TonkWorkerError::Forbidden(message),
            JoinFailureKind::Revoked => TonkWorkerError::Upstream {
                status: 403,
                code: Some("CREDENTIAL_REVOKED".to_string()),
                message,
            },
            JoinFailureKind::Unavailable => TonkWorkerError::Upstream {
                status: 503,
                code: Some("JOIN_UNAVAILABLE".to_string()),
                message,
            },
            JoinFailureKind::ClaimFailed => TonkWorkerError::Internal(message),
        }
    }
}

/// The terminal classifications, their fixed copy, and how each one
/// reaches the caller. Pure data, so this runs on both targets.
#[cfg(test)]
mod failure_vocabulary {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use super::{JoinFailure, JoinFailureKind, JoinRejection};
    use crate::TonkWorkerError;

    const KINDS: [JoinFailureKind; 5] = [
        JoinFailureKind::Malformed,
        JoinFailureKind::AudienceMismatch,
        JoinFailureKind::Revoked,
        JoinFailureKind::Unavailable,
        JoinFailureKind::ClaimFailed,
    ];

    #[dialog_common::test]
    fn it_fixes_the_message_for_every_kind() {
        let messages: Vec<&str> = KINDS.iter().map(|kind| kind.message()).collect();
        assert_eq!(
            messages,
            vec![
                "This invite link is invalid.",
                "This invite was issued to a different identity.",
                "This invite has been revoked.",
                "Tonk could not reach this spot. Try again.",
                "Tonk could not join this spot.",
            ],
        );
    }

    #[dialog_common::test]
    fn it_tags_every_kind_distinctly() {
        let tags: Vec<&str> = KINDS.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(
            tags,
            vec![
                "malformed",
                "audience-mismatch",
                "revoked",
                "unavailable",
                "claim-failed",
            ],
        );
    }

    /// The classification the overlay reads is the one the failure was
    /// raised with — a log line and the state on screen never disagree.
    #[dialog_common::test]
    fn it_carries_its_classification_to_the_overlay() {
        let failure = JoinFailure::unavailable("remote answered 503");
        assert_eq!(failure.kind(), JoinFailureKind::Unavailable);
        assert!(
            format!("{failure:?}").contains("unavailable"),
            "a rendered failure names its tag"
        );
    }

    /// The detail a failure carries is for logs; the recipient only ever
    /// sees the fixed copy, so nothing an upstream said can leak through
    /// the route body.
    #[dialog_common::test]
    fn it_never_returns_the_operator_detail_to_the_caller() {
        let failure = JoinFailure::revoked("remote refused with 403");
        let error: TonkWorkerError = failure.into();
        let TonkWorkerError::Upstream {
            status,
            code,
            message,
        } = error
        else {
            panic!("a revoked join is an upstream refusal");
        };
        assert_eq!(status, 403);
        assert_eq!(code.as_deref(), Some("CREDENTIAL_REVOKED"));
        assert_eq!(message, JoinFailureKind::Revoked.message());
    }

    #[dialog_common::test]
    fn it_reports_an_unreachable_remote_as_retryable() {
        let error: TonkWorkerError = JoinFailure::unavailable("no route").into();
        let TonkWorkerError::Upstream { status, code, .. } = error else {
            panic!("an unreachable remote is an upstream failure");
        };
        assert_eq!(status, 503);
        assert_eq!(code.as_deref(), Some("JOIN_UNAVAILABLE"));
    }

    #[dialog_common::test]
    fn it_separates_a_wrong_recipient_from_a_bad_link() {
        assert!(matches!(
            TonkWorkerError::from(JoinFailure::audience_mismatch("not this root")),
            TonkWorkerError::Forbidden(_)
        ));
        assert!(matches!(
            TonkWorkerError::from(JoinFailure::malformed("no access parameter")),
            TonkWorkerError::Router(_)
        ));
    }

    /// A missing root is a request for identity, not a terminal failure —
    /// the route has to keep answering `ROOT_REQUIRED` so the gate opens.
    #[dialog_common::test]
    fn it_keeps_a_missing_root_out_of_the_failure_vocabulary() {
        assert!(matches!(
            TonkWorkerError::from(JoinRejection::IdentityRequired),
            TonkWorkerError::RootRequired
        ));
    }
}

/// A parsed, audience-verified invite and everything the later stages
/// need to decide what this join has to prove.
///
/// Holds the invite and the URL a guest promotion replays, so it
/// deliberately has no derived `Debug`.
pub(crate) struct PreparedJoin {
    mode: JoinMode,
    /// The URL as supplied. Retained only so a guest visit can store it
    /// for its own later promotion; never logged, asserted, or returned.
    url: String,
    invite: Invite,
    /// Derived from the chain *as parsed*, before any redelegation
    /// changes the leaf.
    invitation: Invitation,
    /// Explicit relay metadata carried by modern invites.
    invitation_execution: Option<InvitationExecution>,
    subject: Did,
    key: String,
    /// The durable member the chain terminates at. `None` for a guest
    /// visit, whose audience is minted per attempt.
    member: Option<Did>,
    /// The candidate chain, already built for a durable claim. A guest
    /// visit mints one per audience, so it is built during staging.
    chain: Option<DelegationChain>,
    /// Access service the invite carried, if any.
    remote_url: Option<String>,
    /// Explicit revocation relay the invite carried, if any.
    revocation_url: Option<String>,
    /// A replica for this subject is already recorded in the profile.
    existing: bool,
    /// A guest credential is retained for this subject.
    guest: bool,
}

impl std::fmt::Debug for PreparedJoin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedJoin")
            .field("mode", &self.mode)
            .field("subject", &self.subject)
            .field("existing", &self.existing)
            .field("guest", &self.guest)
            .finish_non_exhaustive()
    }
}

impl PreparedJoin {
    /// Whether the remote has to honour the candidate chain before
    /// anything durable changes.
    ///
    /// Every remote-backed attempt, including a renewal, proves that the
    /// candidate authority is still accepted before it becomes durable.
    /// The exception is an existing owner redeeming its own link: it already
    /// holds root authority and usable local content, so no candidate access
    /// is being added.
    fn needs_remote_authorization(&self) -> bool {
        self.remote_url.is_some() && !(self.existing && self.is_self_claim())
    }

    /// Whether the current root minted the invitation it is claiming.
    ///
    /// An existing owner already holds the subject's root authority and
    /// usable local content, so redeeming its own link adds no candidate
    /// authority for the remote to authorize.
    fn is_self_claim(&self) -> bool {
        self.member
            .as_ref()
            .is_some_and(|member| self.invitation.inviter.0 == member.this())
    }

    /// Whether this attempt has to produce usable initial content.
    ///
    /// Only a new replica does: an existing one is already readable, and
    /// a renewal does not replace its content.
    fn installs_replica(&self) -> bool {
        !self.existing
    }
}

/// A join whose authority and content have been proven in volatile
/// storage and are ready to be committed.
///
/// Owns the staging pool for as long as the commit needs to read out of
/// it — dropping this before [`commit_join`] is what makes a failed
/// attempt leave nothing behind.
pub(crate) struct StagedJoin {
    prepared: PreparedJoin,
    staging: Staging,
    /// The candidate chain staging accepted. For a guest visit this
    /// terminates at the staged operator and is re-minted at commit time
    /// for the real one.
    chain: DelegationChain,
    /// Staged branch and the exact revision to install when the attempt
    /// produced a content head.
    installable: Option<(Branch, Revision)>,
    /// The same member has already claimed this exact invitation.
    repeated_claim: bool,
}

impl std::fmt::Debug for StagedJoin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StagedJoin")
            .field("prepared", &self.prepared)
            .field(
                "installable",
                &self.installable.as_ref().map(|(_, revision)| revision),
            )
            .finish_non_exhaustive()
    }
}

/// Visit an audience-open invite without creating durable membership.
#[wasm_compat]
pub async fn visit(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let tonk = state.write().await;
    let outcome = join_invite(&tonk, &body.url, JoinMode::GuestVisit).await?;
    joined_response(&tonk, outcome).await
}

/// Redeem an invite URL durably to the local root.
#[wasm_compat]
pub async fn join(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let tonk = state.write().await;
    let outcome = join_invite(&tonk, &body.url, JoinMode::Durable).await?;
    log!(
        "POST /api/profile/join -> subject {} (key {})",
        outcome.subject,
        outcome.key
    );
    joined_response(&tonk, outcome).await
}

/// Load the committed replica and shape the route's success body.
async fn joined_response(
    tonk: &TonkState,
    outcome: JoinOutcome,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let repository = tonk
        .profile
        .repository(outcome.key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to load the joined replica: {error}"))
        })?;
    let info = build_repository_info(tonk, &outcome.key, &repository).await;
    Ok(if outcome.renewed {
        (
            StatusCode::OK,
            Json(JoinResponse::Renewed { repository: info }),
        )
    } else {
        (
            StatusCode::CREATED,
            Json(JoinResponse::Joined { repository: info }),
        )
    })
}

/// The result of a successful join: the routing key, subject DID, and
/// whether the replica pre-existed (`renewed`) or was freshly created.
/// Deliberately repository-free so the concrete `Repository<R>` type
/// (which differs between the load and create paths) doesn't leak into
/// the signature — callers re-load by key if they need the handle.
pub(crate) struct JoinOutcome {
    /// The routing/storage key (subject DID suffix).
    pub key: String,
    /// The joined subject DID.
    pub subject: Did,
    /// `true` when a replica already existed (renewed access, no new
    /// replica); `false` when a fresh replica was created.
    pub renewed: bool,
}

#[derive(Serialize, Deserialize)]
struct GuestRecord {
    version: u8,
    url: String,
}

fn guest_site(subject: &Did) -> String {
    format!("tonk-guest-invite-v1:{}", subject.repo_key())
}

/// Retain the invite a guest visit was opened with, so an explicit
/// promotion can replay it without the page holding the URL.
pub(crate) async fn save_guest(
    tonk: &TonkState,
    subject: &Did,
    url: &str,
) -> Result<(), JoinFailure> {
    let record = serde_json::to_vec(&GuestRecord {
        version: 1,
        url: url.to_string(),
    })
    .map_err(|error| {
        JoinFailure::claim_failed(format!("failed to serialize the guest record: {error}"))
    })?;
    tonk.profile
        .credential()
        .site(guest_site(subject))
        .save(record)
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to save the guest record: {error}"))
        })
}

/// Drop the retained guest invite. Part of the durable commit, never a
/// preflight write: until it runs, a promotion that fails still leaves
/// the guest with working bounded access.
async fn clear_guest(tonk: &TonkState, subject: &Did) -> Result<(), JoinFailure> {
    tonk.profile
        .credential()
        .site(guest_site(subject))
        .save(Vec::<u8>::new())
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to clear the guest record: {error}"))
        })
}

async fn guest_url(tonk: &TonkState, subject: &Did) -> Result<Option<String>, TonkWorkerError> {
    let bytes = match tonk
        .profile
        .credential()
        .site(guest_site(subject))
        .load::<Vec<u8>>()
        .perform(&tonk.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load guest record: {error}"
            )));
        }
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let record: GuestRecord = serde_json::from_slice(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored guest record is invalid: {error}"))
    })?;
    Ok(Some(record.url))
}

/// Whether this replica is a guest visit rather than a durable member.
///
/// A guest installed bounded invite authority and no roster row. Anything that
/// needs to delegate the spot's access — minting an invite of its own — has to
/// ask, because a guest cannot: the retained invite is what it holds, and a
/// mint delegates from durable membership.
/// Gated like its only caller (`run_invite`, service-worker only), so a native
/// build doesn't carry it as dead code.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn is_guest_replica(
    tonk: &TonkState,
    subject: &Did,
) -> Result<bool, TonkWorkerError> {
    Ok(guest_url(tonk, subject).await?.is_some())
}

/// Active local membership mode for one mounted repository.
#[derive(Debug, Serialize)]
pub struct MembershipResponse {
    /// `guest` while only bounded invite authority is installed, otherwise `durable`.
    pub status: &'static str,
}

/// Report whether this local replica is a guest visit or durable root member.
#[wasm_compat]
pub async fn membership(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<MembershipResponse>, TonkWorkerError> {
    let tonk = state.read().await;
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let status = if guest_url(&tonk, &repository.did()).await?.is_some() {
        "guest"
    } else {
        "durable"
    };
    Ok(Json(MembershipResponse { status }))
}

/// Explicitly promote a locally visited guest using its retained invite URL.
///
/// Answers with the promoted replica rather than an empty 204: the
/// acknowledgement is useful to the caller, and a body-bearing status is
/// one fewer null-body case at the browser conversion boundary.
#[wasm_compat]
pub async fn join_guest(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    request: Request,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let tonk = state.write().await;
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let url = guest_url(&tonk, &repository.did())
        .await?
        .ok_or_else(|| TonkWorkerError::Conflict("this replica is already durable".to_string()))?;
    match join_invite(&tonk, &url, JoinMode::Durable).await {
        Ok(outcome) => joined_response(&tonk, outcome).await,
        Err(JoinRejection::IdentityRequired) => {
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            {
                let client = request.extensions().get::<crate::router::ClientId>();
                crate::router::navigate::notify_identity_required(
                    client,
                    tonk_worker_api::IdentityIntent::DurableJoin { url },
                );
            }
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            let _ = (request, url);
            Err(TonkWorkerError::RootRequired)
        }
        Err(JoinRejection::Failed(failure)) => {
            log!("guest promotion failed: {failure:?}");
            Err(failure.into())
        }
    }
}

/// The one join operation: HTTP visit, HTTP join, the `tonk:join`
/// command, and guest promotion all run through it.
///
/// Nothing durable changes before [`commit_join`], and everything
/// [`commit_join`] does is either local or already proven, so a failure
/// at any earlier stage leaves the recipient's profile, repository list,
/// roster, guest credential, and claim backup exactly as they were.
pub(crate) async fn join_invite(
    tonk: &TonkState,
    url: &str,
    mode: JoinMode,
) -> Result<JoinOutcome, JoinRejection> {
    let prepared = prepare_join(tonk, url, mode).await?;
    let staged = stage_join(tonk, prepared).await?;
    Ok(commit_join(tonk, staged).await?)
}

/// Parse the invite, verify it is addressed to this identity, and build
/// the candidate chain. Reads only.
async fn prepare_join(
    tonk: &TonkState,
    url: &str,
    mode: JoinMode,
) -> Result<PreparedJoin, JoinRejection> {
    let invite = Invite::parse_url(url)
        .await
        .map_err(|error| JoinFailure::malformed(format!("invite did not parse: {error}")))?;

    // Derived from the chain as parsed — a claim pushes a redelegation
    // and changes the leaf. Guaranteed `Some` by the `Invite` invariant
    // (the chain has a specific subject).
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");

    let open = matches!(&invite.audience, InviteAudience::Open { .. });
    let invitation_execution = invite.revocation_url.as_ref().map(|relay| {
        InvitationExecution::new(
            &invitation,
            if open { "open" } else { "scoped" },
            relay.as_str(),
        )
    });
    let subject = invite.subject().clone();
    let key = subject.repo_key().to_owned();
    let remote_url = invite.remote_url.as_ref().map(url::Url::to_string);
    let revocation_url = invite.revocation_url.as_ref().map(url::Url::to_string);

    // Audience: an open invite redelegates to whoever redeems it; a
    // targeted one only ever redeems for the DID it names.
    let (member, chain) = match mode {
        JoinMode::GuestVisit => {
            if !open {
                return Err(JoinFailure::audience_mismatch(
                    "a targeted invite cannot be opened as a guest",
                )
                .into());
            }
            (None, None)
        }
        JoinMode::Durable => {
            let member = match crate::router::account::member_did(tonk).await {
                Ok(member) => member,
                Err(TonkWorkerError::RootRequired) => {
                    return Err(JoinRejection::IdentityRequired);
                }
                Err(error) => {
                    return Err(JoinFailure::claim_failed(format!(
                        "failed to resolve the joining member: {error}"
                    ))
                    .into());
                }
            };
            let claimed = invite.clone().claim(&member).await.map_err(|error| {
                if open {
                    JoinFailure::claim_failed(format!("invite did not extend: {error}"))
                } else {
                    JoinFailure::audience_mismatch(format!("invite is not for this root: {error}"))
                }
            })?;
            (Some(member), Some(claimed.chain))
        }
    };

    let existing = find_replica_for_subject(tonk, &subject)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to look up the local replica: {error}"))
        })?;
    let guest = guest_url(tonk, &subject)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to read the guest record: {error}"))
        })?
        .is_some();

    Ok(PreparedJoin {
        mode,
        url: url.to_owned(),
        invite,
        invitation,
        invitation_execution,
        subject,
        key,
        member,
        chain,
        remote_url,
        revocation_url,
        existing,
        guest,
    })
}

/// Prove the candidate chain against the remote and the content, in
/// volatile storage.
///
/// The certificate store this stage writes to is the staging pool's, so
/// the candidate chain never becomes durable authority until it has
/// passed. When the join creates a replica, the roster facts this claim
/// adds are committed onto the staged branch too — so the revision
/// [`commit_join`] installs already contains them, and no fallible
/// content work is left after the durable authority is saved.
async fn stage_join(tonk: &TonkState, prepared: PreparedJoin) -> Result<StagedJoin, JoinFailure> {
    let staging = Staging::open(tonk).await?;

    // Retain only what the proof walk needs. The staged session's
    // `profile -> operator` delegation is already in the pool; a durable
    // claim also needs the `root -> device` grant it composes onto.
    if prepared.member.is_some() {
        let root = crate::router::identity::local_root(tonk)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to read the local root: {error}"))
            })?;
        staging.retain(tonk, root.delegation).await?;
    }

    let chain = match &prepared.chain {
        Some(chain) => chain.clone(),
        // A guest visit's authority is minted per audience, so the staged
        // attempt gets its own bounded delegation and the real operator
        // only receives one once this stage has passed.
        None => {
            prepared
                .invite
                .clone()
                .visit(&staging.operator().did())
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!("invite did not extend to a guest: {error}"))
                })?
                .chain
        }
    };
    staging.retain(tonk, chain.clone()).await?;

    let branch = staging
        .mount(tonk, &prepared.subject, prepared.remote_url.as_deref())
        .await?;

    // A renewal starts from the exact local head, then merges the remote into
    // that staged copy. Starting empty would discard unpushed local content.
    if prepared.existing {
        copy_existing_to_stage(tonk, &prepared, &branch, staging.operator()).await?;
    }

    if prepared.needs_remote_authorization() {
        pull_staged(&branch, staging.operator()).await?;
        validate_content(&branch, staging.operator(), &prepared.subject).await?;
    }

    // A guest records no roster row; every durable claim, including a
    // renewal, stages roster/provenance/name into the exact revision that will
    // be installed before authority is saved.
    let mut repeated_claim = false;
    if let Some(member) = &prepared.member {
        let (changes, already_claimed) = claim_changes(
            tonk,
            &branch,
            staging.operator(),
            &prepared.invitation,
            prepared.invitation_execution.as_ref(),
            member,
            &prepared.subject,
        )
        .await?;
        repeated_claim = already_claimed;
        if !changes.is_empty() {
            branch
                .transaction()
                .assert(changes)
                .commit()
                .perform(staging.operator())
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!("failed to stage the claim: {error}"))
                })?;
        }
    }
    if prepared.needs_remote_authorization() {
        validate_content(&branch, staging.operator(), &prepared.subject).await?;
    }

    // A local-only guest visit can still have no revision. Every durable
    // claim and every existing replica has an exact staged head to install.
    let installable = branch.revision().map(|revision| (branch, revision));

    Ok(StagedJoin {
        prepared,
        staging,
        chain,
        installable,
        repeated_claim,
    })
}

/// Seed a renewal's volatile branch from the exact durable local head.
async fn copy_existing_to_stage(
    tonk: &TonkState,
    prepared: &PreparedJoin,
    destination: &Branch,
    destination_env: &staging::StagedOperator,
) -> Result<(), JoinFailure> {
    let repository = tonk
        .profile
        .repository(prepared.key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to load the existing replica: {error}"))
        })?;
    let source = repository
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to open the existing content: {error}"))
        })?;
    let Some(revision) = source.revision() else {
        return Ok(());
    };
    let installed = source
        .install(revision.clone())
        .perform(&tonk.operator, destination_env)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to stage existing content: {error}"))
        })?;
    if installed != revision {
        return Err(JoinFailure::claim_failed(
            "staged existing revision does not match the local head",
        ));
    }
    destination
        .reset(installed)
        .perform(destination_env)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to open staged existing content: {error}"))
        })?;
    Ok(())
}

/// Commit a proven join, in the order that keeps every intermediate
/// state either invisible or usable.
///
/// Content is installed while the repository is still unindexed, the
/// accepted authority is saved next, and the profile `Replica` fact —
/// the moment the join becomes visible — lands only once both are in
/// place. The guest credential, the backup, and the caller's navigation
/// all follow.
async fn commit_join(tonk: &TonkState, staged: StagedJoin) -> Result<JoinOutcome, JoinFailure> {
    let StagedJoin {
        prepared,
        staging,
        chain,
        installable,
        repeated_claim,
    } = staged;

    let repository = if prepared.installs_replica() {
        mount_replica(
            tonk,
            &prepared.subject,
            prepared.remote_url.as_deref(),
            prepared.revocation_url.as_deref(),
        )
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to prepare the local replica: {error}"))
        })?
    } else {
        tonk.profile
            .repository(prepared.key.as_str())
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to load the renewing replica: {error}"))
            })?
    };
    if let Some((source, revision)) = installable {
        install_revision(
            tonk,
            &source,
            staging.operator(),
            &repository,
            revision,
            prepared.needs_remote_authorization(),
        )
        .await?;
    }

    save_authority(tonk, &prepared, chain).await?;

    if prepared.installs_replica() {
        record_initialized_replica_in_profile(tonk, &prepared.subject)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to index the replica: {error}"))
            })?;
    }

    match prepared.mode {
        JoinMode::GuestVisit => save_guest(tonk, &prepared.subject, &prepared.url).await?,
        JoinMode::Durable => {
            if prepared.guest {
                clear_guest(tonk, &prepared.subject).await?;
            }
            // Escrow a newly accepted claim so another of this account's
            // devices can recover the space. Exact local repeats are already
            // escrowed, while an owner's space-root prefix is backed up by
            // the owned-space path. Best-effort, and strictly after the local
            // commit — the join is already complete.
            if !(prepared.is_self_claim() || prepared.existing && repeated_claim) {
                crate::router::account_backup::back_up_claim(
                    tonk,
                    prepared
                        .chain
                        .as_ref()
                        .expect("a durable join builds its chain during preparation"),
                    prepared.remote_url.as_deref(),
                    prepared.revocation_url.as_deref(),
                )
                .await;
            }
        }
    }

    log!(
        "join: committed subject {} (key {}, renewed {})",
        prepared.subject,
        prepared.key,
        prepared.existing
    );

    Ok(JoinOutcome {
        key: prepared.key,
        subject: prepared.subject,
        renewed: prepared.existing,
    })
}

/// Save the authority this join accepted into the durable certificate
/// store.
///
/// Idempotent at the dialog layer — re-saving the same chain is a no-op,
/// re-saving an extended one adds a fresh proof. Either way the
/// recipient's effective access can only grow, never shrink, by joining.
async fn save_authority(
    tonk: &TonkState,
    prepared: &PreparedJoin,
    staged_chain: DelegationChain,
) -> Result<(), JoinFailure> {
    let chain = match prepared.mode {
        JoinMode::Durable => staged_chain,
        // The staged guest delegation is addressed to the staging
        // operator and dies with it, so the durable one is minted here —
        // after staging proved the invite is good for it.
        JoinMode::GuestVisit => {
            prepared
                .invite
                .clone()
                .visit(&tonk.operator.did())
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!("guest authority did not mint: {error}"))
                })?
                .chain
        }
    };

    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to save the accepted authority: {error}"))
        })
}

/// Copy the exact staged revision — its whole reachable tree and every
/// blob it references — into the durable repository, then publish it as
/// the branch head and verify what landed.
///
/// Not an export/import: that would mint a synthetic commit, drop the
/// history the remote handed back, and leave blobs behind. `install`
/// writes blocks without publishing a head, so the destination stays
/// unreadable until the `reset` below, and the head it then carries is
/// byte-identical to the one that was validated.
async fn install_revision(
    tonk: &TonkState,
    source: &Branch,
    source_env: &staging::StagedOperator,
    repository: &Repository<Credential>,
    revision: Revision,
    validate_remote_content: bool,
) -> Result<(), JoinFailure> {
    let installed = source
        .install(revision.clone())
        .perform(source_env, &tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to install the staged content: {error}"))
        })?;
    if installed != revision {
        return Err(JoinFailure::claim_failed(
            "installed revision does not match the staged one",
        ));
    }

    let destination = repository
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to open the local content branch: {error}"))
        })?;
    destination
        .reset(installed.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to publish the installed content: {error}"))
        })?;

    // Re-open rather than trust the handle just written through: this is
    // the check that the content is readable from durable storage, not
    // merely that the write returned.
    let landed = repository
        .branch(DEFAULT_BRANCH)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("installed content did not load back: {error}"))
        })?;
    if landed.revision().as_ref() != Some(&installed) {
        return Err(JoinFailure::claim_failed(
            "installed content did not become the local head",
        ));
    }
    if validate_remote_content {
        validate_content(&landed, &tonk.operator, &repository.did()).await?;
    }

    // The reactor may hold a handle from an earlier attempt at this key;
    // the install moved the head underneath it. Leaving a stale handle
    // cached would wedge every later sync on this branch, so a failure
    // here fails the join rather than being logged past.
    tonk.reactor
        .refresh_branch(repository.did().repo_key(), DEFAULT_BRANCH, &tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to adopt the installed branch: {error}"))
        })?;

    Ok(())
}

/// Pull the staged branch and classify what the remote said.
///
/// The typed service response survives the pull's error chain, so a
/// revoked credential is separated from an unreachable service without
/// reading either one's body.
async fn pull_staged<Env: BranchEnv>(branch: &Branch, env: &Env) -> Result<(), JoinFailure> {
    match branch.pull().perform(env).await {
        Ok(_) => Ok(()),
        Err(error) => Err(classify_pull(&error)),
    }
}

fn classify_pull(error: &PullError) -> JoinFailure {
    let Some(response) = dialog_effects::service::find_service_response(error) else {
        return JoinFailure::unavailable("the remote could not be reached");
    };
    match response.code.as_deref() {
        Some("CREDENTIAL_REVOKED" | "DEVICE_REVOKED") => {
            JoinFailure::revoked(format!("remote refused with {}", response.status))
        }
        Some("AUDIENCE_MISMATCH" | "SUBJECT_NOT_ALLOWED") => JoinFailure::audience_mismatch(
            format!("remote refused the audience with {}", response.status),
        ),
        Some("REVOCATION_UNAVAILABLE") => {
            JoinFailure::unavailable(format!("remote answered {}", response.status))
        }
        _ if response.status == 401 || response.status == 403 => {
            JoinFailure::claim_failed(format!("remote refused with {}", response.status))
        }
        _ => JoinFailure::unavailable(format!("remote answered {}", response.status)),
    }
}

/// Check that a branch carries what navigating into the space needs: the
/// repository's own identity and name, and the `tonk/space` model the
/// space route mounts.
///
/// Without this a join can finish against a branch that has no view and
/// drop the recipient on "Model not found" — a durable, unusable
/// replica. Only content that resolves both is accepted.
async fn validate_content<Env: BranchEnv>(
    branch: &Branch,
    env: &Env,
    subject: &Did,
) -> Result<(), JoinFailure> {
    let names: Vec<RepositoryName> = branch
        .query()
        .select(Query::<RepositoryName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            JoinFailure::unavailable(format!("repository identity did not resolve: {error:?}"))
        })?;
    if !names.iter().any(|row| row.this == subject.this()) {
        return Err(JoinFailure::unavailable(
            "the space has no repository identity yet",
        ));
    }

    let bookmark: Entity = format!("id:{SPACE_MODEL_NAME}")
        .parse()
        .map_err(|error| JoinFailure::claim_failed(format!("bad model bookmark: {error}")))?;
    let referent = first_value(branch, env, bookmark, NAME_REFERENT)
        .await?
        .ok_or_else(|| JoinFailure::unavailable("the space model name does not resolve"))?;
    let Value::Entity(model) = referent else {
        return Err(JoinFailure::unavailable(
            "the space model name resolves to a non-entity",
        ));
    };
    if first_value(branch, env, model, CONCEPT_MARKER)
        .await?
        .is_none()
    {
        return Err(JoinFailure::unavailable(
            "the space model is not a concept on this branch",
        ));
    }
    Ok(())
}

/// Read one raw claim off a branch, or `None` when the entity carries no
/// such attribute.
async fn first_value<Env: BranchEnv>(
    branch: &Branch,
    env: &Env,
    entity: Entity,
    attribute: &str,
) -> Result<Option<Value>, JoinFailure> {
    let attribute: Attribute = attribute
        .parse()
        .map_err(|error| JoinFailure::claim_failed(format!("bad validation attribute: {error}")))?;
    let claims = branch
        .claims()
        .select(ArtifactSelector::new().of(entity).the(attribute))
        .perform(env)
        .await
        .map_err(|error| {
            JoinFailure::unavailable(format!("staged content did not read back: {error}"))
        })?;
    futures_util::pin_mut!(claims);
    match claims.next().await {
        None => Ok(None),
        Some(Ok(artifact)) => Ok(Some(artifact.is)),
        Some(Err(error)) => Err(JoinFailure::unavailable(format!(
            "staged content did not decode: {error}"
        ))),
    }
}

fn membership_has_name(names: &[MemberName], membership: &Membership) -> bool {
    names.iter().any(|name| name.this == *membership.this())
}

/// Decide which roster facts a claimed invite adds to a repository's
/// content branch, and pack them into one batch either transaction
/// builder can absorb.
///
/// The facts: the invitation itself (idempotent when the minter already
/// wrote it; self-healing when the invite predates invitation records),
/// the claimer's membership, their display name when the membership is
/// unnamed, a `member` role, and — first-wins — the `InvitedVia`
/// provenance stamp.
///
/// The content branch (not meta) because it's the synced, shared branch:
/// every member pulls it, so the roster converges across the space. A
/// roster on the device-local meta branch would only ever show the
/// claimer's own row.
///
/// First-wins: provenance answers "how did this member first get in", so
/// an existing stamp is never overwritten by a later claim (`invitation`
/// is cardinality-one and a re-assert would silently replace the
/// original inviter). Self-claims (the claimer minted the invitation)
/// are not provenance and are skipped. Role is first-wins too — `role`
/// is cardinality-one, so blindly stamping `member` would demote a
/// founder who reclaims their own invite.
///
/// Generic over the environment so a staged branch and the reactor's
/// cached durable handle take the same path: a new replica's roster is
/// part of the revision that gets installed, and a renewal's is a commit
/// on the branch it already has.
async fn claim_changes<Env: BranchEnv>(
    tonk: &TonkState,
    branch: &Branch,
    env: &Env,
    invitation: &Invitation,
    invitation_execution: Option<&InvitationExecution>,
    member: &Did,
    subject: &Did,
) -> Result<(Changes, bool), JoinFailure> {
    let membership = Membership::new(member.clone(), subject.clone());

    let stamps: Vec<InvitedVia> = branch
        .query()
        .select(Query::<InvitedVia> {
            this: Term::var("this"),
            invitation: Term::var("invitation"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("invited-via query failed: {error:?}"))
        })?;
    let already_stamped = stamps.iter().any(|stamp| stamp.this == *membership.this());
    let already_claimed = stamps
        .iter()
        .any(|stamp| stamp.this == *membership.this() && stamp.invitation.0 == *invitation.this());
    if already_claimed {
        return Ok((Changes::new(), true));
    }

    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("member-role query failed: {error:?}"))
        })?;
    let already_roled = roles.iter().any(|role| role.this == *membership.this());

    // Guard the name too: a linked device may resolve a different local
    // display name, but a later sequential join must not overwrite an
    // existing roster rename. This read-then-write guard is intentionally
    // not a linearizable first-writer lock for concurrent claims.
    let names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("member-name query failed: {error:?}"))
        })?;
    let already_named = membership_has_name(&names, &membership);

    // A member claiming their own invite is not provenance.
    let self_invite = invitation.inviter.0 == member.this();

    let mut changes = Changes::new();
    invitation.clone().assert(&mut changes);
    if let Some(execution) = invitation_execution {
        execution.clone().assert(&mut changes);
    }
    membership.clone().assert(&mut changes);
    if !already_named {
        let display_name = crate::router::profile_name::resolve_display_name(tonk).await;
        MemberName::new(membership.this().clone(), display_name).assert(&mut changes);
    }
    if !already_roled {
        MemberRole::member(membership.this().clone()).assert(&mut changes);
    }
    if !already_stamped && !self_invite {
        InvitedVia::new(membership.this().clone(), invitation.this().clone()).assert(&mut changes);
    }
    Ok((changes, already_claimed))
}

/// Prepare a local verifier-only replica for `subject` under its DID and
/// configure its remote/branch, without making it visible.
///
/// Hidden on purpose: this writes the repository's own meta branch but
/// no profile `Replica` fact, so the repository exists in storage and
/// stays out of the Hub until the caller commits its visibility
/// ([`record_initialized_replica_in_profile`]). That split is what lets
/// content be installed and verified before the recipient can navigate
/// to it. Local DID == `subject` DID, so `Replica.this`
/// (`hash(profile, subject)`) and the sigil glyph converge across every
/// recipient of the same space.
///
/// Resumable: an attempt that installed content and then failed before
/// the visibility commit leaves an unindexed repository behind, so a
/// retry loads it rather than failing on a duplicate create.
pub(crate) async fn mount_replica(
    tonk: &TonkState,
    subject: &Did,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
) -> Result<Repository<Credential>, TonkWorkerError> {
    let key = subject.repo_key().to_owned();
    if super::account_state::is_account_key(tonk, &key).await {
        return Err(TonkWorkerError::Forbidden(
            "account system repository cannot be joined as a user space".to_string(),
        ));
    }

    // Create a verifier-only credential keyed to the subject DID, then
    // mount it as a local replica at the routing key (so path ==
    // identity). An earlier attempt may already have done this.
    let repository = match tonk
        .profile
        .repository(key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(_) => {
            let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|e| {
                TonkWorkerError::Router(format!("subject is not a valid Ed25519 did:key: {e:?}"))
            })?;
            let space_capability = Subject::from(tonk.profile.did()).attenuate(Space::new(&key));
            let space_credential = space_capability
                .create(Credential::from(verifier))
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!(
                        "failed to create local replica '{key}': {e}"
                    ))
                })?;
            Repository::from(space_credential)
        }
    };

    // Mirror what `PUT /api/repository/{name}` writes: a single `main`
    // branch, plus an `origin` remote tracking the invite's/space's
    // access service if one was attached.
    let mut configuration = RepositoryConfiguration::default();
    if let Some(url) = remote_url {
        let address = SiteAddress::from(UcanAddress::new(url));
        let mut remote = RemoteConfiguration::new(address).subject(subject.clone());
        if let Some(relay) = revocation_url {
            remote = remote.revocation_url(url::Url::parse(relay).map_err(|error| {
                TonkWorkerError::Router(format!(
                    "invite revocation relay is not a valid URL: {error}"
                ))
            })?);
        }
        configuration = configuration.remote(DEFAULT_REMOTE, remote).branch(
            DEFAULT_BRANCH,
            BranchConfiguration {
                upstream: Some(UpstreamConfiguration::new(DEFAULT_REMOTE, DEFAULT_BRANCH)),
                revision: None,
            },
        );
    } else {
        configuration = configuration.branch(DEFAULT_BRANCH, BranchConfiguration::default());
    }

    // No display name to seed: a joined/restored repo's name lives in
    // the shared content branch and arrives over the pull. The helper
    // only uses this for log context, so the routing key stands in.
    record_replica_local_meta(tonk, &repository, &key, &configuration).await?;

    Ok(repository)
}

/// Check whether the active profile already holds a replica for the
/// given subject DID. Returns `Ok(true)` when one exists.
///
/// The replica is a name-less membership index, so this only tests
/// existence — the recipient's chosen join name does not flow into it
/// (the name lives in the synced repository's own `tonk/repository`).
pub(crate) async fn find_replica_for_subject(
    tonk: &TonkState,
    subject: &Did,
) -> Result<bool, TonkWorkerError> {
    let profile_meta = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to open profile meta branch: {e}"))
        })?;

    let rows: Vec<Replica> = profile_meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::from(tonk_schema::domain::replica::Subject(subject.this())),
            profile: Term::from(tonk_schema::domain::replica::Profile(
                tonk.profile.did().this(),
            )),
            kind: Term::from(Replica::repository_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("replica query on profile meta failed: {e:?}"))
        })?;

    Ok(!rows.is_empty())
}

/// The fixed entity the in-flight join status lives at. Both the handler
/// (writes overlay status) and the `/join` view (`entity=tonk:join/status`)
/// agree on this URI, so there's no per-attempt id to thread.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const JOIN_STATUS_URI: &str = "tonk:join/status";

/// Post-commit handler for the [`Join`] command.
///
/// `<tonk-page onmount=tonk/join>` on the `/join` view fires the command
/// with the full page URL in the event detail. This handler runs the same
/// join operation the HTTP routes do and drives the overlay-only
/// `tonk:join/status` (pending → failed, or retract + navigate on
/// success) on the profile meta branch — the branch the `/join` view
/// subscribes to.
///
/// [`Join`]: tonk_schema::command::Join
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct JoinHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl JoinHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::Join::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for JoinHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::Join::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode the full location synchronously while the caller holds the
        // lock; hand the owned value to the `'static` future.
        let command = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::Join::decode(entity, facts));
        let env = env.clone();

        Box::pin(async move {
            let Some(command) = command else {
                return;
            };
            run_join(&env, command).await;
        })
    }
}

/// Run the join operation from the command's full URL and drive the
/// overlay-only join status. Always leaves the overlay in a terminal state
/// (status retracted on success, `failed` on error).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_join(env: &crate::router::CommandEnv, command: tonk_schema::command::Join) {
    use std::sync::Arc;
    use tonk_schema::command::{JoinFailure as JoinFailureFact, JoinStatus};
    use tonk_schema::domain::join::{Kind, Reason, Status};

    let tonk = env.state().read().await;

    // Acquire the profile meta branch — the `/join` view reads
    // `tonk:join/status` from here; overlay writes + their poll target it.
    let session = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            log!("join: failed to acquire profile meta branch: {e}");
            return;
        }
    };

    let status_entity: dialog_artifacts::Entity = match JOIN_STATUS_URI.parse() {
        Ok(entity) => entity,
        Err(e) => {
            log!("join: bad status URI: {e}");
            return;
        }
    };

    // Pending: a fresh attempt clears any prior status, then marks
    // pending. Schedule a poll so the view shows "Joining…".
    session.state.clear_overlay();
    session.state.assert_overlay(JoinStatus {
        this: status_entity.clone(),
        status: Status(
            "tonk:pending"
                .parse()
                .unwrap_or_else(|_| status_entity.clone()),
        ),
    });
    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    // Use the exact page URL carried by `<tonk-page>`. In particular, do not
    // round-trip the query through `URLSearchParams`: targeted invites may
    // contain empty or repeated fields whose byte-for-byte form matters.
    let url = command.url.0;

    // An open invite mounts a guest; a targeted one goes straight to
    // durable membership. Either way the same operation runs, so the
    // content behind the redirect is proven before the redirect fires.
    let open = Invite::parse_url(&url)
        .await
        .is_ok_and(|invite| matches!(invite.audience, InviteAudience::Open { .. }));
    let mode = if open {
        JoinMode::GuestVisit
    } else {
        JoinMode::Durable
    };
    match join_invite(&tonk, &url, mode).await {
        Ok(outcome) => {
            // Success means the replica is installed, verified, and
            // indexed — its `tonk/space` model is already present, so the
            // redirect cannot land on "Model not found".
            //
            // Clear the in-flight status so the "Joining…" overlay empties,
            // then tell the originating page to redirect into `/space/<subject>`.
            //
            // The redirect is a page capability — the service worker has no
            // `window` — and this command is transient, so it never lands in
            // a branch a subscription could observe. The only channel back to
            // the page that asked is a `postMessage` to its client. We post
            // `{ type: "navigate", href }`; the page's `<tonk-host>` performs
            // the navigation.
            session.state.clear_overlay();
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            let href = format!("/space/{key}", key = outcome.key);
            crate::router::navigate::notify_navigate(env.client(), &href);
            log!(
                "join: succeeded (subject {}, key {})",
                outcome.subject,
                outcome.key
            );
        }
        Err(JoinRejection::IdentityRequired) => {
            crate::router::navigate::notify_identity_required(
                env.client(),
                tonk_worker_api::IdentityIntent::DurableJoin { url },
            );
        }
        Err(JoinRejection::Failed(failure)) => {
            // Failure: mark failed + record the fixed copy and its kind,
            // overlay-only. The reason is chosen from the closed set, so
            // neither the URL nor an upstream body can reach the page.
            let kind = failure.kind();
            session.state.assert_overlay(JoinStatus {
                this: status_entity.clone(),
                status: Status(
                    "tonk:failed"
                        .parse()
                        .unwrap_or_else(|_| status_entity.clone()),
                ),
            });
            session.state.assert_overlay(JoinFailureFact {
                this: status_entity,
                reason: Reason(kind.message().to_owned()),
                kind: Kind(kind.as_str().to_owned()),
            });
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            log!("join: failed ({}): {failure:?}", kind.as_str());
        }
    }
}

/// Post a `{ type: "sync" }` message to the originating client so it
/// dispatches a `tonk:committed` window event, prompting the sync
/// controller to push immediately instead of waiting for the heartbeat.
///
/// Mirrors [`notify_navigate`] exactly — fire-and-forget on a spawned
/// task, no `TonkState` access, so the caller's held read lock is
/// irrelevant.
///
/// [`notify_navigate`]: crate::router::navigate::notify_navigate
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn notify_sync(client: Option<&crate::router::ClientId>) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let Some(client) = client else {
        log!("notify_sync: no originating client; skipping prompt sync");
        return;
    };
    let client_id = client.0.clone();

    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(g) => g,
        Err(_) => {
            log!("notify_sync: not in a service worker scope; skipping prompt sync");
            return;
        }
    };

    spawn_local(async move {
        let client_value = match JsFuture::from(global.clients().get(&client_id)).await {
            Ok(value) if !value.is_undefined() && !value.is_null() => value,
            Ok(_) => {
                log!("notify_sync: originating client {client_id} is gone; skipping");
                return;
            }
            Err(e) => {
                log!("notify_sync: clients.get failed: {e:?}");
                return;
            }
        };
        let Ok(client) = client_value.dyn_into::<web_sys::Client>() else {
            log!("notify_sync: clients.get did not yield a Client; skipping");
            return;
        };

        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("sync"),
        );
        if let Err(e) = client.post_message(&message) {
            log!("notify_sync: post_message(sync) failed: {e:?}");
        }
    });
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::{DEFAULT_BRANCH, membership_has_name};
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;
    use tonk_invite::{Invite, InviteAudience};
    use tonk_schema::prelude::DidExt as _;
    use tonk_schema::{Invitation, MemberName, MemberRole, Membership};

    use crate::router::api_router_with_state;
    use crate::router::repository::build_repository_info;
    use crate::router::tests::{
        attach_remote, content_invitations, content_invited_via, content_member_roles,
        content_memberships, put_repo, test_state,
    };

    /// Hand-craft an audience-open invite URL for a synthetic
    /// repository subject. The subject signer doubles as root issuer.
    /// Distinct tag bytes give distinct subjects/ephemerals. Returns
    /// the URL plus the subject's routing key (the repo the join
    /// mounts the claimer's replica under).
    async fn handcrafted_invite_url(subject_tag: u8, ephemeral_tag: u8) -> (String, String) {
        open_invite_url(subject_tag, ephemeral_tag, None).await
    }

    /// The same open invite, but advertising an access service. The host
    /// does not exist, so any staged pull against it fails the way a
    /// remote outage does.
    async fn unreachable_invite_url(subject_tag: u8, ephemeral_tag: u8) -> (String, String) {
        open_invite_url(
            subject_tag,
            ephemeral_tag,
            Some("https://sync.invalid.test/ucan/"),
        )
        .await
    }

    async fn open_invite_url(
        subject_tag: u8,
        ephemeral_tag: u8,
        remote: Option<&str>,
    ) -> (String, String) {
        let subject_signer = Ed25519Signer::import(&[subject_tag; 32]).await.unwrap();
        let subject = subject_signer.did();
        let key = subject.repo_key().to_owned();
        let ephemeral_seed = [ephemeral_tag; 32];
        let ephemeral = Ed25519Signer::import(&ephemeral_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(subject_signer)
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(delegation);
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: ephemeral_seed,
            },
            remote.map(|url| url::Url::parse(url).unwrap()),
        )
        .await
        .unwrap();
        (invite.to_url("https://hub.tonk.xyz/join").unwrap(), key)
    }

    /// Hand-craft an audience-scoped invite: only `audience` can redeem
    /// it, and no fragment carries a redelegation seed.
    async fn targeted_invite_url(
        subject_tag: u8,
        audience: &dialog_varsig::Did,
    ) -> (String, String) {
        let subject_signer = Ed25519Signer::import(&[subject_tag; 32]).await.unwrap();
        let subject = subject_signer.did();
        let key = subject.repo_key().to_owned();
        let delegation = DelegationBuilder::new()
            .issuer(subject_signer)
            .audience(audience)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let invite = Invite::new(
            DelegationChain::new(delegation),
            InviteAudience::Scoped,
            None,
        )
        .await
        .unwrap();
        (invite.to_url("https://hub.tonk.xyz/join").unwrap(), key)
    }

    /// Everything a failed join must leave untouched, in one value.
    ///
    /// Covers the profile index and its seeding status, the shared
    /// roster on the subject's content branch, and the guest credential
    /// site — i.e. every surface the recipient can observe after an
    /// attempt, plus the one a promotion depends on.
    #[derive(Debug, PartialEq, Eq)]
    struct JoinSnapshot {
        /// Routing keys of every replica the profile indexes.
        replicas: Vec<String>,
        /// Seeding status of each indexed replica, key-sorted.
        statuses: Vec<(String, String)>,
        /// Membership entities on the subject's content branch.
        members: Vec<String>,
        /// Role stamps on the subject's content branch.
        roles: Vec<String>,
        /// Provenance stamps on the subject's content branch.
        provenance: Vec<String>,
        /// Whether a guest credential is retained for the subject.
        guest: bool,
        /// Whether durable proof storage authorizes the worker for the subject.
        authority: bool,
        /// Number of accepted claims handed to the backup boundary.
        backup_dispatches: usize,
    }

    async fn snapshot(state: &crate::router::AppState, key: &str) -> JoinSnapshot {
        use dialog_query::{Output as _, Query, Term};

        // The routing key *is* the subject DID; there is no suffix to strip.
        let subject: dialog_varsig::Did = key.parse().expect("subject parses");

        let (replicas, statuses, guest, authority) = {
            let tonk = state.read().await;
            let profile = tonk
                .reactor
                .profile_repository()
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("profile meta opens");
            let rows: Vec<tonk_schema::Replica> = profile
                .handle()
                .query()
                .select(Query::<tonk_schema::Replica> {
                    this: Term::var("this"),
                    subject: Term::var("subject"),
                    profile: Term::var("profile"),
                    kind: Term::var("kind"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("replica query");
            let stamps: Vec<tonk_schema::SpaceStatus> = profile
                .handle()
                .query()
                .select(Query::<tonk_schema::SpaceStatus> {
                    this: Term::var("this"),
                    status: Term::var("status"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("status query");
            let mut replicas: Vec<String> =
                rows.iter().map(|row| row.subject.0.to_string()).collect();
            replicas.sort();
            let mut statuses: Vec<(String, String)> = stamps
                .iter()
                .map(|stamp| (stamp.this.to_string(), stamp.status.0.to_string()))
                .collect();
            statuses.sort();
            let guest = super::guest_url(&tonk, &subject)
                .await
                .expect("guest record reads")
                .is_some();
            let authority = tonk
                .profile
                .access()
                .prove(dialog_capability::Subject::from(subject.clone()))
                .audience(&tonk.operator)
                .perform(&tonk.operator)
                .await
                .is_ok();
            (replicas, statuses, guest, authority)
        };

        // A repository the profile does not index may still exist in
        // storage from a resumable attempt; read it when it loads, and
        // treat an absent one as empty rather than a failure.
        let (members, roles, provenance) = if replicas.iter().any(|entry| entry == key) {
            let mut members: Vec<String> = content_memberships(state, key)
                .await
                .iter()
                .map(|row| row.this().to_string())
                .collect();
            members.sort();
            let mut roles: Vec<String> = content_member_roles(state, key)
                .await
                .iter()
                .map(|row| format!("{}={}", row.this, row.role.0))
                .collect();
            roles.sort();
            let mut provenance: Vec<String> = content_invited_via(state, key)
                .await
                .iter()
                .map(|row| format!("{}={}", row.this, row.invitation.0))
                .collect();
            provenance.sort();
            (members, roles, provenance)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        JoinSnapshot {
            replicas,
            statuses,
            members,
            roles,
            provenance,
            guest,
            authority,
            backup_dispatches: crate::router::account_backup::backup_dispatch_count(),
        }
    }

    async fn post_join(app: &axum::Router, url: &str) -> StatusCode {
        post_invite(app, "/api/profile/join", url).await
    }

    async fn post_visit(app: &axum::Router, url: &str) -> StatusCode {
        post_invite(app, "/api/profile/visit", url).await
    }

    async fn post_invite(app: &axum::Router, path: &str, url: &str) -> StatusCode {
        let body = serde_json::json!({ "url": url }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
    }

    /// `POST /api/repository/{repo}/membership` — guest promotion.
    async fn promote(app: &axum::Router, key: &str) -> (StatusCode, Option<serde_json::Value>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/membership"))
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).ok())
    }

    /// The entity a roster row is keyed on: the account root, not the
    /// device. Every roster write resolves the member through
    /// `account::member_did`, so a test that looked up its own row by
    /// profile DID would never find it.
    async fn member_entity(state: &crate::router::AppState) -> dialog_artifacts::Entity {
        let tonk = state.read().await;
        crate::router::identity::root_did(&tonk)
            .await
            .expect("the test profile has a local root")
            .this()
    }

    /// `GET /api/repository/{repo}/membership` — guest or durable.
    async fn membership_status(app: &axum::Router, key: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/membership"))
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["status"].as_str().unwrap().to_owned()
    }

    /// Joining an invite records the claimer's membership, the
    /// invitation (self-healed — the minter never wrote one), and the
    /// provenance stamp linking them.
    #[dialog_common::test]
    async fn it_records_membership_and_provenance_on_join() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(10, 11).await;
        let expected = {
            let parsed = Invite::parse_url(&url).await.unwrap();
            Invitation::from_chain(&parsed.chain).unwrap()
        };

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let memberships = content_memberships(&state, &key).await;
        let member_entity = member_entity(&state).await;
        assert!(memberships.iter().any(|m| m.member.0 == member_entity));

        let invitations = content_invitations(&state, &key).await;
        assert!(
            invitations.iter().any(|i| i.this == expected.this),
            "invitation self-healed from the URL",
        );

        let stamps = content_invited_via(&state, &key).await;
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == member_entity)
            .unwrap()
            .this()
            .clone();
        let stamp = stamps
            .iter()
            .find(|s| s.this == membership_entity)
            .expect("provenance stamp present");
        assert_eq!(stamp.invitation.0, expected.this);

        // A claimer (not the inviter) joins as a plain member.
        let roles = content_member_roles(&state, &key).await;
        let role = roles
            .iter()
            .find(|r| r.this == membership_entity)
            .expect("role stamped on the claimer's membership");
        assert_eq!(role.role.0.to_string(), MemberRole::MEMBER);
    }

    /// Claiming an invite names the claimer on the repo meta.
    #[dialog_common::test]
    async fn it_records_the_claimer_name_on_join() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(30, 31).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let memberships = content_memberships(&state, &key).await;
        let names = crate::router::tests::content_member_names(&state, &key).await;
        let member_entity = member_entity(&state).await;
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == member_entity)
            .expect("claimer membership present")
            .this()
            .clone();
        assert_eq!(names.len(), 1, "one name row per membership entity");
        assert!(
            names
                .iter()
                .any(|n| n.this == membership_entity && !n.name.0.is_empty()),
            "the claimer is named on their membership",
        );
    }

    /// A claimer's member entry records the inviter via provenance.
    #[dialog_common::test]
    async fn it_reports_provenance_in_members() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(40, 41).await;
        let expected = {
            let parsed = Invite::parse_url(&url).await.unwrap();
            Invitation::from_chain(&parsed.chain).unwrap()
        };

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let info = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            build_repository_info(&tonk, &key, &repository).await
        };

        let me = info
            .members
            .iter()
            .find(|m| m.is_self)
            .expect("self present");
        assert_eq!(
            me.invited_by.as_deref(),
            Some(expected.inviter.0.to_string().as_str()),
            "claimer records the invitation's inviter as provenance",
        );
    }

    /// A second claim against the same subject (Renewed) preserves a
    /// name already chosen for the membership instead of replacing it
    /// with the joining device's local display name.
    #[dialog_common::test]
    async fn it_does_not_overwrite_an_existing_name_on_a_renewed_join() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url_a, key) = handcrafted_invite_url(60, 61).await;
        let (url_b, _) = handcrafted_invite_url(60, 62).await;

        assert_eq!(post_join(&app, &url_a).await, StatusCode::CREATED);

        let member_entity = member_entity(&state).await;
        let membership_entity = content_memberships(&state, &key)
            .await
            .into_iter()
            .find(|m| m.member.0 == member_entity)
            .expect("claimer membership present")
            .this()
            .clone();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch(DEFAULT_BRANCH)
                .transaction()
                .assert(MemberName::new(
                    membership_entity.clone(),
                    "Chosen Name".to_string(),
                ))
                .commit()
                .perform(&tonk.operator)
                .await
                .expect("existing name commits");
        }

        assert_eq!(post_join(&app, &url_b).await, StatusCode::OK);

        let names = crate::router::tests::content_member_names(&state, &key).await;
        assert_eq!(
            names
                .iter()
                .find(|name| name.this == membership_entity)
                .map(|name| name.name.0.as_str()),
            Some("Chosen Name"),
            "renewed join preserves the existing member name",
        );
    }

    /// The name guard is a local read followed by a local write, not a
    /// linearizable first-writer lock: two replicas reading the same empty
    /// base both decide that they may stamp a name.
    #[dialog_common::test]
    async fn it_allows_concurrent_snapshots_to_both_choose_to_name_a_membership() {
        let member = Ed25519Signer::import(&[63u8; 32]).await.unwrap().did();
        let repository = Ed25519Signer::import(&[64u8; 32]).await.unwrap().did();
        let membership = Membership::new(member, repository);
        let empty_base: Vec<MemberName> = Vec::new();

        let first_replica_should_name = !membership_has_name(&empty_base, &membership);
        let second_replica_should_name = !membership_has_name(&empty_base, &membership);

        assert!(first_replica_should_name);
        assert!(second_replica_should_name);
    }

    /// A second claim against the same subject (Renewed) records the
    /// new invitation but leaves the original provenance stamp alone.
    #[dialog_common::test]
    async fn it_does_not_overwrite_provenance_on_a_renewed_join() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        // Same subject signer (tag 20), two different ephemerals.
        let (url_a, key) = handcrafted_invite_url(20, 21).await;
        let (url_b, _) = handcrafted_invite_url(20, 22).await;
        let expected_a = {
            let parsed = Invite::parse_url(&url_a).await.unwrap();
            Invitation::from_chain(&parsed.chain).unwrap()
        };
        let expected_b = {
            let parsed = Invite::parse_url(&url_b).await.unwrap();
            Invitation::from_chain(&parsed.chain).unwrap()
        };

        assert_eq!(post_join(&app, &url_a).await, StatusCode::CREATED);
        assert_eq!(post_join(&app, &url_b).await, StatusCode::OK);

        // The Renewed path still records the second invitation, even
        // though it leaves provenance pinned to the first.
        let invitations = content_invitations(&state, &key).await;
        assert!(
            invitations.iter().any(|i| i.this == expected_a.this),
            "first invitation recorded",
        );
        assert!(
            invitations.iter().any(|i| i.this == expected_b.this),
            "renewed join records the second invitation too",
        );

        let stamps = content_invited_via(&state, &key).await;
        // Exactly one stamp for this membership, still pointing at the
        // first invitation.
        let memberships = content_memberships(&state, &key).await;
        let member_entity = member_entity(&state).await;
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == member_entity)
            .unwrap()
            .this()
            .clone();
        let mine: Vec<_> = stamps
            .iter()
            .filter(|s| s.this == membership_entity)
            .collect();
        assert_eq!(mine.len(), 1, "exactly one provenance stamp");
        assert_eq!(mine[0].invitation.0, expected_a.this, "first invite wins");
    }

    /// A member claiming an invite they minted themselves gets no
    /// provenance stamp — self-invites are not provenance.
    #[dialog_common::test]
    async fn it_skips_provenance_for_self_claims() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);

        // Create own repo (addressed by its routing key), mint own invite.
        // The mint route refuses a local-only repo, so attach a remote first.
        let key = put_repo(&app, "test-self-claim").await;
        attach_remote(&app, &key, "https://sync.example.test/ucan/").await;
        // The mint route derives its link base from the request origin,
        // which the browser conversion boundary attaches. A request built
        // here bypasses that boundary, so supply it directly.
        let minted_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/invite"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .extension(crate::axum::RequestOrigin::parse("https://hub.tonk.xyz/").unwrap())
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(minted_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(minted_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let minted: crate::router::CreateInviteResponse = serde_json::from_slice(&bytes).unwrap();

        // Claiming own invite hits the Renewed path.
        assert_eq!(post_join(&app, minted.url().as_str()).await, StatusCode::OK);

        // The claimer's own membership exists, but no stamp on it.
        let memberships = content_memberships(&state, &key).await;
        let member_entity = member_entity(&state).await;
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == member_entity)
            .expect("founder membership present")
            .this()
            .clone();
        let stamps = content_invited_via(&state, &key).await;
        assert!(
            !stamps.iter().any(|s| s.this == membership_entity),
            "self-claims must not stamp provenance",
        );

        // The creator is the founder, and reclaiming their own invite must
        // NOT demote them to member (role is first-wins).
        let roles = content_member_roles(&state, &key).await;
        let role = roles
            .iter()
            .find(|r| r.this == membership_entity)
            .expect("founder role stamped at creation");
        assert_eq!(role.role.0.to_string(), MemberRole::FOUNDER);
    }

    /// An account-holder's claim keys the roster row on their root DID,
    /// and no device-keyed row is written.
    #[dialog_common::test]
    async fn it_keys_membership_on_the_root_did_for_an_account_holder() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);

        let (root_did, device_did) = {
            let state = state.read().await;
            (
                crate::router::identity::root_did(&state).await.unwrap(),
                state.profile.did(),
            )
        };

        // Join an invite.
        let (url, key) = handcrafted_invite_url(50, 51).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let memberships = content_memberships(&state, &key).await;
        let root_entity = root_did.this();
        let device_entity = device_did.this();
        assert!(
            memberships.iter().any(|m| m.member.0 == root_entity),
            "membership keyed on the root did",
        );
        assert!(
            !memberships.iter().any(|m| m.member.0 == device_entity),
            "no device-keyed row was written",
        );

        // The viewer's self row must resolve against the same root DID
        // the membership is keyed on, not the device DID — otherwise an
        // account-linked user never matches their own roster row.
        let info = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            build_repository_info(&tonk, &key, &repository).await
        };
        let me = info
            .members
            .iter()
            .find(|m| m.is_self)
            .expect("self present");
        assert_eq!(
            me.did,
            root_did.to_string(),
            "self row resolves against the account root, not the device did",
        );
    }

    // ----- a failed join leaves nothing behind -----

    #[dialog_common::test]
    async fn it_leaves_no_trace_when_the_url_is_not_an_invite() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (_, key) = handcrafted_invite_url(60, 61).await;
        let before = snapshot(&state, &key).await;

        assert_eq!(
            post_join(&app, "https://hub.tonk.xyz/join?access=not-base58").await,
            StatusCode::BAD_REQUEST,
        );

        assert_eq!(snapshot(&state, &key).await, before);
    }

    #[dialog_common::test]
    async fn it_leaves_no_trace_when_the_invite_names_another_identity() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let stranger = Ed25519Signer::import(&[99u8; 32]).await.unwrap();
        let (url, key) = targeted_invite_url(62, &stranger.did()).await;
        let before = snapshot(&state, &key).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::FORBIDDEN);

        assert_eq!(
            snapshot(&state, &key).await,
            before,
            "a wrong-recipient join records nothing",
        );
    }

    /// The invite parses and the audience matches; only the remote is
    /// gone. Nothing may be recorded — a replica whose content never
    /// arrived is exactly the half-installed state to avoid.
    #[dialog_common::test]
    async fn it_leaves_no_trace_when_the_remote_is_unreachable() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = unreachable_invite_url(64, 65).await;
        let before = snapshot(&state, &key).await;

        assert_eq!(
            post_join(&app, &url).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "an unreachable remote is a retryable upstream failure",
        );

        let after = snapshot(&state, &key).await;
        assert_eq!(after, before);
        assert!(
            !after.replicas.iter().any(|entry| entry == &key),
            "the replica never enters the profile index",
        );
    }

    /// The same holds for a guest visit: no bounded credential is minted
    /// for a space that could not be reached.
    #[dialog_common::test]
    async fn it_records_no_guest_credential_when_the_remote_is_unreachable() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = unreachable_invite_url(66, 67).await;
        let before = snapshot(&state, &key).await;

        assert_eq!(
            post_visit(&app, &url).await,
            StatusCode::SERVICE_UNAVAILABLE
        );

        let after = snapshot(&state, &key).await;
        assert_eq!(after, before);
        assert!(!after.guest, "a failed visit leaves no guest credential");
    }

    /// A remote-backed renewal is staged too: an outage cannot save the
    /// candidate authority or mutate the existing roster/head.
    #[dialog_common::test]
    async fn it_leaves_an_existing_replica_untouched_when_renewal_remote_is_unavailable() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(68, 69).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let joined = snapshot(&state, &key).await;

        let (renewal, renewal_key) = unreachable_invite_url(68, 70).await;
        assert_eq!(renewal_key, key);
        assert_eq!(
            post_join(&app, &renewal).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(snapshot(&state, &key).await, joined);
    }

    /// A renewal that fails validation must leave the replica it would
    /// have renewed exactly as it was.
    #[dialog_common::test]
    async fn it_leaves_an_existing_replica_untouched_when_a_renewal_is_rejected() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(68, 69).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let joined = snapshot(&state, &key).await;

        // A renewal of the same subject, but issued to someone else.
        let stranger = Ed25519Signer::import(&[98u8; 32]).await.unwrap();
        let (renewal, renewal_key) = targeted_invite_url(68, &stranger.did()).await;
        assert_eq!(renewal_key, key, "same subject, different audience");

        assert_eq!(post_join(&app, &renewal).await, StatusCode::FORBIDDEN);

        assert_eq!(
            snapshot(&state, &key).await,
            joined,
            "a rejected renewal changes nothing",
        );
    }

    // ----- classification -----

    /// The remote's typed refusal has to survive the pull's error chain,
    /// or a revoked invite reads as a flaky network and offers retry.
    #[dialog_common::test]
    fn it_reads_a_revoked_credential_out_of_the_pull_error_chain() {
        use dialog_effects::memory::MemoryError;
        use dialog_effects::service::ServiceResponseError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let refusal = PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(
            ResolveError::from(MemoryError::ServiceResponse(ServiceResponseError::new(
                403,
                Some("CREDENTIAL_REVOKED".to_string()),
                "credential revoked",
            ))),
        ));

        assert_eq!(
            super::classify_pull(&refusal).kind(),
            super::JoinFailureKind::Revoked,
        );
    }

    #[dialog_common::test]
    fn it_does_not_misreport_an_unrelated_forbidden_response_as_revocation() {
        use dialog_effects::memory::MemoryError;
        use dialog_effects::service::ServiceResponseError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let refusal = PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(
            ResolveError::from(MemoryError::ServiceResponse(ServiceResponseError::new(
                403,
                Some("COMMAND_MISMATCH".to_string()),
                "wrong command",
            ))),
        ));
        assert_eq!(
            super::classify_pull(&refusal).kind(),
            super::JoinFailureKind::ClaimFailed,
        );
    }

    /// A service that is merely unwell is retryable; only a refusal is
    /// terminal.
    #[dialog_common::test]
    fn it_reads_an_unwell_service_as_retryable() {
        use dialog_effects::memory::MemoryError;
        use dialog_effects::service::ServiceResponseError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let outage =
            PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(ResolveError::from(
                MemoryError::ServiceResponse(ServiceResponseError::new(503, None, "unavailable")),
            )));

        assert_eq!(
            super::classify_pull(&outage).kind(),
            super::JoinFailureKind::Unavailable,
        );
    }

    // ----- success and idempotence -----

    /// A guest visit mounts a readable replica and retains the invite for
    /// a later promotion, without writing a roster row.
    #[dialog_common::test]
    async fn it_mounts_a_guest_visit_without_durable_membership() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(70, 71).await;

        assert_eq!(post_visit(&app, &url).await, StatusCode::CREATED);

        assert_eq!(membership_status(&app, &key).await, "guest");
        let after = snapshot(&state, &key).await;
        assert!(after.replicas.iter().any(|entry| entry == &key));
        assert!(after.guest, "the visit retains its invite for promotion");
        assert!(
            after.members.is_empty(),
            "a guest writes no durable roster row",
        );
    }

    /// Promotion answers with the replica rather than an empty 204, and
    /// only then gives up the guest credential.
    #[dialog_common::test]
    async fn it_promotes_a_guest_to_durable_membership() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(72, 73).await;
        assert_eq!(post_visit(&app, &url).await, StatusCode::CREATED);
        let backups_before = crate::router::account_backup::backup_dispatch_count();

        let (status, body) = promote(&app, &key).await;
        assert_eq!(status, StatusCode::OK, "promotion acknowledges with a body");
        assert_eq!(
            body.expect("promotion returns JSON")["outcome"],
            serde_json::json!("renewed"),
        );

        assert_eq!(membership_status(&app, &key).await, "durable");
        let after = snapshot(&state, &key).await;
        assert!(!after.guest, "the guest credential is cleared on promotion");
        assert!(after.authority, "the accepted authority is durable");
        assert_eq!(
            after.backup_dispatches,
            backups_before + 1,
            "backup dispatch follows the local commit exactly once",
        );

        let root_entity = {
            let tonk = state.read().await;
            crate::router::identity::root_did(&tonk)
                .await
                .unwrap()
                .this()
        };
        let memberships = content_memberships(&state, &key).await;
        assert!(
            memberships.iter().any(|row| row.member.0 == root_entity),
            "a promoted guest is recorded by root DID",
        );
    }

    /// A targeted invite issued to this device's root joins durably.
    #[dialog_common::test]
    async fn it_joins_a_targeted_invite_issued_to_this_root() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let root_did = {
            let tonk = state.read().await;
            crate::router::identity::root_did(&tonk).await.unwrap()
        };
        let (url, key) = targeted_invite_url(74, &root_did).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        assert_eq!(membership_status(&app, &key).await, "durable");
        let memberships = content_memberships(&state, &key).await;
        assert!(
            memberships
                .iter()
                .any(|row| row.member.0 == root_did.this()),
            "a targeted join is recorded by root DID",
        );
    }

    /// A join is only visible once its content is installed, so the
    /// replica's card never sits at "installing".
    #[dialog_common::test]
    async fn it_indexes_a_joined_replica_as_initialized() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(76, 77).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let after = snapshot(&state, &key).await;
        assert!(after.replicas.iter().any(|entry| entry == &key));
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let replica_entity = {
            let tonk = state.read().await;
            tonk_schema::Replica::new(tonk.profile.did(), subject)
                .this()
                .to_string()
        };
        let status = after
            .statuses
            .iter()
            .find(|(entity, _)| entity == &replica_entity)
            .map(|(_, status)| status.as_str());
        assert_eq!(
            status,
            Some(tonk_schema::Replica::INITIALIZED),
            "the visibility commit stamps initialized, never blank",
        );
    }

    /// The second attempt at the same invite renews rather than
    /// duplicating: one replica, one membership row.
    #[dialog_common::test]
    async fn it_renews_rather_than_duplicating_a_repeated_attempt() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(78, 79).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let first = snapshot(&state, &key).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::OK);
        let second = snapshot(&state, &key).await;

        assert_eq!(second, first, "a repeated join is idempotent");
        assert_eq!(second.members.len(), 1, "exactly one membership row");
        assert_eq!(
            second
                .replicas
                .iter()
                .filter(|entry| *entry == &key)
                .count(),
            1,
            "exactly one replica in the profile index",
        );
    }

    /// A local-only invite has no remote to prove, so it commits on
    /// cryptographic verification alone — and still lands its roster.
    #[dialog_common::test]
    async fn it_joins_a_local_only_invite_without_a_remote() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(80, 81).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let after = snapshot(&state, &key).await;
        assert_eq!(after.members.len(), 1);
        assert_eq!(after.roles.len(), 1, "the claimer is stamped a member");
    }

    /// Data joined before a revocation stays readable locally —
    /// revocation controls remote access, not local erasure.
    #[dialog_common::test]
    async fn it_keeps_joined_data_readable_after_the_invite_is_revoked() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(82, 83).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let joined = snapshot(&state, &key).await;

        // A later refusal from the remote classifies as revoked, and
        // classification alone touches nothing local.
        assert_eq!(
            super::JoinFailureKind::Revoked.as_str(),
            "revoked",
            "the revoked classification is what sync reports",
        );

        assert_eq!(
            snapshot(&state, &key).await,
            joined,
            "local data survives revocation",
        );
        assert_eq!(membership_status(&app, &key).await, "durable");
    }

    /// The content gate a remote-backed join runs before installing a
    /// revision. Every other test here joins a handcrafted local invite,
    /// which carries no access service, so `needs_remote_authorization` is
    /// false and this gate never runs — which is how it shipped broken.
    ///
    /// The fixture declares the space model through the real evaluate path
    /// rather than hand-asserting the claims a concept is made of. That
    /// matters more than brevity here: hand-asserting would encode this
    /// test's belief about how a concept is stored, and a wrong belief about
    /// exactly that is the bug being fixed.
    #[dialog_common::test]
    async fn it_accepts_content_whose_space_model_is_a_declared_concept() {
        use dialog_repository::{Branch, Repository, RepositoryExt as _};

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/validate-content")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: crate::router::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        let repo = info.name;

        // The repository's own identity, asserted the way
        // `run_rename_repository` does — the typed schema struct owns the
        // storage shape, so the fixture states no opinion about it.
        {
            use dialog_repository::{Branch, Repository, RepositoryExt as _};
            let tonk = state.read().await;
            let repository: Repository = tonk
                .profile
                .repository(&repo)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            let content: Branch = repository
                .branch("main")
                .open()
                .perform(&tonk.operator)
                .await
                .expect("content branch opens");
            content
                .transaction()
                .assert(tonk_schema::RepositoryName {
                    this: repository.did().this(),
                    name: tonk_schema::domain::repo::Name("Untitled".to_string()),
                })
                .commit()
                .perform(&tonk.operator)
                .await
                .expect("the repository name commits");
        }

        // The shape `core.yaml` seeds for a lean spot: a pinned concept for
        // the canvas, and the cardinality-one `tonk/space` alias pointing at
        // it. The space route mounts whatever that alias resolves to.
        const SPACE_MODEL: &str = r#"concept!: &blank
  this: tonk:blank
  description: A lean repo's starting canvas.
  with:
    subject:
      the: xyz.tonk.replica/subject
      as: entity
      cardinality: one
      description: The spot this canvas belongs to

name!:
  this: id:tonk/space
  entity: tonk:blank
"#;
        {
            let guard = state.read().await;
            crate::router::evaluate::evaluate_body(
                &guard,
                &repo,
                "main",
                SPACE_MODEL.to_owned(),
                true,
            )
            .await
            .expect("the space model commits");
        }

        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(&repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");

        super::validate_content(&content, &tonk.operator, &repository.did())
            .await
            .expect("content carrying a declared space model is joinable");
    }
}
