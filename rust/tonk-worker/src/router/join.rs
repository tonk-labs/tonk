//! Redeeming an invite URL, atomically.
//!
//! A join is externally a single transition: either the recipient ends
//! up holding a usable replica of the invited subject, or nothing they
//! can see about their profile changed.
//!
//! ```text
//! parse -> verify audience -> build candidate chain
//!       -> mount replica (hidden) -> save candidate authority
//!       -> pull -> validate -> commit claim
//!       -> index replica (visible) -> backup -> navigate
//! ```
//!
//! [`prepare_join`] does the reads: parse the URL, check the invite is
//! addressed to this identity, and build the candidate delegation chain
//! in memory. [`perform_join`] is then plain dialog against the durable
//! replica: mount it hidden, save the candidate chain (the pull's forks
//! prove with it, and the remote honouring or refusing them is the
//! authorization verdict), pull, validate what the remote served, and
//! commit the roster claim — then save the accepted authority and only
//! then index the replica in the profile.
//!
//! Atomicity needs no side store: a mounted-but-unindexed repository is
//! invisible and resumable, content-addressed blocks are unobservable
//! until a head references them, and a candidate chain the remote
//! refused is inert authority that delegation pruning reclaims.
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
//! no type in this module renders one: [`PreparedJoin`] and
//! [`JoinFailure`] both redact, and failure copy is fixed text chosen
//! from a closed set rather than anything an upstream response said.

use ::axum::{Json, extract::State, http::StatusCode};
use dialog_artifacts::{ArtifactSelector, Changes, Entity, Relation, Statement as _, Value};
use dialog_capability::access::{AuthorizeError, Prove, Retain};
use dialog_capability::{Fork, Provider};
use dialog_credentials::Credential;
use dialog_effects::archive::Get;
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan::UcanAddress;
use dialog_repository::{
    Branch, PullError, RemoteSite, Repository, RepositoryExt as _, ResolveEnv, SiteAddress,
};
use dialog_ucan::{Ucan, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
use tonk_common::log;
use tonk_invite::{Invite, InviteAudience, Ticket};
use tonk_macros::handler;
use tonk_schema::{
    Invitation, InvitationExecution, InvitedVia, MemberName, MemberRole, Membership, Replica,
    RepositoryName, SeedKind, prelude::DidExt as _,
};
use tonk_worker_api::JoinFailureKind;
use zeroize::Zeroizing;

use super::AppState;
use super::repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, build_repository_info, record_initialized_replica_in_profile,
    record_replica_local_meta,
};
use crate::{TonkWorkerError, worker::TonkState};

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
/// The join's helpers (validation, roster reads, the pull wrapper) are
/// generic over this bundle rather than over `TonkState`, so they can be
/// exercised against any operator that provides it.
pub(crate) trait BranchEnv:
    ResolveEnv + Provider<Prove<Ucan>> + Provider<Retain<Ucan>> + Provider<Fork<RemoteSite, Get>>
{
}

impl<T> BranchEnv for T where
    T: ResolveEnv
        + Provider<Prove<Ucan>>
        + Provider<Retain<Ucan>>
        + Provider<Fork<RemoteSite, Get>>
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

    fn private(detail: impl Into<String>) -> Self {
        Self::new(JoinFailureKind::Private, detail)
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
            JoinFailureKind::Private => TonkWorkerError::Forbidden(message),
        }
    }
}

/// The terminal classifications, their fixed copy, and how each one
/// reaches the caller. Pure data, so this runs on both targets.
#[cfg(test)]
mod failure_vocabulary {

    use super::{AuthorizeError, JoinFailure, JoinFailureKind};
    use crate::TonkWorkerError;

    const KINDS: [JoinFailureKind; 7] = [
        JoinFailureKind::Malformed,
        JoinFailureKind::AudienceMismatch,
        JoinFailureKind::Revoked,
        JoinFailureKind::Unavailable,
        JoinFailureKind::Refused,
        JoinFailureKind::ClaimFailed,
        JoinFailureKind::Private,
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
                "This space is private. Ask someone in it for an invite link.",
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
                "private",
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

/// Redeem an invite URL to this device's account.
#[handler]
pub async fn join(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let outcome = join_invite(&state, &body.url).await?;
    log!(
        "POST /api/profile/join -> subject {} (key {})",
        outcome.subject,
        outcome.key
    );
    let tonk = state.read().await;
    joined_response(&tonk, outcome).await
}

/// Load the committed replica and shape the route's success body.
async fn joined_response(
    tonk: &TonkState,
    outcome: JoinOutcome,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let repository = super::repository::space_named(tonk, outcome.key.as_str())
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
/// A failure at any stage leaves the recipient's profile, repository
/// list, roster, and claim backup exactly as they can see them: what a
/// failed [`perform_join`] leaves behind — a hidden unindexed replica,
/// unreferenced content-addressed blocks, an unused candidate chain —
/// is invisible, resumable, and reclaimed by pruning.
pub(crate) async fn join_invite(state: &AppState, url: &str) -> Result<JoinOutcome, JoinFailure> {
    // Per-phase wall clock, logged on success: the perform phase is the
    // network-bound one (pull + validation + roster reads against the
    // remote), so a slow join in the field can be attributed to the
    // network or to local work without reproducing it.
    // A space's bare address names no invite: it opens the space for
    // whoever can read it.
    if let Some(public) = Ticket::public_for_url(url)
        .map_err(|error| JoinFailure::malformed(format!("space address did not parse: {error}")))?
    {
        let tonk = state.read().await;
        return open_space(&tonk, public).await;
    }
    let started = web_time::Instant::now();
    let prepared = {
        let tonk = state.read().await;
        prepare_join(&tonk, url).await?
    };
    let prepared_at = web_time::Instant::now();
    let outcome = perform(state, prepared).await?;
    log!(
        "join: prepared {}ms, performed {}ms",
        prepared_at.duration_since(started).as_millis(),
        prepared_at.elapsed().as_millis()
    );
    Ok(outcome)
}

/// Open a space this device holds no replica of from its bare address,
/// `/space/{did}`: with access the account already holds, or else with
/// the public ticket the space keeps if it is published.
///
/// The public ticket makes its redeemer a reader, not a member. It
/// grants only `/use/get`, so nothing this does writes to the space: no
/// roster claim, no retained hop, no custodied seed. The ticket is saved
/// in the profile, where it proves through the account's powerline from
/// the public principal, so the next open finds it here rather than
/// claiming again.
///
/// [`JoinFailure::private`] when the space keeps no public ticket and
/// the account holds no access to it.
async fn open_space(tonk: &TonkState, ticket: Ticket) -> Result<JoinOutcome, JoinFailure> {
    let subject = ticket.subject().clone();
    let key = subject.repo_key().to_owned();
    let existing = find_replica_for_subject(tonk, &subject)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to look up the local replica: {error}"))
        })?;
    if existing {
        return Ok(JoinOutcome {
            key,
            subject,
            renewed: true,
        });
    }

    let (account, device_grant) = crate::router::account::current_account(tonk)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to resolve the account: {error}"))
        })?;
    super::public_access::ensure_powerline(tonk, &account)
        .await
        .map_err(|error| JoinFailure::claim_failed(format!("public powerline: {error}")))?;
    super::public_access::save(tonk, device_grant)
        .await
        .map_err(|error| JoinFailure::claim_failed(format!("device grant: {error}")))?;

    if !super::public_access::can_read(tonk, &subject).await {
        let chain = claim_public_ticket(&ticket).await?;
        super::public_access::save(tonk, chain)
            .await
            .map_err(|error| JoinFailure::claim_failed(format!("public ticket: {error}")))?;
    }

    let remote_url = ticket.remote().to_string();
    let repository = mount_replica(tonk, &subject, Some(&remote_url), None)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to prepare the local replica: {error}"))
        })?;
    let branch = repository
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to open the content branch: {error}"))
        })?;
    pull_upstream(&branch, &tonk.operator).await?;
    validate_content(&branch, &tonk.operator, &subject).await?;

    tonk.reactor
        .refresh_branch(subject.repo_key(), DEFAULT_BRANCH, &tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to adopt the opened branch: {error}"))
        })?;
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    record_initialized_replica_in_profile(tonk, &subject)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to index the replica: {error}"))
        })?;
    log!("join: opened subject {subject} (key {key}) to read");
    Ok(JoinOutcome {
        key,
        subject,
        renewed: false,
    })
}

