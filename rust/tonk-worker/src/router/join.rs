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
    log!("POST /api/profile/join");

    let tonk = state.write().await;

    // Parse the invite first — the subject DID drives the
    // existing-replica lookup, and a malformed invite shouldn't
    // touch any state.
    let invite = Invite::parse_url(&body.url)
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
    if find_replica_for_subject(&tonk, &subject).await? {
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
        let info = build_repository_info(&tonk, &key, &repository).await;
        return Ok((
            StatusCode::OK,
            Json(JoinResponse::Renewed { repository: info }),
        ));
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
    record_repository_meta(&tonk, &repository, &key, &configuration).await?;

    // `record_repository_meta` stamps the replica `blank` (the create
    // path's "still seeding" state). A joined replica has no local seed
    // step — its content arrives over the pull the recipient triggers —
    // so flip it straight to `initialized`, otherwise its Hub card is
    // stuck on "Installing…" forever.
    mark_replica_initialized(&tonk, &subject).await?;

    log!("Joined invite for subject {subject} as local replica (key {key})");

    let info = build_repository_info(&tonk, &key, &repository).await;
    Ok((
        StatusCode::CREATED,
        Json(JoinResponse::Joined { repository: info }),
    ))
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
