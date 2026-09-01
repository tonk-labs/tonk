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
//!       -> install staged content -> commit authority/profile state
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

use ::axum::{Json, extract::State, http::StatusCode};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Changes, Entity, Statement as _, Value};
use dialog_capability::access::{AuthorizeError, Prove, Retain};
use dialog_capability::{Fork, Provider, Subject};
use dialog_common::ConditionalSync;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::{Attest, Identify};
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
use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
use tonk_common::log;
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::{
    Invitation, InvitationExecution, InvitedVia, MemberName, MemberRole, Membership, Replica,
    RepositoryName, SeedKind, prelude::DidExt as _,
};
use tonk_worker_api::JoinFailureKind;
use zeroize::Zeroizing;

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
const NAME_REFERENT: &str = "db.name/referent";

/// Marker claim every concept declared on a branch carries. Its presence is
/// what separates "the name resolves" from "the model behind it exists".
///
/// Deliberately the marker and not `db.meta/source`: `source` is the
/// descriptor materialised as JSON by the concept-of-concept query, which
/// reconstructs it from the branch's facts. Nothing ever asserts it, so a raw
/// claims read for `source` answers "no" for every concept that has ever
/// existed (see `tonk_schema::concept::concept_of_concept_descriptor`). The
/// marker is the fact the query itself enumerates concepts by.
const CONCEPT_MARKER: &str = "db.meta/concept";

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
    + Provider<Attest>
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
        + Provider<Attest>
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
        let detail = detail.into();
        // The only place the detail is ever emitted. Every recipient-facing
        // surface shows the kind's fixed copy so an upstream body cannot
        // leak, which leaves a failed join otherwise undiagnosable: one
        // sentence on screen standing for any of a dozen causes. Logged at
        // construction so every constructor is covered by one line.
        log!("join failed ({}): {detail}", kind.as_str());
        Self { kind, detail }
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

    fn refused(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::Refused, detail)
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
            JoinFailureKind::Refused => TonkWorkerError::Upstream {
                status: 403,
                code: Some("JOIN_REFUSED".to_string()),
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

    use super::{AuthorizeError, JoinFailure, JoinFailureKind};
    use crate::TonkWorkerError;

    const KINDS: [JoinFailureKind; 6] = [
        JoinFailureKind::Malformed,
        JoinFailureKind::AudienceMismatch,
        JoinFailureKind::Revoked,
        JoinFailureKind::Unavailable,
        JoinFailureKind::Refused,
        JoinFailureKind::ClaimFailed,
    ];

    #[dialog_common::test]
    fn it_fixes_the_message_for_every_kind() {
        let messages: Vec<&str> = KINDS.iter().map(|kind| kind.message()).collect();
        assert_eq!(
            messages,
            vec![
                "This share link is invalid.",
                "This invite was issued to a different identity.",
                "This invite has been revoked.",
                "Tonk could not reach this space. Try again.",
                "This space's host declined the invite. Its owner needs to check the space's plan.",
                "Tonk could not join this space.",
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
                "refused",
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

    /// A policy refusal is the REMOTE's verdict on a chain that proved
    /// out, not a local breakage. It landed in the `_` catch-all and was
    /// reported as `claim-failed` ("Tonk could not join this space"),
    /// which blames this device for a decision taken on the server —
    /// the real one being an unprovisioned subject, which no amount of
    /// retrying or re-inviting fixes.
    #[dialog_common::test]
    fn it_reports_a_policy_refusal_as_the_remotes_verdict() {
        let failure = super::classify_authorization(&AuthorizeError::PolicyViolation {
            predicate: "subject is provisioned by an active customer".to_string(),
        });

        assert_eq!(failure.kind(), JoinFailureKind::Refused);
        assert!(
            !failure.kind().retryable(),
            "the same request will be refused again until the owner acts"
        );

        let TonkWorkerError::Upstream { status, code, .. } = TonkWorkerError::from(failure) else {
            panic!("a refusal is the upstream's answer, not an internal fault");
        };
        assert_eq!(status, 403);
        assert_eq!(code.as_deref(), Some("JOIN_REFUSED"));
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
}

/// A parsed, audience-verified invite and everything the later stages
/// need to decide what this join has to prove.
///
/// Holds the invite, whose open form carries a bearer seed, so it
/// deliberately has no derived `Debug`.
pub(crate) struct PreparedJoin {
    invite: Invite,
    /// Derived from the chain *as parsed*, before any redelegation
    /// changes the leaf.
    invitation: Invitation,
    /// Audience metadata recorded beside the invitation.
    invitation_execution: InvitationExecution,
    subject: Did,
    key: String,
    /// The account the chain terminates at: the passkey root when one is
    /// linked, the onboarding account otherwise.
    member: Did,
    /// The candidate chain, ending at `member`.
    chain: DelegationChain,
    /// The account's grant to this device, which the staged proof walk
    /// composes the candidate chain onto.
    device_grant: DelegationChain,
    /// Access service the invite carried, if any.
    remote_url: Option<String>,
    /// Explicit revocation relay the invite carried, if any.
    revocation_url: Option<String>,
    /// A replica for this subject is already recorded in the profile.
    existing: bool,
}

impl std::fmt::Debug for PreparedJoin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedJoin")
            .field("subject", &self.subject)
            .field("member", &self.member)
            .field("existing", &self.existing)
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
        self.invitation.inviter.0 == self.member.this()
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
    /// The candidate chain staging accepted.
    chain: DelegationChain,
    /// Staged content to publish, when the attempt produced a head.
    installable: Option<StagedContent>,
}

/// The staged head to publish locally, and how much of it the durable
/// replica has to be handed up front.
struct StagedContent {
    /// Staged branch the revision lives on.
    branch: Branch,
    /// The exact revision to publish.
    revision: Revision,
    /// The head the remote served, before this claim's facts were staged
    /// on top of it.
    ///
    /// `Some` only when everything reachable from it is still reachable
    /// *through the durable replica's own remote* — a fresh remote-backed
    /// join, whose staged branch started empty and therefore holds exactly
    /// what the remote handed back. Then the install carries only the nodes
    /// this claim created and the rest is read lazily, the way every other
    /// path in the worker already reads a synced branch.
    ///
    /// `None` when there is nowhere to read the remainder back from: a
    /// local-only invite has no remote, and a renewal's staged branch is a
    /// merge of local and remote content whose nodes exist whole in
    /// neither store.
    remote_head: Option<Revision>,
}

impl std::fmt::Debug for StagedJoin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StagedJoin")
            .field("prepared", &self.prepared)
            .field(
                "installable",
                &self.installable.as_ref().map(|content| &content.revision),
            )
            .finish_non_exhaustive()
    }
}

/// Redeem an invite URL to this device's account.
#[wasm_compat]
pub async fn join(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let tonk = state.write().await;
    let outcome = join_invite(&tonk, &body.url).await?;
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

/// The one join operation: the HTTP join and the `tonk:join` command
/// both run through it.
///
/// Every join is durable, to whatever account this device has: the
/// passkey root when one is linked, the onboarding account otherwise.
/// Accreditation re-roots the membership from the custodied invite
/// seed, so a join never has to be redone.
///
/// Nothing durable changes before [`commit_join`], and everything
/// [`commit_join`] does is either local or already proven, so a failure
/// at any earlier stage leaves the recipient's profile, repository list,
/// roster, and claim backup exactly as they were.
pub(crate) async fn join_invite(tonk: &TonkState, url: &str) -> Result<JoinOutcome, JoinFailure> {
    // Per-phase wall clock, logged on success: the staging phase is the
    // network-bound one (pull + validation + roster reads against the
    // remote), so a slow join in the field can be attributed to the
    // network or to local work without reproducing it.
    let started = web_time::Instant::now();
    let prepared = prepare_join(tonk, url).await?;
    let prepared_at = web_time::Instant::now();
    let staged = stage_join(tonk, prepared).await?;
    let staged_at = web_time::Instant::now();
    let outcome = commit_join(tonk, staged).await?;
    log!(
        "join: prepared {}ms, staged {}ms, committed {}ms",
        prepared_at.duration_since(started).as_millis(),
        staged_at.duration_since(prepared_at).as_millis(),
        staged_at.elapsed().as_millis()
    );
    Ok(outcome)
}

/// Parse the invite, verify it is addressed to this identity, and build
/// the candidate chain. Reads only, except that a device joining before
/// it has any account mints its onboarding account here.
async fn prepare_join(tonk: &TonkState, url: &str) -> Result<PreparedJoin, JoinFailure> {
    let invite = Invite::parse_url(url)
        .await
        .map_err(|error| JoinFailure::malformed(format!("invite did not parse: {error}")))?;

    // Derived from the chain as parsed — a claim pushes a redelegation
    // and changes the leaf. Guaranteed `Some` by the `Invite` invariant
    // (the chain has a specific subject).
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");

    let open = matches!(&invite.audience, InviteAudience::Open { .. });
    let invitation_execution =
        InvitationExecution::new(&invitation, if open { "open" } else { "scoped" });
    let subject = invite.subject().clone();
    let key = subject.repo_key().to_owned();
    let remote_url = invite.remote_url.as_ref().map(url::Url::to_string);
    let revocation_url = invite.revocation_url.as_ref().map(url::Url::to_string);

    let existing = find_replica_for_subject(tonk, &subject)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to look up the local replica: {error}"))
        })?;
    // The membership terminates at this device's account, and the
    // account's grant to the device is what the proof walk composes the
    // claim onto. Both come from the same place, so a device cannot end
    // up holding a chain it cannot prove for.
    let (member, device_grant) = crate::router::account::current_account(tonk)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to resolve the joining account: {error}"))
        })?;

    // Audience: an open invite redelegates to whoever redeems it; a
    // targeted one only ever redeems for the DID it names.
    let claimed = invite.clone().claim(&member).await.map_err(|error| {
        if open {
            JoinFailure::claim_failed(format!("invite did not extend: {error}"))
        } else {
            JoinFailure::audience_mismatch(format!("invite is not for this account: {error}"))
        }
    })?;

    Ok(PreparedJoin {
        invite,
        invitation,
        invitation_execution,
        subject,
        key,
        member,
        chain: claimed.chain,
        device_grant,
        remote_url,
        revocation_url,
        existing,
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
    // `profile -> operator` delegation is already in the pool; the claim
    // composes onto the `account -> device` grant.
    staging.retain(tonk, prepared.device_grant.clone()).await?;
    let chain = prepared.chain.clone();
    staging.retain(tonk, chain.clone()).await?;

    let branch = staging
        .mount(tonk, &prepared.subject, prepared.remote_url.as_deref())
        .await?;

    // A renewal starts from the exact local head, then merges the remote into
    // that staged copy. Starting empty would discard unpushed local content.
    if prepared.existing {
        copy_existing_to_stage(tonk, &prepared, &branch, staging.operator()).await?;
    }

    // What the remote served, captured before this claim writes on top of
    // it. Only a fresh join can use it as an install base: a renewal staged
    // its local head first, so the merge below produces nodes the remote
    // cannot serve back.
    let mut remote_head = None;
    if prepared.needs_remote_authorization() {
        pull_staged(&branch, staging.operator()).await?;
        validate_content(&branch, staging.operator(), &prepared.subject).await?;
        if !prepared.existing {
            remote_head = branch.revision();
        }
    }

    // Every claim, including a renewal, stages roster/provenance/name into
    // the exact revision that will be installed before authority is saved.
    let (changes, _already_claimed) = claim_changes(
        tonk,
        &branch,
        staging.operator(),
        &prepared.invitation,
        &prepared.invitation_execution,
        &prepared.member,
        &prepared.subject,
    )
    .await?;
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
    if prepared.needs_remote_authorization() {
        validate_content(&branch, staging.operator(), &prepared.subject).await?;
    }

    // Every claim and every existing replica has an exact staged head to
    // install.
    let installable = branch.revision().map(|revision| StagedContent {
        branch,
        revision,
        remote_head,
    });

    Ok(StagedJoin {
        prepared,
        staging,
        chain,
        installable,
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
    install_revision_between(
        &source,
        &repository,
        &revision,
        &tonk.operator,
        destination_env,
    )
    .await?;
    destination
        .reset(revision)
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
/// place. The backup and the caller's navigation follow.
async fn commit_join(tonk: &TonkState, staged: StagedJoin) -> Result<JoinOutcome, JoinFailure> {
    let StagedJoin {
        prepared,
        staging,
        chain,
        installable,
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
    if let Some(content) = installable {
        install_revision(
            tonk,
            &content,
            staging.operator(),
            &repository,
            prepared.needs_remote_authorization(),
        )
        .await?;
    }

    save_authority(tonk, &prepared, chain.clone()).await?;
    retain_claim_authority(tonk, &prepared.key, &chain).await;

    if prepared.installs_replica() {
        record_initialized_replica_in_profile(tonk, &prepared.subject)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to index the replica: {error}"))
            })?;
    }

    {
        // The account directory is how another of this account's
        // devices recovers this claim: alongside the membership
        // facts, record the mount configuration the invite carried,
        // or a fresh sign-in lists a space it can never mount.
        // Renewals record too. Best-effort, and strictly after the local
        // commit — the
        // join is already complete.
        match invite_configuration(
            &prepared.subject,
            prepared.remote_url.as_deref(),
            prepared.revocation_url.as_deref(),
        ) {
            Ok(configuration) => {
                super::repository::record_space_mount(
                    tonk,
                    &prepared.subject,
                    &configuration,
                    None,
                )
                .await;
            }
            Err(error) => {
                log!("claimed space directory record skipped: {error}");
            }
        }
        // The membership hangs off the invite principal, so the
        // account keeps that principal's seed: at rotation it mints
        // `principal -> new root` itself instead of needing a fresh
        // invite. A targeted invite carries no seed and its chain is
        // rooted at the account already. Best-effort, like the
        // directory record above.
        if let InviteAudience::Open { seed } = &prepared.invite.audience {
            super::account_state::custody_seed(
                tonk,
                prepared.invite.chain.audience(),
                SeedKind::Invite,
                Zeroizing::new(*seed),
            )
            .await;
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

/// Retain the claimed chain into the space's content branch, so the hop
/// that admits this member is provable from the space itself.
///
/// The invite hop is already there (the inviter retained it at mint);
/// what an open invite adds at claim time is the hop from the invite's
/// ephemeral audience to this member, and without it the space knows
/// this member only through the invite everyone else also came in
/// through. An admin removing one member needs that member's own hop:
/// revoking the shared invite hop would remove everyone who used it.
///
/// Best-effort: the join is complete once the authority is saved
/// locally, and a member whose hop did not land here is still a member,
/// just not individually removable until it does.
pub(super) async fn retain_claim_authority(tonk: &TonkState, key: &str, chain: &DelegationChain) {
    let session = match tonk
        .reactor
        .repository(key)
        .branch(DEFAULT_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(error) => {
            log!("claimed chain not retained on '{key}': {error}");
            return;
        }
    };
    if let Err(error) = session
        .handle()
        .delegations()
        .retain(UcanDelegation(chain.clone()))
        .perform(&tonk.operator)
        .await
    {
        log!("claimed chain not retained on '{key}': {error}");
    }
}

/// Save the authority this join accepted into the durable certificate
/// store.
///
/// Idempotent at the dialog layer — re-saving the same chain is a no-op,
/// re-saving an extended one adds a fresh proof. Either way the
/// recipient's effective access can only grow, never shrink, by joining.
///
async fn save_authority(
    tonk: &TonkState,
    prepared: &PreparedJoin,
    chain: DelegationChain,
) -> Result<(), JoinFailure> {
    let prefix_bytes = chain.to_bytes().map_err(|error| {
        JoinFailure::claim_failed(format!(
            "failed to serialize the accepted authority: {error}"
        ))
    })?;
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to save the accepted authority: {error}"))
        })?;
    tonk.profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{}", prepared.subject))
        .save(prefix_bytes)
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!(
                "failed to persist the accepted root prefix: {error}"
            ))
        })?;
    Ok(())
}

/// Give the durable repository the staged revision, then publish it as the
/// branch head and verify what landed.
///
/// How much content moves depends on what the durable replica can read for
/// itself. A synced replica reads through a remote-backed index on every
/// ordinary path — `select`, `session`, `commit`, `pull`, `blob` — so a
/// fresh remote-backed join only has to carry the nodes this claim created;
/// the rest of the tree resolves on demand against the same `origin` the
/// invite named. Copying it eagerly instead meant one authorized round trip
/// per node and per blob, strictly sequential, before the recipient could
/// see anything (~500 requests and ~110s on a modest space), which is a full
/// replication masquerading as a join.
///
/// Everything else still moves whole, because there is nowhere to read the
/// remainder back from: see [`StagedContent::remote_head`].
///
/// Not an export/import: that would mint a synthetic commit, drop the
/// history the remote handed back, and leave blobs behind. Both paths write
/// blocks without publishing a head, so the destination stays unreadable
/// until the `reset` below, and the head it then carries is byte-identical
/// to the one that was validated.
async fn install_revision(
    tonk: &TonkState,
    content: &StagedContent,
    source_env: &staging::StagedOperator,
    repository: &Repository<Credential>,
    validate_remote_content: bool,
) -> Result<(), JoinFailure> {
    let StagedContent {
        branch: source,
        revision,
        remote_head,
    } = content;
    let revision = revision.clone();

    match remote_head {
        Some(base) => {
            let nodes = install_claim_nodes(tonk, source, source_env, &revision, base).await?;
            log!("join: installed {nodes} claim node(s); the rest reads through the remote");
        }
        None => {
            install_revision_between(source, repository, &revision, source_env, &tonk.operator)
                .await?;
        }
    }
    let installed = revision.clone();

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
    // Re-read the content through the durable replica — but only when the
    // durable replica actually holds it. On the lazy path most of the tree
    // is still upstream, so this query would go to the remote, and the
    // authority to ask has deliberately not been saved yet
    // (`save_authority` runs after this returns). Nothing is lost: the
    // staged pass validated this exact revision, and the head-equality
    // check above is what proves the reset landed.
    if validate_remote_content && remote_head.is_none() {
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
    // Deliver the fresh snapshot the refresh scheduled for any
    // subscriptions the rebind carried over — a live view left waiting
    // for the next commit waits forever on a branch nothing edits.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    Ok(())
}

/// Copy `revision`'s complete reachable tree and referenced blobs from
/// `source_env`'s storage into `destination_env`'s without publishing a
/// head — dialog's snapshot export/import (the successor of the backported
/// `Branch::install`). Reads fall back to `branch`'s remote upstream when
/// it tracks one, so a sparse source replica still exports a complete
/// snapshot.
async fn install_revision_between<Source, Destination>(
    branch: &Branch,
    repository: &Repository<Credential>,
    revision: &Revision,
    source_env: &Source,
    destination_env: &Destination,
) -> Result<(), JoinFailure>
where
    Source: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<dialog_effects::blob::Read>
        + Provider<dialog_effects::blob::Import>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, dialog_effects::blob::Read>>
        + ConditionalSync
        + 'static,
    Destination: Provider<Put> + Provider<dialog_effects::blob::Import> + ConditionalSync + 'static,
{
    use dialog_repository::{RepositoryMemoryExt as _, Upstream};

    let mut export = repository.snapshot(revision.clone()).export();
    if let Some(Upstream::Remote { remote, .. }) = branch.upstream() {
        let remote = branch
            .subject()
            .remote(remote)
            .load()
            .perform(source_env)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to read the source remote: {error}"))
            })?;
        export = export.download(remote);
    }
    let items = export.perform(source_env);
    repository
        .import(items)
        .perform(destination_env)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to install the staged content: {error}"))
        })?;
    Ok(())
}