/// Claim the ticket a space keeps for the public principal, signed as
/// that principal, or [`JoinFailure::private`] when it keeps none.
async fn claim_public_ticket(ticket: &Ticket) -> Result<DelegationChain, JoinFailure> {
    let holder = ticket
        .holder()
        .await
        .map_err(|error| JoinFailure::claim_failed(format!("public key: {error}")))?;
    let holder_did = dialog_varsig::Principal::did(&holder);
    let fetched =
        tonk_account::ticket::claim(ticket.remote(), holder, &holder_did, None, ticket.subject())
            .await
            .map_err(|error| match error {
                tonk_account::ticket::ClaimError::Refused(reason) => {
                    classify_authorization(&reason)
                }
                _ => JoinFailure::unavailable("the space's public ticket could not be fetched"),
            })?
            .ok_or_else(|| JoinFailure::private("the space keeps no public ticket"))?;
    ticket
        .accept(&fetched)
        .await
        .map_err(|error| JoinFailure::malformed(format!("public ticket: {error}")))
}

/// Run the ordinary join pipeline for a browser-approved local-space link.
/// The wrapper keeps the join's private diagnostic type inside this module.
pub(crate) async fn join_for_local_space_link(
    state: &AppState,
    url: &str,
    approved_subject: &Did,
) -> Result<JoinOutcome, TonkWorkerError> {
    let invite = parse_invite(url).await?;
    // Check consent before preparation can mint an account, and before the
    // join can save authority, membership, or a replica. Use this same parsed
    // invitation throughout so validation and mutation cannot name two spaces.
    if invite.subject() != approved_subject {
        return Err(TonkWorkerError::Forbidden(
            "local-space link invite names another space".into(),
        ));
    }
    let prepared = {
        let tonk = state.read().await;
        prepare_parsed_join(&tonk, invite).await?
    };
    perform(state, prepared).await.map_err(Into::into)
}

/// Perform a prepared join with whichever worker holds the space: this one,
/// on a host with one database ([`perform_join`]), or the space's own
/// ([`perform_join_elsewhere`]).
///
/// Takes the state and not a guard over it: the space's own worker asks this
/// one for its delegation while it is being asked to commit the claim, so
/// nothing may be held across asking it.
async fn perform(state: &AppState, prepared: PreparedJoin) -> Result<JoinOutcome, JoinFailure> {
    {
        // Alone with the state while it commits here: what else writes the
        // space's branch (a sync, a command still running) would move the
        // head under the claim.
        let tonk = state.write().await;
        if !tonk.spaces_elsewhere() {
            return perform_join(&tonk, prepared).await;
        }
    }
    perform_join_elsewhere(state, prepared).await
}

/// Replace a browser-approved local-space join's invite authority with its
/// direct space-to-account prefix. The ordinary join needs the narrower invite
/// while pulling; the direct prefix is the durable ownership authority.
pub(crate) async fn save_local_space_root_authority(
    tonk: &TonkState,
    subject: &Did,
    chain: DelegationChain,
) -> Result<(), TonkWorkerError> {
    save_authority(tonk, subject, chain)
        .await
        .map_err(Into::into)
}

/// Parse the invite, verify it is addressed to this identity, and build
/// the candidate chain. Reads only, except that a device joining before
/// it has any account mints its onboarding account here.
async fn prepare_join(tonk: &TonkState, url: &str) -> Result<PreparedJoin, JoinFailure> {
    prepare_parsed_join(tonk, parse_invite(url).await?).await
}

async fn parse_invite(url: &str) -> Result<Invite, JoinFailure> {
    if let Some(ticket) = Ticket::parse_url(url)
        .map_err(|error| JoinFailure::malformed(format!("invite did not parse: {error}")))?
    {
        return claim_ticket(ticket).await;
    }
    Invite::parse_url(url)
        .await
        .map_err(|error| JoinFailure::malformed(format!("invite did not parse: {error}")))
}

/// Fetch the ticket a ticket link's space keeps for the link's key, and
/// read it as the open invite it is. From here a ticket link joins the
/// way an `access=` link does.
async fn claim_ticket(ticket: Ticket) -> Result<Invite, JoinFailure> {
    let holder = ticket
        .holder()
        .await
        .map_err(|error| JoinFailure::malformed(format!("ticket key: {error}")))?;
    let holder_did = dialog_varsig::Principal::did(&holder);
    let fetched =
        tonk_account::ticket::claim(ticket.remote(), holder, &holder_did, None, ticket.subject())
            .await
            .map_err(|error| match error {
                tonk_account::ticket::ClaimError::Refused(reason) => {
                    classify_authorization(&reason)
                }
                _ => JoinFailure::unavailable("the space's ticket could not be fetched"),
            })?;
    // The space no longer keeps a ticket for this link's key: it was
    // taken back, which withdraws the link as far as a newcomer goes.
    let Some(fetched) = fetched else {
        return Err(JoinFailure::revoked(
            "the space no longer holds this link's ticket",
        ));
    };
    ticket
        .redeem(&fetched)
        .await
        .map_err(|error| JoinFailure::malformed(format!("ticket did not redeem: {error}")))
}

