//! `POST /api/profile/join`: redeem an invite URL.
//!
//! Joining means: parse the invite, persist the delegation chain
//! to the profile, and ensure a local replica for the invited
//! subject exists. Two outcomes:
//!
//! - **Joined** — there was no replica for this subject; one was
//!   created, keyed by the subject DID. The name is not chosen here:
//!   it lives in the shared repository's content branch and arrives
//!   over the pull the recipient triggers by querying the repo. 201
//!   Created.
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

use ::axum::{Json, extract::State, http::StatusCode};
use axum_wasm_macros::wasm_compat;
use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Repository, RepositoryExt as _, SiteAddress};
use dialog_ucan::UcanDelegation;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_invite::Invite;
use tonk_schema::{
    Invitation, InvitedVia, MemberName, MemberRole, Membership, Replica, prelude::DidExt as _,
};

use super::AppState;
use super::repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, build_repository_info, mark_replica_initialized, record_repository_meta,
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

/// Redeem an invite URL.
#[wasm_compat]
pub async fn join(
    State(state): State<AppState>,
    Json(body): Json<JoinRequest>,
) -> Result<(StatusCode, Json<JoinResponse>), TonkWorkerError> {
    let tonk = state.write().await;
    let outcome = claim_invite(&tonk, &body.url).await?;
    log!(
        "POST /api/profile/join → subject {} (key {})",
        outcome.subject,
        outcome.key
    );
    let repository = tonk
        .profile
        .repository(outcome.key.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to load joined replica: {e}")))?;
    let info = build_repository_info(&tonk, &outcome.key, &repository).await;
    let (status, response) = if outcome.renewed {
        (StatusCode::OK, JoinResponse::Renewed { repository: info })
    } else {
        (
            StatusCode::CREATED,
            JoinResponse::Joined { repository: info },
        )
    };
    Ok((status, Json(response)))
}

/// The result of a successful claim: the routing key, subject DID, and
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

/// Parse + claim an invite URL and ensure a local replica exists.
///
/// Shared by the HTTP `/api/profile/join` route and the `tonk:join`
/// command provider. Persists the delegation chain, and either surfaces
/// the existing replica (`renewed: true`) or creates one (`renewed:
/// false`). Errors carry a recipient-readable message; the command
/// provider maps them to a `tonk:join/failure`.
pub(crate) async fn claim_invite(
    tonk: &TonkState,
    url: &str,
) -> Result<JoinOutcome, TonkWorkerError> {
    // Parse the invite first — the subject DID drives the
    // existing-replica lookup, and a malformed invite shouldn't
    // touch any state.
    let invite = Invite::parse_url(url)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?;

    // Derive the invitation record from the chain as parsed — before
    // the claim pushes a redelegation and changes the leaf. Guaranteed
    // Some by the Invite invariant (specific subject).
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");

    let claimed = invite
        .claim(&tonk.profile.did())
        .await
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?;

    let subject = claimed.subject().clone();
    let remote_url = claimed.remote_url.clone();
    let chain = claimed.chain;

    // Always persist the delegation chain. Idempotent at the
    // dialog layer — re-saving the same chain is a no-op,
    // re-saving an extended one adds a fresh proof. Either way
    // the recipient's effective access can only grow, never
    // shrink, by joining.
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to persist delegation chain: {e}"))
        })?;

    // The shared repository's DID is its identity; the routing/storage
    // key is the DID suffix. There is no local display name — it lives in
    // the repository's own content branch.
    let key = subject.repo_key().to_owned();

    // If the recipient already has a replica for this subject, we're done
    // — surface the existing replica as `Renewed`. The chain refresh we
    // just did is kept; the replica is mounted at the routing key
    // (identity), not a stored label.
    if find_replica_for_subject(tonk, &subject).await? {
        // Renewed: the replica already exists, but still record the roster
        // facts for this claim — a renewing invite can carry a fresh
        // invitation, and provenance is first-wins so re-stamping is a no-op.
        let repository = tonk
            .profile
            .repository(key.as_str())
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "replica '{key}' present in profile meta but failed to load: {e}",
                ))
            })?;
        record_claim_on_content(tonk, &repository, &key, &invitation).await?;
        return Ok(JoinOutcome {
            key,
            subject,
            renewed: true,
        });
    }

    // Create a verifier-only credential keyed to the invited
    // subject DID, then mount it as a local replica at the routing
    // key (so path == identity). Local DID == invited subject DID, so
    // `Replica.this` and the sigil glyph converge across recipients.
    let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|e| {
        TonkWorkerError::Router(format!(
            "invite subject is not a valid Ed25519 did:key: {e:?}"
        ))
    })?;
    let credential = Credential::from(verifier);

    let space_capability = Subject::from(tonk.profile.did()).attenuate(Space::new(&key));
    let space_credential = space_capability
        .create(credential)
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "failed to create local replica '{key}' for invited subject: {e}",
            ))
        })?;
    let repository = Repository::from(space_credential);

    // Mirror what `PUT /api/repository/{name}` writes: a single
    // `main` branch, plus an `origin` remote tracking the
    // invite's access service if one was attached.
    let mut configuration = RepositoryConfiguration::default();
    if let Some(url) = remote_url {
        let address = SiteAddress::from(UcanAddress::new(url.as_str()));
        configuration = configuration
            .remote(
                DEFAULT_REMOTE,
                RemoteConfiguration::new(address).subject(subject.clone()),
            )
            .branch(
                DEFAULT_BRANCH,
                BranchConfiguration {
                    upstream: Some(UpstreamConfiguration::new(DEFAULT_REMOTE, DEFAULT_BRANCH)),
                    revision: None,
                },
            );
    } else {
        configuration = configuration.branch(DEFAULT_BRANCH, BranchConfiguration::default());
    }

    // No display name to seed: a joined repo's name lives in the shared
    // content branch and arrives over the pull the Hub triggers when it
    // queries the repo for its name. `record_repository_meta` only uses
    // this for log context + the home-demo check (never matched here), so
    // the routing key stands in.
    record_repository_meta(
        tonk,
        &repository,
        &key,
        &configuration,
        tonk_schema::MemberRole::MEMBER,
    )
    .await?;
    record_claim_on_content(tonk, &repository, &key, &invitation).await?;

    // `record_repository_meta` stamps the replica `blank` (the create
    // path's "still seeding" state). A joined replica has no local seed
    // step — its content arrives over the pull the recipient triggers —
    // so flip it straight to `initialized`, otherwise its Hub card is
    // stuck on "Installing…" forever.
    mark_replica_initialized(tonk, &subject).await?;

    log!("Joined invite for subject {subject} as local replica (key {key})");

    Ok(JoinOutcome {
        key,
        subject,
        renewed: false,
    })
}