/// Copy only the tree nodes this claim created — the difference between
/// what the remote served and the staged head committed on top of it.
///
/// The nodes left behind are exactly those reachable from `base`, which is
/// the head the remote handed back and can hand back again. The durable
/// replica tracks that same remote, so its own reads resolve them on demand.
///
/// Blobs are skipped for the same reason and are never novel here anyway: a
/// claim writes roster facts, not blobs.
///
/// This is dialog's `Branch::install` with a real diff base. That command
/// hardcodes `Index::empty()`, which makes every node in the tree novel and
/// turns the walk into a full replication.
/// Returns how many nodes were copied — the number this change exists to
/// keep small, and the one a regression would blow up.
async fn install_claim_nodes<Source>(
    tonk: &TonkState,
    source: &Branch,
    source_env: &Source,
    revision: &Revision,
    base: &Revision,
) -> Result<usize, JoinFailure>
where
    Source: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Fork<RemoteSite, Get>>
        + ConditionalSync
        + 'static,
{
    use dialog_artifacts::tree::TreeStorageBridge;
    use dialog_common::Blake3Hash;
    use dialog_effects::archive::prelude::CatalogExt as _;
    use dialog_repository::{
        Index, NetworkedIndex, RepositoryArchiveExt as _, RepositoryMemoryExt as _, Upstream,
    };
    use dialog_search_tree::{ContentAddressedStorage, TreeDifference};

    // The staged branch reads through its own upstream, so a node the diff
    // needs but the volatile pool never fetched still resolves.
    let remote = match source.upstream() {
        Some(Upstream::Remote { remote, .. }) => Some(
            source
                .subject()
                .remote(remote)
                .load()
                .perform(source_env)
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!("failed to read the staged remote: {error}"))
                })?,
        ),
        _ => None,
    };

    let catalog = source.archive().index();
    let index = NetworkedIndex::new(source_env, catalog.clone(), remote);
    let storage = ContentAddressedStorage::new(TreeStorageBridge(index));
    let from = Index::from_hash(Blake3Hash::from(*base.tree.hash()));
    let to = Index::from_hash(Blake3Hash::from(*revision.tree.hash()));

    let difference = TreeDifference::compute(&from, &to, &storage, &storage)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to diff the staged claim: {error}"))
        })?;
    let nodes = difference.novel_nodes();
    futures_util::pin_mut!(nodes);
    let mut installed = 0usize;
    while let Some(node) = nodes.next().await {
        let node = node.map_err(|error| {
            JoinFailure::claim_failed(format!("staged claim node did not read back: {error}"))
        })?;
        catalog
            .clone()
            .put(node.buffer().clone())
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to install a claim node: {error}"))
            })?;
        installed += 1;
    }
    Ok(installed)
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
    // The typed reasons survive the pull's error chain (dialog carries
    // `AuthorizeError` / `Rejection` intact from the service boundary),
    // so classification is a match, not a code-table lookup.
    if let Some(authorization) = crate::router::sync::authorization_reason(error) {
        return classify_authorization(authorization);
    }
    if let Some(rejection) = crate::router::sync::rejection_reason(error) {
        return JoinFailure::unavailable(format!("remote answered: {rejection}"));
    }
    JoinFailure::unavailable("the remote could not be reached")
}