async fn prepare_parsed_join(
    tonk: &TonkState,
    invite: Invite,
) -> Result<PreparedJoin, JoinFailure> {
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

/// Perform a prepared join directly against the durable replica —
/// dialog's ordinary create-and-replicate flow, with the invite's chain
/// as the authority that makes the pull answerable.
///
/// There is no staging: the replica is mounted (hidden — no profile
/// `Replica` fact yet, so nothing is navigable), the candidate chain is
/// saved into the durable certificate store so the pull's forks can
/// prove with it, and then the join is plain dialog: pull (the remote
/// honouring the chain IS the authorization check), validate what it
/// served, commit the roster claim. Content-addressed blocks a failed
/// attempt hydrated are unobservable until a head references them, an
/// unindexed repository is invisible and resumable, and a candidate
/// chain the remote refused is inert authority that delegation pruning
/// reclaims — so a failure needs no rollback and leaves nothing a user
/// can see.
///
/// A renewal is the same flow on the already-mounted replica: the pull
/// is dialog's normal merge of the remote into the local head, which is
/// exactly what every later sync tick does.
async fn perform_join(
    tonk: &TonkState,
    prepared: PreparedJoin,
) -> Result<JoinOutcome, JoinFailure> {
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
            .space(prepared.key.as_str())
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to load the renewing replica: {error}"))
            })?
    };

    // The candidate authority, durable before the pull because the pull
    // is what exercises it: the proof walk composes the chain onto the
    // account's device grant out of the durable certificate store, and
    // the remote accepting or refusing the forks it signs is the
    // authorization verdict. Saving first is safe — idempotent at the
    // dialog layer, and a chain a failed attempt leaves behind is unused
    // authority that delegation pruning reclaims.
    for grant in [&prepared.device_grant, &prepared.chain] {
        tonk.profile
            .access()
            .save(UcanDelegation(grant.clone()))
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!(
                    "failed to save the candidate authority: {error}"
                ))
            })?;
    }

    let name = crate::router::profile_name::resolve_display_name(tonk).await;
    commit_claim(
        tonk,
        &repository,
        Claim {
            invitation: &prepared.invitation,
            execution: &prepared.invitation_execution,
            member: &prepared.member,
            subject: &prepared.subject,
            name: &name,
            authorize: prepared.needs_remote_authorization(),
        },
    )
    .await?;

    save_authority(tonk, &prepared.subject, prepared.chain.clone()).await?;
    retain_claim_authority(tonk, &prepared.key, &prepared.chain).await;

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
        // commit — the join is already complete.
        match invite_configuration(
            &prepared.subject,
            prepared.remote_url.as_deref(),
            prepared.revocation_url.as_deref(),
        ) {
            Ok(configuration) => {
                // A fresh install seeds the directory name from the
                // invite's advisory `name` so the Hub row is labeled
                // before content syncs; the mint-time name may be stale,
                // and the space's own record supersedes it once content
                // hydrates. A renewal seeds nothing — the directory
                // already carries a name at least as fresh as the link's.
                let seeded_name = if prepared.installs_replica() {
                    prepared.invite.space_name.as_deref()
                } else {
                    None
                };
                super::repository::record_space_mount(
                    tonk,
                    &prepared.subject,
                    &configuration,
                    seeded_name,
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

/// A claim on a space, as the worker that holds the space commits it.
struct Claim<'a> {
    /// The invitation being claimed.
    invitation: &'a Invitation,
    /// Audience metadata recorded beside the invitation.
    execution: &'a InvitationExecution,
    /// The account the claim is for.
    member: &'a Did,
    /// The space.
    subject: &'a Did,
    /// The name the member goes by, written when the roster has none.
    name: &'a str,
    /// Whether the remote has to honour the candidate chain first: the
    /// space is pulled, and what the pull brought is checked to be a space.
    authorize: bool,
}

/// Commit `claim` on the content branch of `repository`, which this worker
/// has mounted: pull and validate first when the claim has to be authorized,
/// then land its roster facts as an ordinary commit, which the next sync
/// pushes.
async fn commit_claim(
    tonk: &TonkState,
    repository: &Repository<Credential>,
    claim: Claim<'_>,
) -> Result<(), JoinFailure> {
    let branch = repository
        .branch(DEFAULT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to open the content branch: {error}"))
        })?;

    if claim.authorize {
        pull_upstream(&branch, &tonk.operator).await?;
        validate_content(&branch, &tonk.operator, claim.subject).await?;
    }

    // The claim below commits into the content branch directly, not
    // through the reactor: a branch an earlier release wrote (a renewal,
    // or a head the pull just adopted) is moved to this release's format
    // first, once per process, as every reactor commit is. A failure is
    // logged; the commit reports the refusal.
    if let Err(error) = tonk
        .reactor
        .repository(claim.subject.repo_key())
        .branch(DEFAULT_BRANCH)
        .migrate_once(&tonk.operator)
        .await
    {
        log!(
            "join: migrating '{}' failed: {error}",
            claim.subject.repo_key()
        );
    }

    // Every claim, including a renewal, lands its roster/provenance/name
    // facts as an ordinary commit; the next sync tick pushes them.
    let (changes, _already_claimed) = claim_changes(
        &branch,
        &tonk.operator,
        claim.invitation,
        claim.execution,
        claim.member,
        claim.subject,
        claim.name,
    )
    .await?;
    if !changes.is_empty() {
        branch
            .transaction()
            .assert(changes)
            .commit()
            .publish()
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to commit the claim: {error}"))
            })?;
    }

    // The reactor may hold a handle from an earlier attempt at this key;
    // the pull and commit moved the head underneath it. Leaving a stale
    // handle cached would wedge every later sync on this branch, so a
    // failure here fails the join rather than being logged past.
    tonk.reactor
        .refresh_branch(claim.subject.repo_key(), DEFAULT_BRANCH, &tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("failed to adopt the joined branch: {error}"))
        })?;
    // Deliver the fresh snapshot the refresh scheduled for any
    // subscriptions the rebind carried over — a live view left waiting
    // for the next commit waits forever on a branch nothing edits.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

/// A claim as the person's profile hands it to the worker that holds the
/// space: the invitation as it was issued, the chain that extends it to the
/// claiming account, and what the roster is to say of them. What a
/// [`ClaimMembership`] command carries.
///
/// [`ClaimMembership`]: tonk_schema::command::ClaimMembership
struct MembershipClaim {
    /// The invitation's chain as issued, before the claim extended it:
    /// base58. The invitation's record is derived from it.
    invitation: String,
    /// The chain extended to the claiming account: base58. Retained in the
    /// space, so the hop that admits this member is provable from it.
    chain: String,
    /// Whether the invitation is open to whoever redeems it.
    open: bool,
    /// The account the claim is for.
    member: Did,
    /// The name the member goes by.
    name: String,
    /// Whether the remote has to honour the chain before anything is
    /// written (see [`PreparedJoin::needs_remote_authorization`]).
    authorize: bool,
}

/// The outcome a claim that landed is reported with. A claim that did not
/// is reported with the kind of its failure.
const CLAIMED: &str = "claimed";