/// Record the roster facts for a claimed invite on the repo's content
/// branch: the invitation itself (idempotent when the minter already
/// wrote it; self-healing when the invite predates invitation
/// records), the claimer's membership stamped with the `member` role,
/// and — first-wins — the `InvitedVia` provenance stamp.
///
/// The content branch (not meta) because it's the synced, shared branch:
/// every member pulls it, so the roster converges across the space. A
/// roster on the device-local meta branch would only ever show the
/// claimer's own row.
///
/// The roster lives on the content branch (`main`) because that branch
/// syncs across replicas; the meta branch is local-only, so a roster
/// written there never converges between the inviter and the claimer.
///
/// First-wins: provenance answers "how did this member first get in",
/// so an existing stamp is never overwritten by a later claim
/// (`invitation` is cardinality-one and a re-assert would silently
/// replace the original inviter). Self-claims (the claimer minted the
/// invitation) are not provenance and are skipped.
async fn record_claim_on_content<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    key: &str,
    invitation: &Invitation,
) -> Result<(), TonkWorkerError>
where
    C: dialog_varsig::Principal + Clone,
{
    let membership = Membership::new(tonk.profile.did(), repository.did());

    // Route both the read and the write through the *reactor's* cached
    // `main` handle (keyed by the routing key) rather than a fresh
    // `repository.branch().open()`. Background sync pulls/publishes through
    // the reactor's cached handle; a commit on a separate handle leaves it
    // pinned at a stale head, so a later pull's CAS fails forever
    // (`VersionMismatch`), wedging all `main` sync. Going through the
    // reactor advances the cached handle and re-polls its subscriptions.
    let session = tonk
        .reactor
        .repository(key)
        .branch(DEFAULT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to open repo content branch: {e}"))
        })?;

    // First-wins: look for any existing provenance stamp on this membership.
    let stamps: Vec<InvitedVia> = session
        .handle()
        .query()
        .select(Query::<InvitedVia> {
            this: Term::var("this"),
            invitation: Term::var("invitation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("invited-via query failed: {e:?}")))?;
    let already_stamped = stamps.iter().any(|s| s.this == *membership.this());

    // First-wins on role too: `role` is cardinality-one, so blindly
    // stamping `member` would DEMOTE a founder who reclaims their own
    // invite. Only stamp when the membership has no role yet.
    let roles: Vec<MemberRole> = session
        .handle()
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("member-role query failed: {e:?}")))?;
    let already_roled = roles.iter().any(|r| r.this == *membership.this());

    // A member claiming their own invite is not provenance.
    let self_invite = invitation.inviter.0 == tonk.profile.did().this();

    let display_name = crate::router::profile_name::resolve_display_name(tonk).await;
    let member_name = MemberName::new(membership.this().clone(), display_name);
    let mut transaction = tonk
        .reactor
        .repository(key)
        .branch(DEFAULT_BRANCH)
        .transaction()
        .assert(invitation.clone())
        .assert(membership.clone())
        .assert(member_name);
    if !already_roled {
        transaction = transaction.assert(MemberRole::member(membership.this().clone()));
    }
    if !already_stamped && !self_invite {
        transaction = transaction.assert(InvitedVia::new(
            membership.this().clone(),
            invitation.this().clone(),
        ));
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to record claim on content: {e}"))
        })?;
    Ok(())
}