/// Map one authorization verdict to the failure the user is shown.
///
/// Split from [`classify_pull`] so it can be tested without building a
/// `PullError`: the mapping is the part that decides what the page says,
/// and one arm landing in the wrong bucket is invisible until someone
/// reads a log.
fn classify_authorization(authorization: &AuthorizeError) -> JoinFailure {
    match authorization {
        AuthorizeError::Revoked { .. } => JoinFailure::revoked("remote access has been revoked"),
        AuthorizeError::InvalidAudience { .. } | AuthorizeError::UnprovenSubject { .. } => {
            JoinFailure::audience_mismatch(format!("remote refused: {authorization}"))
        }
        AuthorizeError::Unavailable { .. } | AuthorizeError::UnavailableProof { .. } => {
            JoinFailure::unavailable(format!("remote answered: {authorization}"))
        }
        // The chain proved out and the remote evaluated it; either a
        // policy predicate on the delegation said no, or the remote
        // declined to serve the subject at all. Nothing on this device
        // is wrong, so `claim-failed` — which says the local claim
        // broke, and reads as "Tonk could not join this space" — pointed
        // the user at the wrong thing entirely.
        AuthorizeError::PolicyViolation { .. } | AuthorizeError::Declined { .. } => {
            JoinFailure::refused(format!("remote refused: {authorization}"))
        }
        _ => JoinFailure::claim_failed(format!("remote refused: {authorization}")),
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
        Some(Ok(artifact)) => match artifact.value() {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(JoinFailure::unavailable(format!(
                "staged content did not decode: {error}"
            ))),
        },
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
    invitation_execution: &InvitationExecution,
    member: &Did,
    subject: &Did,
) -> Result<(Changes, bool), JoinFailure> {
    let membership = Membership::new(member.clone(), subject.clone());

    // The three roster reads land in three separate index regions, and on
    // a staged (network-backed) branch each one descends the tree cold —
    // paying its round trips in full. Run them concurrently so a join
    // pays one descent of latency instead of three back to back; the
    // reads are independent and read-only, so ordering carries nothing.
    // On the already-claimed early return below the role/name results go
    // unused — that is the renewal re-claim path, where the staged branch
    // was seeded from the local head and the extra reads are warm.
    let stamps_read = async {
        branch
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
            })
    };
    let roles_read = async {
        branch
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
            })
    };
    // Guard the name too: a linked device may resolve a different local
    // display name, but a later sequential join must not overwrite an
    // existing roster rename. This read-then-write guard is intentionally
    // not a linearizable first-writer lock for concurrent claims.
    let names_read = async {
        branch
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
            })
    };
    let (stamps, roles, names): (Vec<InvitedVia>, Vec<MemberRole>, Vec<MemberName>) =
        futures_util::try_join!(stamps_read, roles_read, names_read)?;

    let already_stamped = stamps.iter().any(|stamp| stamp.this == *membership.this());
    let already_claimed = stamps
        .iter()
        .any(|stamp| stamp.this == *membership.this() && stamp.invitation.0 == *invitation.this());
    if already_claimed {
        return Ok((Changes::new(), true));
    }

    let already_roled = roles.iter().any(|role| role.this == *membership.this());
    let already_named = membership_has_name(&names, &membership);

    // A member claiming their own invite is not provenance.
    let self_invite = invitation.inviter.0 == member.this();

    let mut changes = Changes::new();
    invitation.clone().assert(&mut changes);
    invitation_execution.clone().assert(&mut changes);
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

    let configuration = invite_configuration(subject, remote_url, revocation_url)?;
    finish_mount(tonk, &repository, &key, configuration).await?;
    Ok(repository)
}