fn decode_chain(what: &str, encoded: &str) -> Result<DelegationChain, JoinFailure> {
    let bytes = bs58::decode(encoded)
        .into_vec()
        .map_err(|error| JoinFailure::malformed(format!("{what} is not base58: {error}")))?;
    DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| JoinFailure::malformed(format!("{what} did not decode: {error}")))
}

fn encode_chain(what: &str, chain: &DelegationChain) -> Result<String, JoinFailure> {
    chain
        .to_bytes()
        .map(|bytes| bs58::encode(bytes).into_string())
        .map_err(|error| JoinFailure::claim_failed(format!("{what} did not encode: {error}")))
}

/// Run the [`ClaimMembership`] command: commit, in the space this worker
/// holds, the claim the person's profile prepared, and say what came of it
/// where that profile is watching, in the session overlay at the command's
/// entity.
///
/// [`ClaimMembership`]: tonk_schema::command::ClaimMembership
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ClaimMembership>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ClaimMembership) {
        use tonk_schema::command::MembershipClaimed;
        use tonk_schema::domain::membership_claim::{Detail, Outcome};

        let origin = self.origin();
        let (repo, branch) = (origin.repo.clone(), origin.branch.clone());
        let tonk = self.state().read().await;
        let claimed = async {
            let member = command.member.0.to_string().parse().map_err(|error| {
                JoinFailure::malformed(format!("the claim names no account: {error:?}"))
            })?;
            take_claim(
                &tonk,
                &repo,
                MembershipClaim {
                    invitation: command.invitation.0.clone(),
                    chain: command.chain.0.clone(),
                    open: command.open.0,
                    member,
                    name: command.name.0.clone(),
                    authorize: command.authorize.0,
                },
            )
            .await
        };
        let (outcome, detail) = match claimed.await {
            Ok(()) => (CLAIMED.to_owned(), String::new()),
            Err(failure) => (failure.kind.as_str().to_owned(), failure.detail),
        };
        let session = match tonk
            .reactor
            .repository(&repo)
            .branch(&branch)
            .acquire(&tonk.operator)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                log!("ClaimMembership on '{repo}': nowhere to say '{outcome}': {error}");
                return;
            }
        };
        session.state.assert_overlay(MembershipClaimed {
            this: command.this,
            outcome: Outcome(outcome),
            detail: Detail(detail),
        });
        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&session.state));
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    }
}

/// Commit `claim` in the space `repo` names, which this worker holds, and
/// retain the chain that admits the member.
async fn take_claim(
    tonk: &TonkState,
    repo: &str,
    claim: MembershipClaim,
) -> Result<(), JoinFailure> {
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!("this worker does not hold '{repo}': {error}"))
        })?;
    let subject = repository.did();
    let issued = decode_chain("the invitation", &claim.invitation)?;
    let chain = decode_chain("the claimed chain", &claim.chain)?;
    if issued.subject() != Some(&subject) || chain.subject() != Some(&subject) {
        return Err(JoinFailure::malformed(format!(
            "the claim is not for {subject}"
        )));
    }
    let invitation = Invitation::from_chain(&issued)
        .ok_or_else(|| JoinFailure::malformed("the invitation names no space"))?;
    let execution =
        InvitationExecution::new(&invitation, if claim.open { "open" } else { "scoped" });
    commit_claim(
        tonk,
        &repository,
        Claim {
            invitation: &invitation,
            execution: &execution,
            member: &claim.member,
            subject: &subject,
            name: &claim.name,
            authorize: claim.authorize,
        },
    )
    .await?;
    retain_claim_authority(tonk, subject.repo_key(), &chain).await;
    Ok(())
}

