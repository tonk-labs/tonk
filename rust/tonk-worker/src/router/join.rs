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
use tonk_schema::{Invitation, InvitedVia, MemberName, Membership, Replica, prelude::DidExt as _};

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
        record_claim_on_meta(&tonk, &repository, &invitation).await?;
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
    record_claim_on_meta(&tonk, &repository, &invitation).await?;

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

/// Record the roster facts for a claimed invite on the repo's meta
/// branch: the invitation itself (idempotent when the minter already
/// wrote it; self-healing when the invite predates invitation
/// records), the claimer's membership, and — first-wins — the
/// `InvitedVia` provenance stamp.
///
/// First-wins: provenance answers "how did this member first get in",
/// so an existing stamp is never overwritten by a later claim
/// (`invitation` is cardinality-one and a re-assert would silently
/// replace the original inviter). Self-claims (the claimer minted the
/// invitation) are not provenance and are skipped.
async fn record_claim_on_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    invitation: &Invitation,
) -> Result<(), TonkWorkerError>
where
    C: dialog_varsig::Principal + Clone,
{
    let membership = Membership::new(tonk.profile.did(), repository.did());

    let meta = repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to open repo meta branch: {e}")))?;

    // First-wins: look for any existing stamp on this membership.
    let stamps: Vec<InvitedVia> = meta
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

    // A member claiming their own invite is not provenance.
    let self_invite = invitation.inviter.0 == tonk.profile.did().this();

    let member_name = MemberName::new(membership.this().clone(), tonk.profile_name.clone());
    let mut transaction = meta
        .transaction()
        .assert(invitation.clone())
        .assert(membership.clone())
        .assert(member_name);
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
        .map_err(|e| TonkWorkerError::Internal(format!("failed to record claim on meta: {e}")))?;
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
    use tonk_schema::Invitation;
    use tonk_schema::prelude::DidExt as _;

    use crate::router::api_router_with_state;
    use crate::router::tests::{
        meta_invitations, meta_invited_via, meta_memberships, put_repo, test_state,
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

        let memberships = meta_memberships(&state, &key).await;
        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        assert!(memberships.iter().any(|m| m.member.0 == profile_entity));

        let invitations = meta_invitations(&state, &key).await;
        assert!(
            invitations.iter().any(|i| i.this == expected.this),
            "invitation self-healed from the URL",
        );

        let stamps = meta_invited_via(&state, &key).await;
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
    }

    /// Claiming an invite names the claimer on the repo meta.
    #[dialog_common::test]
    async fn it_records_the_claimer_name_on_join() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = handcrafted_invite_url(30, 31).await;

        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let memberships = meta_memberships(&state, &key).await;
        let names = crate::router::tests::meta_member_names(&state, &key).await;
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
        let invitations = meta_invitations(&state, &key).await;
        assert!(
            invitations.iter().any(|i| i.this == expected_a.this),
            "first invitation recorded",
        );
        assert!(
            invitations.iter().any(|i| i.this == expected_b.this),
            "renewed join records the second invitation too",
        );

        let stamps = meta_invited_via(&state, &key).await;
        // Exactly one stamp for this membership, still pointing at the
        // first invitation.
        let memberships = meta_memberships(&state, &key).await;
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
        let key = put_repo(&app, "test-self-claim").await;
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
        let memberships = meta_memberships(&state, &key).await;
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
        let stamps = meta_invited_via(&state, &key).await;
        assert!(
            !stamps.iter().any(|s| s.this == membership_entity),
            "self-claims must not stamp provenance",
        );
    }
}