/// The repository configuration an invite describes: a single `main`
/// branch, plus an `origin` remote tracking the invite's/space's access
/// service if one was attached — mirroring what
/// `PUT /api/repository/{name}` writes.
fn invite_configuration(
    subject: &Did,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
) -> Result<RepositoryConfiguration, TonkWorkerError> {
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
    Ok(configuration)
}

/// Mount a space with a full, caller-supplied configuration — the
/// directory-adoption entry point, where the configuration is rebuilt
/// from account-DB facts and applied verbatim so a non-default setup
/// replicates identically.
pub(crate) async fn mount_replica_with_configuration(
    tonk: &TonkState,
    subject: &Did,
    configuration: RepositoryConfiguration,
) -> Result<Repository<Credential>, TonkWorkerError> {
    let key = subject.repo_key().to_owned();
    if super::account_state::is_account_key(tonk, &key).await {
        return Err(TonkWorkerError::Forbidden(
            "account system repository cannot be mounted as a user space".to_string(),
        ));
    }
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
    finish_mount(tonk, &repository, &key, configuration).await?;
    Ok(repository)
}

/// The shared tail of every mount: record the local meta from the
/// configuration. No display name to seed — a joined/adopted repo's
/// name lives in the shared content branch and arrives over the pull;
/// the helper only uses the name for log context, so the routing key
/// stands in.
async fn finish_mount(
    tonk: &TonkState,
    repository: &Repository<Credential>,
    key: &str,
    configuration: RepositoryConfiguration,
) -> Result<(), TonkWorkerError> {
    record_replica_local_meta(tonk, repository, key, &configuration).await?;
    Ok(())
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
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
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
            // The invite principal's seed is custodied under the account
            // as part of the join. A linked device whose root record
            // predates the encryption key asks the originating page for a
            // passkey assertion here, before the state lock is taken.
            if let Err(error) =
                crate::router::custody::ensure_recipient(env.state(), env.client()).await
            {
                log!("join refused: {error}");
                return;
            }
            run_join(&env, command).await;
        })
    }
}