/// [`perform_join`] where each space has a worker of its own, which holds
/// the space: this worker, the person's profile's, mounts nothing.
///
/// It keeps what is the profile's to keep: the candidate authority, saved
/// first because the space's worker is delegated to out of it, and, once the
/// claim has landed, the space's place among the person's spaces. The space
/// itself is the other worker's: that worker mounts it, pulls it (the remote
/// honouring the chain is the authorization check), validates what the pull
/// brought and commits the roster claim ([`take_claim`]).
///
/// A join that fails leaves nothing a person can see, as before: the space is
/// listed only after the claim lands, and a worker brought up for a space
/// that was not joined is told to forget it.
async fn perform_join_elsewhere(
    state: &AppState,
    prepared: PreparedJoin,
) -> Result<JoinOutcome, JoinFailure> {
    let configuration = invite_configuration(
        &prepared.subject,
        prepared.remote_url.as_deref(),
        prepared.revocation_url.as_deref(),
    )
    .map_err(|error| JoinFailure::malformed(format!("invite names no usable remote: {error}")))?;
    let claim = {
        let tonk = state.read().await;
        for grant in [&prepared.device_grant, &prepared.chain] {
            tonk.profile
                .access()
                .save(UcanDelegation(grant.clone()))
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!(
                        "failed to save the candidate authority: {error}"
                    ))
                })?;
        }
        // The space's worker asks where the space syncs before it can be
        // asked anything, and the directory does not list a space being
        // joined.
        if prepared.installs_replica() {
            super::space_directory::expect(&tonk, &prepared.subject, &configuration)
                .await
                .map_err(|error| {
                    JoinFailure::claim_failed(format!(
                        "failed to keep where the space syncs: {error}"
                    ))
                })?;
        }
        MembershipClaim {
            invitation: encode_chain("the invitation", &prepared.invite.chain)?,
            chain: encode_chain("the claimed chain", &prepared.chain)?,
            open: matches!(prepared.invite.audience, InviteAudience::Open { .. }),
            member: prepared.member.clone(),
            name: crate::router::profile_name::resolve_display_name(&tonk).await,
            authorize: prepared.needs_remote_authorization(),
        }
    };

    let claimed = hand_claim(&prepared.key, &claim).await;

    let tonk = state.read().await;
    if let Err(failure) = claimed {
        if prepared.installs_replica() {
            super::space_directory::settle(&tonk, &prepared.subject).await;
            drop(tonk);
            super::space_reach::forget(&prepared.key).await;
        }
        return Err(failure);
    }

    save_authority(&tonk, &prepared.subject, prepared.chain.clone()).await?;
    if prepared.installs_replica() {
        record_initialized_replica_in_profile(&tonk, &prepared.subject)
            .await
            .map_err(|error| {
                JoinFailure::claim_failed(format!("failed to index the replica: {error}"))
            })?;
    }
    // See [`perform_join`]: the directory is how another of this account's
    // devices recovers this claim, and here it is also where this worker
    // reads where the space syncs.
    let seeded_name = if prepared.installs_replica() {
        prepared.invite.space_name.as_deref()
    } else {
        None
    };
    super::repository::record_space_mount(&tonk, &prepared.subject, &configuration, seeded_name)
        .await;
    if prepared.installs_replica() {
        super::space_directory::settle(&tonk, &prepared.subject).await;
    }
    if let InviteAudience::Open { seed } = &prepared.invite.audience {
        super::account_state::custody_seed(
            &tonk,
            prepared.invite.chain.audience(),
            SeedKind::Invite,
            Zeroizing::new(*seed),
        )
        .await;
    }

    log!(
        "join: committed subject {} by its own worker (key {}, renewed {})",
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

/// How long the worker that holds a space is given to say what came of a
/// claim. It pulls the space first, and a space brought to this device for
/// the first time is fetched whole.
const CLAIM_ANSWER_WAIT: web_time::Duration = web_time::Duration::from_secs(180);

/// Hand `claim` to the worker that holds the space `key`, and read what came
/// of it.
///
/// The claim goes as a [`ClaimMembership`] command on the space's branch,
/// and its outcome comes back as a fact at the command's entity: this
/// subscribes to that fact before committing the command, and reads the
/// first frame that carries it.
///
/// [`ClaimMembership`]: tonk_schema::command::ClaimMembership
async fn hand_claim(key: &str, claim: &MembershipClaim) -> Result<(), JoinFailure> {
    use dialog_query::ConceptQuery;
    use futures_util::StreamExt as _;
    use futures_util::future::{Either, select};
    use ipld_core::ipld::Ipld;
    use tonk_schema::command::MembershipClaimed;
    use tonk_schema::query::Query as WireQuery;

    let unreached = |error: &dyn std::fmt::Display| {
        JoinFailure::unavailable(format!("the space's own worker was not reached: {error}"))
    };
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| JoinFailure::claim_failed(format!("no entropy for the claim: {error}")))?;
    let this = format!(
        "tonk:membership-claim/{}",
        bs58::encode(nonce).into_string()
    );
    let entity: Entity = this
        .parse()
        .map_err(|error| JoinFailure::claim_failed(format!("bad claim entity: {error}")))?;
    let command = super::space_reach::command(
        &[
            (
                "invitation",
                "xyz.tonk.command.claim-membership/invitation",
                "Text",
            ),
            ("chain", "xyz.tonk.command.claim-membership/chain", "Text"),
            ("open", "xyz.tonk.command.claim-membership/open", "Boolean"),
            (
                "member",
                "xyz.tonk.command.claim-membership/member",
                "Entity",
            ),
            ("name", "xyz.tonk.command.claim-membership/name", "Text"),
            (
                "authorize",
                "xyz.tonk.command.claim-membership/authorize",
                "Boolean",
            ),
        ],
        serde_json::json!({
            "this": this,
            "invitation": claim.invitation,
            "chain": claim.chain,
            "open": claim.open,
            "member": claim.member.to_string(),
            "name": claim.name,
            "authorize": claim.authorize,
        }),
    )
    .map_err(|error| JoinFailure::claim_failed(format!("the claim did not encode: {error}")))?;

    let peer = super::space_reach::peer(key);
    let outcome = ConceptQuery::from(Query::<MembershipClaimed> {
        this: Term::from(entity),
        outcome: Term::var("outcome"),
        detail: Term::var("detail"),
    });
    let mut frames = peer
        .content()
        .subscribe(WireQuery::from(&outcome))
        .perform(&peer)
        .await
        .map_err(|error| unreached(&error))?;
    super::space_reach::run(peer, peer.content(), command)
        .await
        .map_err(|error| unreached(&error))?;

    let answered = async {
        while let Some(frame) = frames.next().await {
            let rows = match frame.map_err(|error| unreached(&error))? {
                crate::reactor::Frame::Snapshot { conclusions } => conclusions,
                crate::reactor::Frame::Delta { asserted, .. } => asserted,
            };
            let Some(row) = rows.into_iter().next() else {
                continue;
            };
            let text = |field: &str| match row.fields.get(field) {
                Some(Ipld::String(text)) => text.clone(),
                _ => String::new(),
            };
            let (outcome, detail) = (text("outcome"), text("detail"));
            if outcome == CLAIMED {
                return Ok(());
            }
            let kind = serde_json::from_value(serde_json::Value::String(outcome.clone())).map_err(
                |_| {
                    JoinFailure::claim_failed(format!(
                        "the space's own worker said what is no outcome: {outcome}"
                    ))
                },
            )?;
            return Err(JoinFailure::new(kind, detail));
        }
        Err(JoinFailure::unavailable(
            "the space's own worker stopped answering before the claim settled",
        ))
    };
    let waited = crate::r#async::sleep(CLAIM_ANSWER_WAIT);
    futures_util::pin_mut!(answered, waited);
    match select(answered, waited).await {
        Either::Left((answer, _)) => answer,
        Either::Right(_) => Err(JoinFailure::unavailable(
            "the space's own worker did not say what came of the claim in time",
        )),
    }
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
    subject: &Did,
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
        .secrets()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{subject}"))
        .save(prefix_bytes)
        .perform(&tonk.profile)
        .await
        .map_err(|error| {
            JoinFailure::claim_failed(format!(
                "failed to persist the accepted root prefix: {error}"
            ))
        })?;
    Ok(())
}