/// Check whether the active profile already holds a replica for the
/// given subject DID. Returns `Ok(true)` when one exists.
///
/// The replica is a name-less membership index, so this only tests
/// existence — the recipient's chosen join name does not flow into it
/// (the name lives in the synced repository's own `tonk/repository`).
async fn find_replica_for_subject(
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
            kind: Term::var("kind"),
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
/// with the parsed location in the event detail. This handler reassembles
/// the invite URL from the `access`/`remote`/`hash` fields, claims it, and
/// drives the overlay-only `tonk:join/status` (pending → failed, or
/// retract + durable replica on success) on the profile meta branch — the
/// branch the `/join` view subscribes to.
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

        // Decode the parsed-location fields synchronously while the caller
        // holds the lock; hand owned values to the `'static` future.
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

/// Reassemble the invite URL from the command's parsed pieces, claim it,
/// and drive the overlay-only join status. Always leaves the overlay in a
/// terminal state (status retracted on success, `failed` on error).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_join(env: &crate::router::CommandEnv, command: tonk_schema::command::Join) {
    use std::sync::Arc;
    use tonk_schema::command::{JoinFailure, JoinStatus};
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

    // Reassemble the invite URL: ?access=…[&remote=…][#hash]. `access`
    // and `remote` came from the parsed `searchParams` (already decoded),
    // so re-encode them; `hash` keeps its leading `#`.
    let url = build_invite_url(&command);

    match claim_invite(&tonk, &url).await {
        Ok(outcome) => {
            // Success: the durable replica is recorded + initialized. But a
            // *freshly* joined replica has an empty content branch — unlike a
            // locally-created space (which is seeded with the standard library
            // at creation), a joined space's content, including its `tonk/space`
            // view, only arrives over the pull from the remote. Redirecting
            // before that pull lands would drop the recipient on a branch with
            // no view — `<tonk-display>` would resolve "Model not found".
            //
            // So pull the content branch now, while the "Joining…" spinner is
            // still up, and only navigate once it has landed. A renewed replica
            // already has content; a local-only invite has no remote to pull.
            if !outcome.renewed {
                pull_joined_content(&tonk, &outcome.key).await;
            }

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
        Err(error) => {
            // Failure: mark failed + record the reason/kind, overlay-only.
            // Never echo `url` (it carries the seed) into the message.
            let kind = match &error {
                TonkWorkerError::Router(_) => "malformed",
                _ => "claim-failed",
            };
            session.state.assert_overlay(JoinStatus {
                this: status_entity.clone(),
                status: Status(
                    "tonk:failed"
                        .parse()
                        .unwrap_or_else(|_| status_entity.clone()),
                ),
            });
            session.state.assert_overlay(JoinFailure {
                this: status_entity,
                reason: Reason(error.to_string()),
                kind: Kind(kind.to_owned()),
            });
            tonk.reactor.schedule_poll(Arc::clone(&session.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            log!("join: failed ({kind}): {error}");
        }
    }
}

/// Build the invite URL from the command's parsed pieces: the full
/// `search` (incl. `?`, carrying `access` + optional `remote`) and `hash`
/// (incl. `#`) are appended verbatim onto a placeholder origin.
/// `Invite::parse_url` reads only the query + fragment, so the host is
/// irrelevant; it recovers the sync remote from the query when present.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn build_invite_url(command: &tonk_schema::command::Join) -> String {
    let search = &command.search.0; // includes leading `?` (or empty)
    let hash = &command.hash.0; // includes leading `#` (or empty)
    format!("https://join.invalid/join{search}{hash}")
}

/// Pull the freshly joined replica's `main` content branch from its remote,
/// so the standard-library views (notably `tonk/space`) are present before
/// the recipient is redirected into the space. Without this the redirect can
/// land on an empty branch and `<tonk-display>` resolves "Model not found".
///
/// A pull failure is logged, not fatal: the redirect still fires (the
/// recipient lands on the space and a later sync / reload fills it in) — far
/// better than blocking the join on a flaky network.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn pull_joined_content(tonk: &TonkState, key: &str) {
    match tonk
        .reactor
        .repository(key)
        .branch(DEFAULT_BRANCH)
        .pull()
        .perform(&tonk.operator)
        .await
    {
        Ok(_) => log!("join: pulled content for joined replica {key}"),
        Err(e) => log!("join: content pull for {key} did not complete: {e:?}"),
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
    use tonk_schema::{Invitation, MemberRole};

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
            None,
        )
        .await
        .unwrap();
        (invite.to_url("https://hub.tonk.xyz/join").unwrap(), key)
    }

    async fn post_join(app: &axum::Router, url: &str) -> StatusCode {
        let body = serde_json::json!({ "url": url }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/profile/join")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
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
        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        assert!(memberships.iter().any(|m| m.member.0 == profile_entity));

        let invitations = content_invitations(&state, &key).await;
        assert!(
            invitations.iter().any(|i| i.this == expected.this),
            "invitation self-healed from the URL",
        );

        let stamps = content_invited_via(&state, &key).await;
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == profile_entity)
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
        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == profile_entity)
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
        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == profile_entity)
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
        let minted_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/invite"))
                    .method("POST")
                    .header("content-type", "application/json")
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
        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        let membership_entity = memberships
            .iter()
            .find(|m| m.member.0 == profile_entity)
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
}