/// Whether a `/join` URL carries an invite at all.
///
/// The delegation chain rides in `access`, so its presence is what
/// separates "redeem this" from "someone opened /join to paste a link".
/// Deliberately a query test and not a parse: a malformed or truncated
/// invite IS an attempt and must still fail loudly with its reason,
/// rather than being silently treated as an empty visit.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn carries_invite(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        parsed
            .query_pairs()
            .any(|(key, value)| key == "access" && !value.is_empty())
    })
}

/// Drop this join's own overlay facts, and only those (scoped clear).
///
/// The branch overlay is SHARED: the tab's `tonk:site` facts (path,
/// route, concept) live there too, and they are what every view on the
/// page resolves through. A blanket `clear_overlay()` therefore wiped
/// the site out from under the page on every `/join` mount — the site
/// display lost its entity, fell back to its pending spinner, and
/// nothing downstream ever rendered. Scope the clear to the join's own
/// entities, exactly as the site re-stamp does with its own.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn clear_join_overlay(session: &dialog_reactor::BranchSession, status: &dialog_artifacts::Entity) {
    let status = status.clone();
    session
        .state
        .retain_overlay_entities(move |overlaid| retains_overlay_entity(overlaid, &status));
}

/// Whether an overlaid entity SURVIVES a join's scoped clear.
///
/// Everything but the join's own status entity does. Split out from
/// [`clear_join_overlay`] so the rule can be tested off-wasm: it is the
/// whole contract, and getting it backwards is invisible until a page
/// silently stops rendering.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn retains_overlay_entity(
    overlaid: &dialog_artifacts::Entity,
    status: &dialog_artifacts::Entity,
) -> bool {
    overlaid != status
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

    // A `/join` opened with no invite in its URL is not a failed attempt
    // — it is someone who arrived holding a link they have not pasted
    // yet. Asserting `pending` here is what made the paste form flash
    // and vanish behind a spinner that waited on nothing. Claiming
    // nothing leaves the view in its own inviteless state rather than
    // flashing a spinner for an invite that will never arrive. A URL
    // that carries an invite is untouched by this and proceeds exactly
    // as before.
    if !carries_invite(&command.url.0) {
        clear_join_overlay(&session, &status_entity);
        tonk.reactor.schedule_poll(Arc::clone(&session.state));
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        return;
    }

    // Pending: a fresh attempt clears any prior status, then marks
    // pending. Schedule a poll so the view shows "Joining…".
    clear_join_overlay(&session, &status_entity);
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

    // The same operation the HTTP join runs, so the content behind the
    // redirect is proven before the redirect fires.
    match join_invite(&tonk, &url).await {
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
            clear_join_overlay(&session, &status_entity);
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
        Err(failure) => {
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

/// The inviteless-`/join` guard, pinned on every target: it decides
/// whether the route claims or offers its paste form, and getting it
/// wrong flashes the form before a spinner for an invite that will
/// never arrive.
#[cfg(test)]
mod invite_presence_tests {
    use super::carries_invite;

    #[test]
    fn it_separates_a_redeemable_invite_from_an_empty_visit() {
        assert!(carries_invite(
            "https://tonk.space/join?access=chain&remote=https%3A%2F%2Fs#seed"
        ));
        // A malformed chain is still an ATTEMPT: it must reach the claim
        // path and fail with its reason, not be mistaken for an empty visit.
        assert!(carries_invite("https://tonk.space/join?access=not-a-chain"));

        assert!(!carries_invite("https://tonk.space/join"));
        assert!(!carries_invite("https://tonk.space/join#seed"));
        assert!(
            !carries_invite("https://tonk.space/join?access="),
            "an empty access parameter carries no chain"
        );
        assert!(!carries_invite(
            "https://tonk.space/join?remote=https%3A%2F%2Fs"
        ));
        assert!(!carries_invite("not a url"));
    }
}

/// The join's overlay clear must be SCOPED. The branch overlay is
/// shared: the tab's `tonk:site` facts (path, route, concept) live
/// there too, and every view on the page resolves through them. A
/// blanket clear wiped the site out from under the page on each
/// `/join` mount, so the site display lost its entity and fell back to
/// its spinner forever. Pinned here because the failure is silent —
/// nothing errors, the page just stops rendering.
#[cfg(test)]
mod overlay_scope_tests {
    use super::{JOIN_STATUS_URI, retains_overlay_entity};
    use dialog_artifacts::Entity;

    #[test]
    fn it_clears_only_the_joins_own_overlay_entity() {
        let status: Entity = JOIN_STATUS_URI.parse().expect("status URI parses");

        assert!(
            !retains_overlay_entity(&status, &status),
            "the join's own status is what the clear is for"
        );

        for foreign in ["tonk:site", "tonk:join/route", "tonk:replica"] {
            assert!(
                retains_overlay_entity(&foreign.parse::<Entity>().expect("URI parses"), &status),
                "{foreign} belongs to the page, not to this join"
            );
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) mod tests {
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
        content_memberships, put_repo, test_state, test_state_without_root,
    };

    /// Hand-craft an audience-open invite URL for a synthetic
    /// repository subject. Distinct tag bytes give distinct
    /// subjects/ephemerals. Returns the URL plus the subject's routing
    /// key (the repo the join mounts the claimer's replica under).
    pub(crate) async fn handcrafted_invite_url(
        subject_tag: u8,
        ephemeral_tag: u8,
    ) -> (String, String) {
        crate::router::tests::open_invite_url(subject_tag, ephemeral_tag, None).await
    }

    /// The same open invite, but advertising an access service. The host
    /// does not exist, so any staged pull against it fails the way a
    /// remote outage does.
    async fn unreachable_invite_url(subject_tag: u8, ephemeral_tag: u8) -> (String, String) {
        crate::router::tests::open_invite_url(
            subject_tag,
            ephemeral_tag,
            Some("https://sync.invalid.test/ucan/"),
        )
        .await
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
            .issuer(dialog_credentials::Signer::from(subject_signer))
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
        (invite.to_url("https://tonk.network/join").unwrap(), key)
    }

    /// Everything a failed join must leave untouched, in one value.
    ///
    /// Covers the profile index and its seeding status, the shared
    /// roster on the subject's content branch, and whether the accepted
    /// authority proves — i.e. every surface the recipient can observe
    /// after an attempt.
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
        /// Whether durable proof storage authorizes the worker for the subject.
        authority: bool,
    }

    async fn snapshot(state: &crate::router::AppState, key: &str) -> JoinSnapshot {
        use dialog_query::{Output as _, Query, Term};

        // The routing key *is* the subject DID; there is no suffix to strip.
        let subject: dialog_varsig::Did = key.parse().expect("subject parses");

        let (replicas, statuses, authority) = {
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
            let authority = tonk
                .profile
                .access()
                .prove(
                    dialog_capability::Subject::from(subject.clone())
                        .attenuate(dialog_effects::Use),
                )
                .audience(&tonk.operator)
                .perform(&tonk.operator)
                .await
                .is_ok();
            (replicas, statuses, authority)
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
            authority,
        }
    }

    pub(crate) async fn post_join(app: &axum::Router, url: &str) -> StatusCode {
        post_invite(app, "/api/profile/join", url).await
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

    #[dialog_common::test]
    async fn durable_join_persists_and_builds_a_named_root_ending_backup() {
        use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
        use tonk_schema::RepositoryName;
        use tonk_schema::prelude::DidExt as _;

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(105, 106).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let remote = crate::router::repository::RepositoryConfiguration::default()
            .remote(
                "origin",
                crate::router::repository::RemoteConfiguration::new(
                    dialog_repository::SiteAddress::from(dialog_remote_ucan_s3::UcanAddress::new(
                        "https://sync.example.test/ucan/",
                    )),
                )
                .subject(subject.clone())
                .revocation_url("https://relay.example.test/revocations/".parse().unwrap()),
            )
            .branch(
                "main",
                crate::router::repository::BranchConfiguration::default()
                    .upstream("origin", "main"),
            );
        let attached = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&remote).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(attached.status(), StatusCode::OK);
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(RepositoryName {
                    this: subject.this(),
                    name: tonk_schema::domain::repo::Name("joined-garden".to_string()),
                })
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }
        let (root, persisted) = {
            let tonk = state.read().await;
            let root = crate::router::identity::root_did(&tonk).await.unwrap();
            let bytes = tonk
                .profile
                .credential()
                .site(format!("{SPACE_ROOT_SITE_PREFIX}{subject}"))
                .load::<Vec<u8>>()
                .perform(&tonk.operator)
                .await
                .unwrap();
            (root, bytes)
        };
        let chain = dialog_ucan_core::DelegationChain::try_from(persisted.as_slice())
            .expect("the persisted space-root prefix parses");
        assert_eq!(chain.audience(), &root, "the prefix ends at the root");
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
                    .extension(crate::axum::RequestOrigin::parse("https://tonk.network/").unwrap())
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
            post_join(&app, "https://tonk.network/join?access=not-base58").await,
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
        use dialog_capability::access::AuthorizeError;
        use dialog_effects::memory::MemoryError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let refusal = PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(
            ResolveError::from(MemoryError::Authorization(AuthorizeError::Revoked {
                subject: dialog_capability::did!("key:zSubject"),
            })),
        ));

        assert_eq!(
            super::classify_pull(&refusal).kind(),
            super::JoinFailureKind::Revoked,
        );
    }

    #[dialog_common::test]
    fn it_does_not_misreport_an_unrelated_forbidden_response_as_revocation() {
        use dialog_capability::access::AuthorizeError;
        use dialog_effects::memory::MemoryError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let refusal =
            PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(ResolveError::from(
                MemoryError::Authorization(AuthorizeError::CommandEscalation {
                    claimed: "/storage/put".to_string(),
                    authorized: "/storage/get".to_string(),
                }),
            )));
        assert_eq!(
            super::classify_pull(&refusal).kind(),
            super::JoinFailureKind::ClaimFailed,
        );
    }

    /// A service that is merely unwell is retryable; only a refusal is
    /// terminal.
    #[dialog_common::test]
    fn it_reads_an_unwell_service_as_retryable() {
        use dialog_effects::Rejection;
        use dialog_effects::memory::MemoryError;
        use dialog_repository::{FetchRemoteBranchError, PullError, ResolveError};

        let outage = PullError::FetchRemoteBranch(FetchRemoteBranchError::Resolve(
            ResolveError::from(MemoryError::Rejected(Rejection::Unavailable {
                reason: "unavailable".to_string(),
            })),
        ));

        assert_eq!(
            super::classify_pull(&outage).kind(),
            super::JoinFailureKind::Unavailable,
        );
    }

    // ----- success and idempotence -----

    #[dialog_common::test]
    async fn it_joins_a_targeted_invite_issued_to_this_root() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let root_did = {
            let tonk = state.read().await;
            crate::router::identity::root_did(&tonk).await.unwrap()
        };
        let (url, key) = targeted_invite_url(74, &root_did).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

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

    /// Re-opening an invite link for a space this profile already holds
    /// must not re-route the authority its sync presigns with: the
    /// renewal saves the same account-rooted chain again, and the
    /// certificate walk (which never consults the clock, see
    /// [`crate::session`]) keeps one live route rather than choosing
    /// between two.
    #[dialog_common::test]
    async fn it_keeps_a_durable_members_authority_when_the_invite_is_reopened() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(90, 91).await;
        let subject: dialog_varsig::Did = key.parse().unwrap();

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);
        let durable = proof_window(&state, &subject).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::OK);

        assert_eq!(
            proof_window(&state, &subject).await,
            durable,
            "re-opening the invite bounded the durable member's own authority",
        );
        assert_eq!(
            snapshot(&state, &key).await.members.len(),
            1,
            "re-opening the invite keeps one membership row",
        );
    }

    /// When the presign path's proof for `subject` lapses, if ever.
    async fn proof_window(
        state: &crate::router::AppState,
        subject: &dialog_varsig::Did,
    ) -> Option<u64> {
        let tonk = state.read().await;
        tonk.profile
            .access()
            .prove(dialog_capability::Subject::from(subject.clone()).attenuate(dialog_effects::Use))
            .audience(&tonk.operator)
            .perform(&tonk.operator)
            .await
            .expect("the durable member stays authorized")
            .duration
            .expiration
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

    /// A device holding only its onboarding account joins the same way a
    /// registered one does; the membership just terminates at that
    /// account until accreditation re-roots it.
    #[dialog_common::test]
    async fn it_joins_under_the_onboarding_account_before_accreditation() {
        let (app, state, _lsp) = api_router_with_state(test_state_without_root().await);
        let (url, key) = handcrafted_invite_url(84, 85).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        // The membership terminates at the onboarding account, the same
        // shape a passkey root gives, so accreditation re-roots it from
        // the custodied invite seed rather than redoing the join.
        let onboarding = crate::onboarding::did(&*state.read().await)
            .await
            .expect("the onboarding account reads")
            .expect("the join minted the onboarding account")
            .this();
        let after = snapshot(&state, &key).await;
        assert!(after.authority, "the accepted authority proves");
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let session_expires_at = state.read().await.session_expires_at;
        assert_eq!(
            proof_window(&state, &subject).await,
            Some(session_expires_at),
            "only the renewable browser session bounds a pre-account join; \
             there is no one-hour guest hop",
        );
        let memberships = content_memberships(&state, &key).await;
        assert!(
            memberships.iter().any(|row| row.member.0 == onboarding),
            "the membership is keyed on the onboarding account",
        );

        // The invite principal's seed is custodied on profile main, sealed
        // to the onboarding account, which is what accreditation opens to
        // re-root the membership.
        use dialog_query::{Output as _, Query, Term};
        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        // The principal's entity IS the subject, so the invite principal
        // is read from `this` rather than from a repeated field.
        let principals: Vec<tonk_schema::SecretPrincipal> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretPrincipal> {
                this: Term::var("this"),
                kind: Term::from(tonk_schema::SeedKind::Invite.kind()),
                seed: Term::var("seed"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(principals.len(), 1, "one sealed invite principal");
        let principal: dialog_varsig::Did = principals[0].this.to_string().parse().unwrap();

        let rows: Vec<tonk_schema::SecretMessage> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretMessage> {
                this: Term::from(principals[0].seed.0.clone()),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "the principal names a real message");
        let sealed = tonk_identity::sealed::Sealed::decode(&rows[0].message.0).unwrap();
        let opened = crate::onboarding::account(&tonk)
            .await
            .unwrap()
            .secret()
            .reveal(&sealed, &principal)
            .expect("the onboarding account opens its custodied seed");
        let reissued = dialog_credentials::Ed25519Signer::import(&*opened)
            .await
            .unwrap();
        assert_eq!(
            reissued.did(),
            principal,
            "the seed derives the invite principal the membership hangs off",
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

    /// Classifying a remote's refusal as a revocation is a read: it reports
    /// what sync should say and erases nothing locally.
    ///
    /// Scope, because the old name for this test (`it_keeps_joined_data_
    /// readable_after_the_invite_is_revoked`) claimed far more than the body
    /// checks. The invite here is local-only, nothing is actually revoked,
    /// and the two snapshots bracket a constant comparison — so what is
    /// pinned is that a durable join lands and that classification has no
    /// side effects. It is not evidence that a revoked member can still read
    /// a space.
    ///
    /// That stronger property no longer holds unconditionally anyway. A
    /// remote-backed join now installs only the nodes its claim created and
    /// reads the rest through the remote (see [`install_claim_nodes`]), so a
    /// revoked replica retains what it happened to read, not a whole copy.
    /// Access control is unchanged — the access service still refuses a
    /// revoked credential — but local durability after revocation is now a
    /// consequence of what was read, not a guarantee.
    ///
    /// Proving the stronger property would need a working remote, which no
    /// fixture here has: every invite is either remote-less or points at an
    /// unreachable host.
    ///
    /// [`install_claim_nodes`]: super::install_claim_nodes
    #[dialog_common::test]
    async fn it_leaves_local_state_untouched_when_a_refusal_is_classified() {
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
            "classifying a refusal wrote nothing",
        );
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

        // The shape `core.yaml` seeds for a lean space: a pinned concept for
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
      description: The space this canvas belongs to

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

    /// What installing one claim costs on a space of `filler` facts.
    struct ClaimInstallCost {
        /// Nodes copied when the diff keeps the base the remote served —
        /// what a join actually pays.
        based: usize,
        /// Nodes copied by the same install once that base is thrown
        /// away: the space itself.
        baseless: usize,
    }

    /// Build a branch holding `filler` facts, snapshot it, commit one more
    /// fact on top, and report what
    /// [`install_claim_nodes`](super::install_claim_nodes) copies to carry
    /// that last commit — against the snapshot, and against a head from
    /// before the filler was written.
    ///
    /// The snapshot stands in for the head a remote served, and the commit
    /// on top for the roster facts a claim stages — the two revisions the
    /// real install diffs. The pre-filler head stands in for the base the
    /// regression loses, and prices the same install as a full copy of the
    /// space.
    async fn claim_install_cost(filler: usize) -> ClaimInstallCost {
        use dialog_repository::{Branch, RepositoryExt as _};

        let tonk = test_state().await;

        // Tree shape is a pure function of its keys, and history keys carry
        // the repository issuer. `put_repo` deliberately generates a fresh
        // signer, which made this cost fixture sample a different tree on
        // every run and occasionally turn one insert into a near-total
        // rechunk. Pin the issuer so this test measures the install
        // algorithm rather than key-distribution luck.
        let signer = Ed25519Signer::import(&[65u8; 32])
            .await
            .expect("the fixture signer imports");
        let repo = signer.did().repo_key().to_owned();
        let repository = tonk
            .profile
            .repository(repo)
            .create()
            .with_credential(signer)
            .perform(&tonk.operator)
            .await
            .expect("the fixture repository creates");
        let content: Branch = repository
            .branch(DEFAULT_BRANCH)
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");

        // A head that predates the filler, so a diff against it has to
        // carry the whole space. Committed rather than read off the fresh
        // branch, which need not have a revision until something is
        // written to it.
        content
            .transaction()
            .assert(tonk_schema::RepositoryName {
                this: "id:filler/origin".parse().expect("entity parses"),
                name: tonk_schema::domain::repo::Name("the origin".to_string()),
            })
            .commit()
            .perform(&tonk.operator)
            .await
            .expect("the origin commits");
        let origin = content.revision().expect("the origin produced a head");

        // Enough distinct entities to give the tree real depth. One
        // transaction: the cost under test is the diff between two
        // revisions, not how many commits produced them.
        let mut bulk = content.transaction();
        for index in 0..filler {
            bulk = bulk.assert(tonk_schema::RepositoryName {
                this: format!("id:filler/{index}").parse().expect("entity parses"),
                name: tonk_schema::domain::repo::Name(format!("filler {index}")),
            });
        }
        bulk.commit()
            .perform(&tonk.operator)
            .await
            .expect("the filler commits");
        let base = content.revision().expect("the filler produced a head");

        content
            .transaction()
            .assert(tonk_schema::RepositoryName {
                this: "id:filler/claim".parse().expect("entity parses"),
                name: tonk_schema::domain::repo::Name("the claim".to_string()),
            })
            .commit()
            .perform(&tonk.operator)
            .await
            .expect("the claim commits");
        let target = content.revision().expect("the claim produced a head");

        ClaimInstallCost {
            based: super::install_claim_nodes(&tonk, &content, &tonk.operator, &target, &base)
                .await
                .expect("the claim installs"),
            baseless: super::install_claim_nodes(&tonk, &content, &tonk.operator, &target, &origin)
                .await
                .expect("the space copies"),
        }
    }

    /// A claim install carries what the claim wrote, not what the space
    /// holds — so it stays a fraction of what copying the space costs.
    ///
    /// This is the regression that shipped: dialog's `Branch::install`
    /// diffs against `Index::empty()`, which makes every node in the tree
    /// novel and turns a join into a full replication. On a modest space
    /// that was ~500 sequential authorized round trips and ~110 seconds
    /// before the recipient saw anything, against ~40 and ~9s once the diff
    /// had a real base.
    ///
    /// Both counts are taken on the same tree, because losing the base is
    /// precisely what makes them converge. Pricing the big space against a
    /// small one instead — the shape this test had first — put a one-node
    /// tree in the denominator, and left the bound unable to say how bad a
    /// violation was: a CI run reported 36 nodes against a bound of 3, and
    /// only measuring the space itself showed that 36 was most of a full
    /// copy rather than a handful of legitimately rewritten ancestors.
    /// A correct install copies one node against the low forties for the
    /// space, so the eighth asserted here carries the tree changing shape
    /// without letting a near-total copy through.
    ///
    /// The fixture is sized to discriminate, not to impress: in the browser
    /// harness this test runs against Chrome's 30-second renderer-liveness
    /// check, and a bigger space proves nothing more while drifting toward
    /// that cliff on a loaded runner.
    #[dialog_common::test]
    async fn it_installs_a_claim_without_copying_the_space() {
        let cost = claim_install_cost(3000).await;

        assert!(
            cost.based > 0,
            "a claim that wrote a fact must carry at least one node",
        );
        assert!(
            cost.based * 8 <= cost.baseless,
            "installing a claim onto a 3000-fact space copied {} nodes, \
             against {} for the space itself — the copy is scaling with the \
             space, so the diff has lost its base",
            cost.based,
            cost.baseless,
        );
    }

    /// The limit case of the rule above: when a claim writes nothing, the
    /// join copies nothing, and the replica is composed entirely of content
    /// read back through the remote.
    #[dialog_common::test]
    async fn it_copies_nothing_when_a_claim_wrote_nothing() {
        use dialog_repository::{Branch, Repository, RepositoryExt as _};

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let repo = put_repo(&app, "guest-visit").await;
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(&repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch(DEFAULT_BRANCH)
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
            .expect("the content commits");
        let head = content.revision().expect("the content produced a head");

        let copied = super::install_claim_nodes(&tonk, &content, &tonk.operator, &head, &head)
            .await
            .expect("an empty claim installs");

        assert_eq!(
            copied, 0,
            "a visit that adds no facts must carry no nodes of its own",
        );
    }
}