/// Pull the branch from its upstream and classify what the remote said.
///
/// The typed service response survives the pull's error chain, so a
/// revoked credential is separated from an unreachable service without
/// reading either one's body.
async fn pull_upstream<Env: BranchEnv>(branch: &Branch, env: &Env) -> Result<(), JoinFailure> {
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
    let attribute: Relation = attribute
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
    branch: &Branch,
    env: &Env,
    invitation: &Invitation,
    invitation_execution: &InvitationExecution,
    member: &Did,
    subject: &Did,
    name: &str,
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
        MemberName::new(membership.this().clone(), name.to_owned()).assert(&mut changes);
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
        .space(key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(_) => {
            let space_credential = tonk_account::peer::mount_verifier(
                tonk.profile.storage(),
                tonk.registry.space_location(&key),
                subject,
            )
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("failed to create local replica '{key}': {e}"))
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
    let _admission_mutation = tonk.admission.mutation(subject.as_str());
    let key = subject.repo_key().to_owned();
    if super::account_state::is_account_key(tonk, &key).await {
        return Err(TonkWorkerError::Forbidden(
            "account system repository cannot be mounted as a user space".to_string(),
        ));
    }
    let repository = match tonk
        .profile
        .space(key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(_) => {
            let space_credential = tonk_account::peer::mount_verifier(
                tonk.profile.storage(),
                tonk.registry.space_location(&key),
                subject,
            )
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("failed to create local replica '{key}': {e}"))
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
        .branch(&tonk.active_branch)
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
const JOIN_STATUS_URI: &str = "tonk:join/status";

/// Run the [`Join`] command.
///
/// `<page-mount onmount=tonk/join>` on the `/join` view fires the command
/// with the full page URL in the event detail. This provider runs the same
/// join operation the HTTP routes do and drives the overlay-only
/// `tonk:join/status` (pending → failed, or retract + navigate on
/// success) on the profile meta branch — the branch the `/join` view
/// subscribes to.
///
/// [`Join`]: tonk_schema::command::Join
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Join> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::Join) {
        // A short link (`/@/{hash}#seed`) resolves to the long form
        // first. The deep-link flow never needed this — the BROWSER
        // follows the 301 (with fragment inheritance) before the /join
        // view mounts — but a PASTED short link reaches the worker
        // verbatim, and without resolution it carries no `access=` and
        // would silently read as a bare visit. Mirrors the CLI's claim.
        let mut command = command;
        if tonk_invite::shortcut::is_shortcut(&command.url.0) {
            match resolve_shortcut(&command.url.0).await {
                Ok(resolved) => command.url.0 = resolved,
                Err(error) => log!("join: short link did not resolve: {error}"),
            }
        }
        // There is no paste-link page: invite links open directly in the
        // browser. Return bare /join visits home before requesting custody.
        // A space's own page mounts the join too, where this device holds
        // no replica of it: a ticket link opens where it points, and the
        // bare address opens the space for whoever can read it.
        if !carries_invite(&command.url.0) && !is_bare_space_address(&command.url.0) {
            if !is_space_address(&command.url.0) {
                crate::router::navigate::notify_navigate(self.client(), "/");
            }
            return;
        }
        run_join(self, command).await;
    }
}

/// Native resolution: fetch without following, read `Location`, resolve
/// against the short link. Mirrors the CLI's `resolve_shortcut`.
/// A kernel on WASI has no HTTP client: the capability that would carry
/// this through the host is not designed yet (`plan/kernel-capabilities.md`,
/// `services`).
#[cfg(target_arch = "wasm32")]
async fn resolve_shortcut(short_url: &str) -> Result<String, String> {
    Err(format!(
        "this host gives the kernel no way to resolve {short_url}"
    ))
}

#[cfg(not(target_arch = "wasm32"))]
async fn resolve_shortcut(short_url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("short link client: {error}"))?;
    let response = client
        .head(short_url)
        .send()
        .await
        .map_err(|error| format!("short link fetch: {error}"))?;
    if !response.status().is_redirection() {
        return Err(format!(
            "the short link did not redirect (HTTP {})",
            response.status()
        ));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "the short link redirect carries no Location".to_owned())?;
    tonk_invite::shortcut::resolve_location(short_url, location).map_err(|error| error.to_string())
}

/// Whether a URL carries an invite at all.
///
/// On `/join` the delegation chain rides in `access`, so its presence is
/// what separates "redeem this" from a bare /join visit that returns
/// home. Deliberately a query test and not a parse: a malformed or
/// truncated invite IS an attempt and must still fail loudly with its
/// reason, rather than being silently treated as an empty visit.
///
/// A ticket link is a space's address with a seed for a fragment (see
/// [`Ticket`]); the same address without one is just the space.
fn carries_invite(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        parsed
            .query_pairs()
            .any(|(key, value)| key == "access" && !value.is_empty())
    }) || matches!(Ticket::parse_url(url), Ok(Some(_)))
}

/// Whether a URL is a space's own page, `/space/...`: where a join is
/// mounted to pick up a ticket link, and where finding none is no
/// reason to leave.
fn is_space_address(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| parsed.path().starts_with("/space/"))
}

/// Whether a URL is a space's address with no ticket beside it: a visit,
/// which opens the space if it is readable and otherwise leaves the page
/// to say it is private.
fn is_bare_space_address(url: &str) -> bool {
    matches!(Ticket::public_for_url(url), Ok(Some(_)))
}

