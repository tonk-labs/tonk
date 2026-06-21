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
use tonk_schema::{Replica, prelude::DidExt as _};

use super::AppState;
use super::repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, build_repository_info, mark_replica_initialized, record_repository_meta,
};
use crate::{TonkWorkerError, worker::TonkState};

/// Name of the meta branch on the profile repository.
const META_BRANCH: &str = "meta";

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
    record_repository_meta(tonk, &repository, &key, &configuration).await?;

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
        .branch(META_BRANCH)
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
        .branch(META_BRANCH)
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
            notify_navigate(env.client(), &href);
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

/// Post a `{ type: "navigate", href }` message to the originating client so
/// it redirects there. This is how a worker-side command performs a page
/// capability: the service worker has no `window`, and the command is
/// transient (it never lands in a branch a subscription could observe), so
/// the originating client is the only path back to the page that asked.
///
/// No-ops (with a log) when the client is unknown or its handle can't be
/// resolved — the join still succeeded; only the convenience redirect is
/// lost, and the recipient can navigate from the Hub.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn notify_navigate(client: Option<&crate::router::ClientId>, href: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let Some(client) = client else {
        log!("join: no originating client to navigate; skipping redirect");
        return;
    };
    let client_id = client.0.clone();
    let href = href.to_owned();

    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(g) => g,
        Err(_) => {
            log!("join: not in a service worker scope; skipping redirect");
            return;
        }
    };

    // `clients.get(id)` resolves the live `Client` handle; post the message
    // on it. Done on a spawned task so the caller isn't blocked on the
    // round-trip (the navigate is fire-and-forget).
    spawn_local(async move {
        let client_value = match JsFuture::from(global.clients().get(&client_id)).await {
            Ok(value) if !value.is_undefined() && !value.is_null() => value,
            Ok(_) => {
                log!("join: originating client {client_id} is gone; skipping redirect");
                return;
            }
            Err(e) => {
                log!("join: clients.get failed: {e:?}");
                return;
            }
        };
        let Ok(client) = client_value.dyn_into::<web_sys::Client>() else {
            log!("join: clients.get did not yield a Client; skipping redirect");
            return;
        };

        // `{ type: "navigate", href }` — the page's `<tonk-host>` listens
        // for `navigate` messages and assigns `window.location`.
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("navigate"),
        );
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("href"),
            &JsValue::from_str(&href),
        );
        if let Err(e) = client.post_message(&message) {
            log!("join: post_message(navigate) failed: {e:?}");
        }
    });
}