/// Whether `url` is a space's own address, bare or as a ticket link: the
/// page a join from it runs on is already that space's page.
fn opens_in_place(url: &str) -> bool {
    matches!(Ticket::parse_url(url), Ok(Some(_))) || is_bare_space_address(url)
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
fn retains_overlay_entity(
    overlaid: &dialog_artifacts::Entity,
    status: &dialog_artifacts::Entity,
) -> bool {
    overlaid != status
}

/// Run the join operation from the command's full URL and drive the
/// overlay-only join status. Always leaves the overlay in a terminal state
/// (status retracted on success, `failed` on error).
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
        .branch(&tonk.active_branch)
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

    // Use the exact page URL carried by `<page-mount>`. In particular, do not
    // round-trip the query through `URLSearchParams`: targeted invites may
    // contain empty or repeated fields whose byte-for-byte form matters.
    let url = command.url.0;

    // The same operation the HTTP join runs, so the content behind the
    // redirect is proven before the redirect fires.
    // Nothing is held across the join: where the space's own worker commits
    // the claim, it asks this worker for its delegation meanwhile.
    drop(tonk);
    let joined = join_invite(env.state(), &url).await;
    let tonk = env.state().read().await;
    match joined {
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
            if !outcome.renewed {
                crate::router::navigate::notify_analytics(
                    env.client(),
                    tonk_worker_api::AnalyticsEvent::SpaceJoined {
                        space: outcome.key.clone(),
                    },
                );
            }
            // A space's own address (bare, or a ticket link) is already
            // the page the join ran on. When the join installed the space,
            // its view mounted while the space was absent, and a route
            // change to where the page already is re-mounts nothing: the
            // view stayed on its loading state until a manual reload. Load
            // the space afresh instead, in place of the entry, which also
            // keeps a ticket link's seed out of the history. A space that
            // was already here (a page that opened its own space a moment
            // before the directory listed it) needs no reload.
            if opens_in_place(&url) && !outcome.renewed {
                crate::router::navigate::notify_replace(env.client(), &href);
            } else {
                crate::router::navigate::notify_navigate(env.client(), &href);
            }
            log!(
                "join: succeeded (subject {}, key {})",
                outcome.subject,
                outcome.key
            );
        }
        // A visit to a space's address carried no link that could have
        // expired, so it shows no failure: the page it is on already
        // says the space is private and how to get in.
        Err(failure) if is_bare_space_address(&url) => {
            clear_join_overlay(&session, &status_entity);
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            log!("join: the space did not open ({})", failure.kind().as_str());
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

/// No page exists on this host to prompt; the background sync loop (or
/// the host's own drain) picks the commit up on its ordinary cadence.
/// Mirrors the native arm of `broadcast` — structural parity so commit
/// sites don't need their own `cfg`s.
pub(crate) fn notify_sync(client: Option<&crate::router::ClientId>) {
    let _ = client;
}

/// The inviteless-`/join` guard, pinned on every target: it decides
/// whether the route redeems an invite or returns home.
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

    /// A ticket link is the space's address with a key seed for its
    /// fragment; the same address without one is the space, which a
    /// visitor stays on rather than being sent home.
    #[test]
    fn it_reads_a_ticket_link_as_an_invite_and_the_bare_space_as_a_visit() {
        use super::{is_bare_space_address, is_space_address};
        let space = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let seed = bs58::encode([6u8; 32]).into_string();

        assert!(carries_invite(&format!(
            "https://tonk.space/space/{space}#{seed}"
        )));
        assert!(!carries_invite(&format!(
            "https://tonk.space/space/{space}"
        )));
        assert!(is_space_address(&format!(
            "https://tonk.space/space/{space}"
        )));
        assert!(!is_space_address("https://tonk.space/join"));

        assert!(is_bare_space_address(&format!(
            "https://tonk.space/space/{space}"
        )));
        assert!(!is_bare_space_address(&format!(
            "https://tonk.space/space/{space}#{seed}"
        )));
        assert!(!is_bare_space_address(&format!(
            "https://tonk.space/space/{space}/inspector"
        )));
    }

    /// A join from a space's own address, bare or as a ticket link, raw or
    /// percent-encoded, runs on that space's page, so its success reloads
    /// the page in place; a join from anywhere else navigates to the space.
    #[test]
    fn it_reloads_in_place_after_a_join_from_the_spaces_own_address() {
        use super::opens_in_place;
        let space = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let encoded = space.replace(':', "%3A");
        let seed = bs58::encode([6u8; 32]).into_string();

        for address in [
            format!("https://tonk.space/space/{space}"),
            format!("https://tonk.space/space/{encoded}"),
            format!("https://tonk.space/space/{space}#{seed}"),
            format!("https://tonk.space/space/{encoded}#{seed}"),
        ] {
            assert!(opens_in_place(&address), "{address}");
        }
        assert!(!opens_in_place("https://tonk.space/join?access=abc"));
        assert!(!opens_in_place(&format!(
            "https://tonk.space/space/{space}/inspector"
        )));
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

/// Native end-to-end coverage for the invite's advisory `name`: a fresh
/// join seeds the account directory's [`tonk_schema::SpaceName`] from the
/// link, a renewal never overwrites what the directory already carries,
/// and a later authoritative record supersedes the seeded value — the
/// full "labeled immediately, stale at worst, catches up" contract.
#[cfg(test)]
mod invite_name_tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_query::{Query, Term};
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;
    use tonk_invite::{Invite, InviteAudience};

    /// Hand-craft an audience-open, remote-free invite URL carrying a
    /// display name. Distinct tag bytes give distinct subjects and
    /// ephemerals, so tests never collide on a routing key.
    async fn named_invite_url(subject_tag: u8, ephemeral_tag: u8, name: &str) -> (String, String) {
        seeded_invite_url([subject_tag; 32], [ephemeral_tag; 32], name).await
    }

    /// [`named_invite_url`] for a space no run of these tests has seen: a
    /// native run keeps what it stores, and a test of what is NOT stored
    /// for a space needs one nothing stored before.
    async fn fresh_invite_url(name: &str) -> (String, String) {
        seeded_invite_url(rand::random(), rand::random(), name).await
    }

    async fn seeded_invite_url(
        subject_seed: [u8; 32],
        ephemeral_seed: [u8; 32],
        name: &str,
    ) -> (String, String) {
        let subject_signer = Ed25519Signer::import(&subject_seed).await.unwrap();
        let subject = subject_signer.did();
        let key = subject.repo_key().to_owned();
        let ephemeral = Ed25519Signer::import(&ephemeral_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(subject_signer))
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let invite = Invite::new(
            DelegationChain::new(delegation),
            InviteAudience::Open {
                seed: ephemeral_seed,
            },
            None,
        )
        .await
        .unwrap()
        .with_space_name(Some(name.to_owned()));
        (invite.to_url("https://tonk.network/join").unwrap(), key)
    }

    /// The directory's name rows for `key`, read off the profile branch
    /// the way the Hub reads them.
    async fn directory_name(state: &crate::router::AppState, key: &str) -> Vec<String> {
        let subject: dialog_varsig::Did = key.parse().expect("subject parses");
        let tonk = state.read().await;
        let profile = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .expect("profile branch opens");
        let rows: Vec<tonk_schema::SpaceName> = profile
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceName> {
                this: Term::from(subject.this()),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("space-name query");
        rows.into_iter().map(|row| row.name.0).collect()
    }

    #[dialog_common::test]
    async fn local_space_link_rejects_another_subject_before_joining() {
        let state = crate::router::command::tests::native::test_state().await;
        let (invite, key) = named_invite_url(60, 61, "Unapproved space").await;
        let subject: Did = key.parse().unwrap();
        let approved = Ed25519Signer::generate().await.unwrap().did();
        let result = join_for_local_space_link(&state, &invite, &approved).await;
        let tonk = state.read().await;
        assert!(matches!(result, Err(TonkWorkerError::Forbidden(_))));
        assert!(!find_replica_for_subject(&tonk, &subject).await.unwrap());
        assert!(
            tonk.profile
                .access()
                .prove(
                    dialog_capability::Subject::from(subject.clone())
                        .attenuate(dialog_effects::Use)
                )
                .audience(&tonk.operator)
                .perform(&tonk.operator)
                .await
                .is_err()
        );
        drop(tonk);
        join_for_local_space_link(&state, &invite, &subject)
            .await
            .expect("the approved invitation still joins normally");
        assert!(
            find_replica_for_subject(&*state.read().await, &subject)
                .await
                .unwrap()
        );
        assert_eq!(directory_name(&state, &key).await, vec!["Unapproved space"]);
    }

    /// Stand in for the space's own worker, answering a claim with
    /// `outcome`: the fact it would write, as the first frame of the
    /// subscription the profile opens on it.
    fn answer_claims_with(outcome: &'static str) {
        crate::router::space_reach::stand_in::answer_with(move |(_, method, _)| {
            Ok(if method == "SUBSCRIBE" {
                let frame = serde_json::json!({
                    "kind": "snapshot",
                    "conclusions": [{
                        "this": "tonk:membership-claim/stand-in",
                        "fields": { "outcome": outcome, "detail": "" }
                    }]
                });
                serde_json::Value::String(format!("data: {frame}\n\n"))
            } else {
                serde_json::json!({})
            })
        });
    }

    /// Where each space has a worker of its own, joining one mounts nothing
    /// in the profile: the claim is handed to that worker as a command on
    /// the space's branch, the outcome is read off a subscription opened
    /// first, and the space is then listed from what the invitation said.
    #[dialog_common::test]
    async fn it_hands_a_claim_to_the_spaces_own_worker_and_mounts_nothing() {
        let state = crate::router::command::tests::native::test_state().await;
        state
            .read()
            .await
            .site_origins
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (url, key) = fresh_invite_url("Held Elsewhere").await;
        let subject: Did = key.parse().unwrap();

        answer_claims_with("claimed");
        let joined = join_invite(&state, &url).await;
        let asked = crate::router::space_reach::stand_in::asked();

        assert!(joined.is_ok(), "the join fails: {:?}", joined.err());
        let branch = format!("/api/repository/{key}/branch/main");
        assert_eq!(
            asked
                .iter()
                .map(|(space, method, path)| (space.as_str(), method.as_str(), path.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    key.as_str(),
                    "SUBSCRIBE",
                    format!("{branch}/query").as_str()
                ),
                (key.as_str(), "POST", format!("{branch}/transact").as_str()),
            ],
            "the outcome is subscribed to before the claim is committed"
        );
        let tonk = state.read().await;
        assert!(find_replica_for_subject(&tonk, &subject).await.unwrap());
        assert!(
            tonk.profile
                .space(key.as_str())
                .load()
                .perform(&tonk.operator)
                .await
                .is_err(),
            "the profile mounts nothing for the space"
        );
        drop(tonk);
        assert_eq!(directory_name(&state, &key).await, vec!["Held Elsewhere"]);
    }

    /// A claim the space's own worker refuses is a join that failed the way
    /// that worker said, and leaves the space unlisted.
    #[dialog_common::test]
    async fn it_lists_nothing_when_the_spaces_own_worker_refuses_a_claim() {
        let state = crate::router::command::tests::native::test_state().await;
        state
            .read()
            .await
            .site_origins
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (url, key) = fresh_invite_url("Refused").await;
        let subject: Did = key.parse().unwrap();

        answer_claims_with("revoked");
        let joined = join_invite(&state, &url).await;
        crate::router::space_reach::stand_in::asked();

        let failure = joined.err().expect("the join is refused");
        assert_eq!(failure.kind(), JoinFailureKind::Revoked);
        let tonk = state.read().await;
        assert!(!find_replica_for_subject(&tonk, &subject).await.unwrap());
        assert!(
            crate::router::space_directory::configuration(&tonk, &subject)
                .await
                .unwrap()
                .is_none(),
            "the space is no longer expected"
        );
        assert!(
            crate::router::space_directory::held(&tonk, &key)
                .await
                .is_err(),
            "the space is not the person's"
        );
    }

    #[dialog_common::test]
    async fn it_seeds_the_directory_name_from_the_invite_and_lets_the_record_catch_up() {
        let state = crate::router::command::tests::native::test_state().await;
        let (url, key) = named_invite_url(0xA1, 0xA2, "Garden Plans").await;

        // Fresh join: the space is labeled the moment it appears, from
        // the (possibly stale) mint-time name — not nameless until sync.
        join_invite(&state, &url).await.expect("the join succeeds");
        assert_eq!(
            directory_name(&state, &key).await,
            vec!["Garden Plans".to_string()],
            "a fresh join seeds the directory name from the invite"
        );

        // Renewal with a fresher link: the directory keeps what it has.
        // The second link's name is DIFFERENT precisely to prove the
        // renewal path never writes it — by the time a replica exists,
        // the local record is at least as fresh as any link.
        let (renewal, renewal_key) = named_invite_url(0xA1, 0xA3, "Stale Old Label").await;
        assert_eq!(key, renewal_key, "same subject, same routing key");
        join_invite(&state, &renewal)
            .await
            .expect("the renewal succeeds");
        assert_eq!(
            directory_name(&state, &key).await,
            vec!["Garden Plans".to_string()],
            "a renewed join must not overwrite the directory name"
        );

        // Catch-up: the authoritative record path (what the account
        // reconcile and the rename provider both write through) is
        // cardinality-one on the directory entity, so the seeded value
        // is superseded in place, never accumulated beside.
        {
            let tonk = state.read().await;
            let subject: dialog_varsig::Did = key.parse().unwrap();
            let configuration = invite_configuration(&subject, None, None).unwrap();
            super::super::repository::record_space_mount(
                &tonk,
                &subject,
                &configuration,
                Some("Hydrated Real Name"),
            )
            .await;
        }
        assert_eq!(
            directory_name(&state, &key).await,
            vec!["Hydrated Real Name".to_string()],
            "the authoritative record supersedes the invite-seeded name"
        );
    }

    /// A PASTED short link, end to end through the command provider: the
    /// deep-link flow lets the browser follow the 301, but a paste hands
    /// the worker the `/@/{hash}#seed` form verbatim — without
    /// resolution it carries no `access=` and would silently read as a
    /// bare visit. The one-shot server is the shortcut host answering
    /// the redirect; the fragment (the seed) rides the pasted string,
    /// never the wire.
    #[dialog_common::test]
    async fn it_resolves_a_pasted_short_link_before_joining() {
        use dialog_artifacts::Statement as _;
        use dialog_query::the;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let state = crate::router::command::tests::native::test_state().await;
        let (long, key) = named_invite_url(0xC1, 0xC2, "Shortcut Space").await;
        let (target, fragment) = long
            .split_once('#')
            .expect("an open invite carries a seed fragment");
        let target = target.to_owned();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the shortcut host binds");
        let port = listener
            .local_addr()
            .expect("the bound address reads")
            .port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("the probe connects");
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: {target}\r\nContent-Length: 0\r\n\r\n"
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("the redirect writes");
        });
        let short = format!(
            "http://127.0.0.1:{port}/@/2eyEBFxYVkAy4zRTAtpJEeXAWyzScUYDkxhaizAgZgcF#{fragment}"
        );

        // The same transient the Hub paste form commits, through the
        // full dispatch path (profile origin selects the vocabulary
        // that carries Join).
        let mut changes = dialog_artifacts::Changes::new();
        the!("xyz.tonk.command.join/url")
            .of("cmd:join".parse::<dialog_artifacts::Entity>().unwrap())
            .is(short)
            .assert(&mut changes);
        crate::router::command::dispatch(
            &state,
            crate::router::command::CommandOrigin::default(),
            changes,
        )
        .await;
        server.await.expect("the shortcut host served");

        assert_eq!(
            directory_name(&state, &key).await,
            vec!["Shortcut Space".to_string()],
            "a pasted short link resolves to the long form and joins, \
             seeding the invite's signed name"
        );
    }

    #[dialog_common::test]
    async fn it_joins_a_nameless_invite_without_inventing_a_name() {
        let state = crate::router::command::tests::native::test_state().await;
        let subject_signer = Ed25519Signer::import(&[0xB1; 32]).await.unwrap();
        let subject = subject_signer.did();
        let key = subject.repo_key().to_owned();
        let ephemeral = Ed25519Signer::import(&[0xB2; 32]).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(subject_signer))
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let invite = Invite::new(
            DelegationChain::new(delegation),
            InviteAudience::Open { seed: [0xB2; 32] },
            None,
        )
        .await
        .unwrap();
        let url = invite.to_url("https://tonk.network/join").unwrap();

        join_invite(&state, &url).await.expect("the join succeeds");
        assert_eq!(
            directory_name(&state, &key).await,
            Vec::<String>::new(),
            "a link minted before the name rode the URL seeds nothing"
        );
    }
}
