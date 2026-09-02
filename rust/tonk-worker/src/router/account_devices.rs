//! Devices as facts in the account space: list, register, revoke.
//!
//! A device IS its `account -> profile` delegation. The chain is
//! retained into profile main at the moment it is minted, a
//! [`tonk_schema::DeviceLink`] fact carries the label dialog does not,
//! and the branch every device syncs doubles as the registry. Only the
//! account summary still proxies the account service.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::delegations::account_scope;
use tonk_common::log;
use tonk_worker_api::{
    AccountDevice, AccountSummary, PasskeyMetadata, RevokeDeviceAcknowledgement,
    RevokeDeviceRequest,
};

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// This account's device links and their audiences, from local facts.
///
/// The query itself is [`tonk_schema::device_link::device_links`],
/// shared with the CLI so both adapters list the same thing; what is
/// local here is how the branch is reached.
///
/// Revoked devices are absent rather than listed as revoked: revoking
/// retracts the link's facts, so the row goes with the authority it
/// described.
async fn local_devices(
    state: &TonkState,
) -> Result<Vec<(tonk_schema::DeviceLink, String)>, TonkWorkerError> {
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open account branch to list devices: {error}"))
        })?;
    tonk_schema::device_link::device_links(branch.handle(), &state.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("query device links: {error}")))
}

/// List the devices authorized under this profile's account.
///
/// Read from local delegation facts rather than the account service: a
/// device IS its `account -> profile` delegation, and dialog retains
/// that with issuer/audience decomposed onto its entity. Serving the
/// list locally makes it work offline and removes a projection that
/// could disagree with the authority it described.
///
/// One row per device: a device described more than once (a historical
/// grant minted alongside a fresh one) keeps its earliest link time,
/// since the extra grants are the same authorization re-minted.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    use std::collections::BTreeMap as DeviceIndex;

    let state = state.read().await;
    // Self-describing keeps the list honest about this device even when
    // no sweep has run yet — a fresh sign-up renders its dashboard
    // before the account has ever hydrated. Local-only and idempotent:
    // an already-described link is one query and no commit.
    super::account_state::describe_own_device(&state).await;
    let mut rows: DeviceIndex<String, AccountDevice> = DeviceIndex::new();
    for (link, did) in local_devices(&state).await? {
        let row = AccountDevice {
            did,
            name: link.title.0,
            created_at: link.created_at.0,
        };
        match rows.get_mut(&row.did) {
            Some(existing) if existing.created_at <= row.created_at => {}
            Some(existing) => *existing = row,
            None => {
                rows.insert(row.did.clone(), row);
            }
        }
    }
    let mut devices: Vec<AccountDevice> = rows.into_values().collect();
    devices.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(Json(devices))
}

/// `POST /api/account/devices/register` request: a device the approving
/// page just authorized, to be described in the account space under
/// this profile's account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDeviceRequest {
    /// The registered device's DID.
    pub did: String,
    /// Display name for the device registry.
    pub name: String,
    /// Hex-encoded `root → device` delegation the device will present.
    pub delegation_hex: String,
}

/// Record a freshly authorized device in the account space itself.
///
/// The grant's chain is retained into profile main — the branch every
/// device of the account syncs — and a [`DeviceLink`] fact with the
/// device's label lands on the delegation's own entity. The approving
/// page runs this because it is the side that MINTS the grant: minting
/// is where the label is known, and asserting there is what makes the
/// row reach every device without a registry to ask.
///
/// The push is what makes registration visible before the grant is even
/// delivered: the waiting device's first account pull brings the row
/// down with the authority it describes. A profile with no provider keeps
/// the row local. Otherwise, if the one-shot push races the worker's ordinary
/// account sweep, finish through that serialized sweep before delivering the
/// grant.
///
/// [`DeviceLink`]: tonk_schema::DeviceLink
#[wasm_compat]
pub async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterDeviceRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let state = state.read().await;
    let bytes = hex::decode(&request.delegation_hex).map_err(|error| {
        TonkWorkerError::Conflict(format!("device delegation is not valid hex: {error}"))
    })?;
    let chain = DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        TonkWorkerError::Conflict(format!(
            "device delegation is not a delegation container: {error:?}"
        ))
    })?;
    if chain.audience().to_string() != request.did {
        return Err(TonkWorkerError::Conflict(format!(
            "the delegation is addressed to {}, not to {}",
            chain.audience(),
            request.did
        )));
    }
    crate::onboarding::describe_device_link(&state, &chain, request.name)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("describe the registered device: {error}"))
        })?;
    if let Err(push_error) = super::account_state::push_account_main(&state).await {
        let (status, swept) = super::account_state::ensure_account_state_swept(&state).await;
        match status {
            tonk_account::AccountStateStatus::Unconfigured => {
                log!("registered device remains local: {push_error}");
            }
            tonk_account::AccountStateStatus::Unhydrated => {
                return Err(TonkWorkerError::Internal(format!(
                    "publish the registered device after {push_error}: account state is {status:?}"
                )));
            }
            tonk_account::AccountStateStatus::Ready => swept.map_err(|sweep_error| {
                TonkWorkerError::Internal(format!(
                    "publish the registered device after {push_error}: {sweep_error}"
                ))
            })?,
        }
    }
    // The delegation CID is the attachment id now: it names the exact
    // grant this registration described, which is what the id was for.
    let attachment_id = chain.proof_cids()[0].to_string();
    Ok(Json(serde_json::json!({ "attachmentId": attachment_id })))
}

/// Prefer the portable account-space fact; fall back to this device's local
/// root creation metadata.
///
/// The local root answers the fresh-account window before the first account
/// sweep exposes the portable fact. It is only a fallback: another device's
/// newer account-space fact must win over this device's creation record.
fn merge_summary(
    email: Option<String>,
    space: Option<PasskeyMetadata>,
    local: Option<PasskeyMetadata>,
) -> AccountSummary {
    AccountSummary {
        email,
        passkey: space.or(local),
        display_name: None,
    }
}

/// Verified account facts for the linked profile, preferring what the account
/// repository carries over what the provider recorded.
///
/// Shared by the HTTP route and the roster hooks that capture the
/// account email best-effort at link time.
pub(crate) async fn account_summary(state: &TonkState) -> Result<AccountSummary, TonkWorkerError> {
    // An unlinked profile has no account to summarize, and answering
    // with empty facts would read as "an account with nothing in it".
    super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    // Facts, not a request. Both halves of the summary already ride the
    // account's own sync — the address enrollment recorded, and the
    // passkey metadata the root save wrote — so asking a service for
    // them was asking for a second copy of what this branch holds.
    let email = super::customer::account_registration(state).await.email;
    let passkey = super::account_state::passkey_facts(state).await;
    let local = super::identity::local_root(state)
        .await
        .ok()
        .and_then(|root| root.passkey);
    let mut summary = merge_summary(email, passkey, local);
    summary.display_name = account_display_name(state).await;
    Ok(summary)
}

/// The chosen account display name, straight from the fact — `None` when
/// nobody has named the account yet. Distinct from the roster's
/// display name, which falls back to an auto-generated petname and so
/// cannot say whether a person ever chose one.
async fn account_display_name(state: &TonkState) -> Option<String> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{AccountDisplayName, prelude::DidExt as _};

    let account = super::identity::root_did(state).await.ok()?;
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .ok()?;
    let names: Vec<AccountDisplayName> = branch
        .handle()
        .query()
        .select(Query::<AccountDisplayName> {
            this: Term::from(account.this()),
            name: Term::var("name"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
        .ok()?;
    names
        .into_iter()
        .next()
        .map(|row| row.name.0)
        .filter(|name| !name.trim().is_empty())
}

/// Return verified account facts authorized by this profile's active grant.
#[wasm_compat]
pub async fn summary(
    State(state): State<AppState>,
) -> Result<Json<AccountSummary>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(account_summary(&state).await?))
}

/// Mint a revocation for ANOTHER device, under this device's own
/// account grant.
///
/// No passkey is involved. A device link is a powerline — subject-open
/// and command-open — so any authorized device can prove for the account
/// subject, and that is exactly the authority a revocation of an
/// account-issued grant requires. The passkey requirement this replaces
/// was a workaround from before revocations were enforced, not a
/// property of the model.
///
/// `path` answers "why may this be revoked" — the target's own grant,
/// reconstructed from retained `dialog.ucan/*` facts rather than a
/// stored copy, so it cannot drift from what the proof search reads.
/// `proofs` answers "may this principal invoke at all" — our own link.
/// Mint a revocation for another device, returning the artifact and the
/// CID of the grant it withdraws.
async fn delegated_revocation(
    state: &TonkState,
    link: &DelegationChain,
    target_did: &Did,
) -> Result<(String, String), TonkWorkerError> {
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open account branch to revoke: {error}"))
        })?;
    let proof = branch
        .handle()
        .delegations()
        .prove(target_did.clone(), account_scope(link))
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!("no retained grant reaches {target_did}: {error}"))
        })?;
    let mut certificates = proof.proofs.into_iter();
    let first = certificates
        .next()
        .ok_or_else(|| TonkWorkerError::NotFound(format!("the grant for {target_did} is empty")))?;
    let mut path = DelegationChain::new(first.0);
    for certificate in certificates {
        path = path.push(certificate.0).map_err(|error| {
            TonkWorkerError::Internal(format!("proved certificates do not chain: {error}"))
        })?;
    }
    let target = path.proof_cids()[0];
    let artifact = hex::encode(
        tonk_identity::revocation::mint_delegated_revocation(
            state.profile.signer().signer().clone(),
            &path,
            &target,
            link,
        )
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("cannot revoke {target_did}: {error}"))
        })?,
    );
    Ok((artifact, target.to_string()))
}

/// Mint this device's own revocation, returning the artifact and the CID
/// of the grant it withdraws.
async fn self_revocation(
    state: &TonkState,
    link: &DelegationChain,
) -> Result<(String, String), TonkWorkerError> {
    let target = link.proof_cids()[0];
    let artifact = hex::encode(
        tonk_identity::revocation::mint_self_revocation(
            state.profile.signer().signer().clone(),
            link,
            &target,
        )
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("build self-revocation: {error}")))?,
    );
    Ok((artifact, target.to_string()))
}

/// Retract a revoked device's link rows from the account space.
///
/// The list is driven by [`DeviceLink`] facts, so retracting them is
/// what makes the row leave every device's list — the branch they sit on
/// syncs, and the retraction travels like any other commit. The retained
/// delegation itself stays: it may share certificates with other chains,
/// and the authority it described is already withdrawn where it counts,
/// in every access service's revocation index.
///
/// [`DeviceLink`]: tonk_schema::DeviceLink
async fn retract_device_links(state: &TonkState, target: &str) -> Result<(), TonkWorkerError> {
    let links = local_devices(state).await?;
    let mut transaction = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction();
    let mut retracting = false;
    for (link, did) in links {
        if did == target {
            transaction = transaction.retract(link);
            retracting = true;
        }
    }
    if !retracting {
        return Ok(());
    }
    transaction
        .commit()
        .perform(&state.operator)
        .await
        .map(|_| ())
        .map_err(|error| {
            TonkWorkerError::Internal(format!("retract the revoked device's rows: {error}"))
        })
}

/// Publish the revocation everywhere it could still be honoured.
///
/// The account service records a device revocation onto the device list
/// it renders, which is a projection: nothing consults it when a presign
/// is authorized. The chain walk consults an access service's revocation
/// index, so an artifact that never reaches one withdraws nothing — the
/// revoked device keeps its storage access.
///
/// A device grant is a powerline: not scoped to one space, so not scoped
/// to one access service either. Telling only the account's own service
/// would leave the device serving every space synced elsewhere, so the
/// artifact goes to each distinct endpoint the directory knows about as
/// well.
///
/// Every endpoint must accept it. A partial publication is the dangerous
/// outcome: the user is told the device is gone while one service still
/// serves it, so a refusal anywhere is reported rather than swallowed.
pub(super) async fn publish_revocation(
    state: &TonkState,
    artifact: &[u8],
) -> Result<(), TonkWorkerError> {
    use dialog_remote_ucan_s3::UcanAddress;
    use std::collections::BTreeSet;

    let mut endpoints: BTreeSet<String> = BTreeSet::new();

    // The account's own service: the address enrollment recorded, or
    // this deployment's own when nothing has answered yet.
    if let Ok(remote) = super::account_state::account_remote(state).await {
        endpoints.insert(UcanAddress::new(remote.as_str()).endpoint().to_string());
    }

    // And every service a space in the directory syncs through.
    match state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
    {
        Ok(main) => {
            match tonk_schema::directory::access_endpoints(main.handle(), &state.operator).await {
                Ok(known) => endpoints.extend(known),
                // The directory is a cache of what the account branch says.
                // Failing to read it must not silently narrow a revocation,
                // so it is refused rather than treated as "no other remotes".
                Err(error) => {
                    return Err(TonkWorkerError::Internal(format!(
                        "cannot enumerate the services this revocation must reach: {error:?}"
                    )));
                }
            }
        }
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "cannot open the account directory to publish a revocation: {error}"
            )));
        }
    }

    if endpoints.is_empty() {
        return Err(TonkWorkerError::Conflict(
            "this profile has no access service to publish a revocation to".to_string(),
        ));
    }

    for endpoint in &endpoints {
        let url = url::Url::parse(endpoint).map_err(|error| {
            TonkWorkerError::Internal(format!("unusable access endpoint '{endpoint}': {error}"))
        })?;
        let response = super::http::post_cbor(&url, artifact).await?;
        let receipt: tonk_account::customer::RevokeReceipt = serde_json::from_slice(&response.body)
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "'{endpoint}' returned an unreadable revoke receipt: {error}"
                ))
            })?;
        log!(
            "device revocation recorded at {endpoint}: {}",
            receipt.revoked
        );
    }
    Ok(())
}

/// Revoke a device: mint the artifact, publish it to every access
/// service that honours it, and retract the device's rows from the
/// account space.
///
/// No passkey and no account service are involved. This device's own
/// account grant is a powerline, so it can prove for the account subject
/// and mint the revocation itself — for another device from the target's
/// retained grant, for itself from its own link.
///
/// The order encodes what each failure costs. For another device the
/// publication comes first: enforcement lives in the revocation index,
/// and a row that disappears over a device still reaching storage would
/// be a lie. For this device itself the retraction and its push come
/// first, because a device that has just revoked itself can no longer
/// push anything — the published revocation is what makes its pushes
/// start failing.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<RevokeDeviceAcknowledgement>, TonkWorkerError> {
    let state = state.read().await;
    let own = request.did == state.profile.did().to_string();
    let target_did = request.did.clone();
    let link = super::account::account_link(&state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let (revocation, target_cid) = if own {
        self_revocation(&state, &link).await?
    } else {
        // This needs the TARGET's grant retained here. Grants are
        // retained into the account space at the moment they are minted,
        // so any device that has synced the account can rebuild the path
        // that says why the target may be revoked.
        let target: Did = target_did.parse().map_err(|error| {
            TonkWorkerError::Conflict(format!("'{target_did}' is not a did:key: {error:?}"))
        })?;
        delegated_revocation(&state, &link, &target).await?
    };
    let artifact = hex::decode(&revocation).map_err(|error| {
        TonkWorkerError::Conflict(format!("revocation artifact is not valid hex: {error}"))
    })?;

    if own {
        if let Err(error) = retract_device_links(&state, &target_did).await {
            log!("self-revocation keeps its rows: {error}");
        } else if let Err(error) = super::account_state::push_account_main(&state).await {
            log!("self-revocation retracted its rows but did not push them: {error}");
        }
        publish_revocation(&state, &artifact).await?;
    } else {
        publish_revocation(&state, &artifact).await?;
        // The authority is withdrawn; the rows are bookkeeping. A failure
        // here costs a stale row that any device can retract later, not
        // enforcement, so it is logged rather than turned into "the
        // revocation failed".
        if let Err(error) = retract_device_links(&state, &target_did).await {
            log!("revoked device's rows not retracted: {error}");
        } else if let Err(error) = super::account_state::push_account_main(&state).await {
            log!("revoked device's retraction not yet published: {error}");
        }
    }

    Ok(Json(RevokeDeviceAcknowledgement {
        target_did,
        target_cid,
        published: true,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use std::sync::Arc;

    use axum::Json;
    use axum::extract::State;
    use tokio::sync::RwLock;
    use tonk_worker_api::RevokeDeviceRequest;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    use crate::TonkWorkerError;
    use crate::router::tests::test_state_without_account;

    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A profile that holds a root grant lists itself even before any
    /// account service knows it: the list is local facts and it
    /// describes this device's own link on the way through, so it never
    /// depends on a sweep having run.
    #[dialog_common::test]
    async fn it_lists_this_device_for_a_profile_with_a_root() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let own = { state.read().await.profile.did().to_string() };
        let devices = list(State(state)).await.unwrap();
        assert_eq!(devices.0.len(), 1);
        assert_eq!(devices.0[0].did, own);
    }

    #[dialog_common::test]
    async fn it_refuses_a_summary_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        assert!(matches!(
            summary(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
    }

    #[dialog_common::test]
    fn it_prefers_the_account_space_passkey_fact_over_the_local_root() {
        let space = PasskeyMetadata {
            created_at: 1_754_380_800,
            created_on: "Chrome on macOS".to_string(),
        };
        let local = PasskeyMetadata {
            created_at: 1_600_000_000,
            created_on: "Safari on iOS".to_string(),
        };

        let merged = merge_summary(
            Some("person@example.com".to_string()),
            Some(space.clone()),
            Some(local.clone()),
        );
        assert_eq!(merged.passkey, Some(space.clone()));

        let fallback = merge_summary(
            Some("person@example.com".to_string()),
            None,
            Some(local.clone()),
        );
        assert_eq!(
            fallback.passkey,
            Some(local),
            "a fresh account renders its creation metadata before the first sweep"
        );

        let neither = merge_summary(Some("person@example.com".to_string()), None, None);
        assert_eq!(neither.passkey, None);

        // What an unreachable provider leaves: the portable fact, no address.
        let offline = merge_summary(None, Some(space), None);
        assert_eq!(offline.email, None);
        assert_eq!(offline.passkey.unwrap().created_on, "Chrome on macOS");
    }

    #[dialog_common::test]
    async fn it_self_revokes_without_a_passkey_artifact() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let tonk = state.read().await;
            crate::router::account::tests_matching_request(&tonk).await
        };
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request)
                .await
                .unwrap();
        }
        let tonk = state.read().await;
        let link = crate::router::account::account_link(&tonk).await.unwrap();
        let (artifact, target_cid) = self_revocation(&tonk, &link).await.unwrap();
        let artifact = hex::decode(artifact).unwrap();
        let verified = tonk_identity::revocation::verify(&artifact).await.unwrap();
        assert_eq!(verified.target_cid, link.proof_cids()[0].to_string());
        assert_eq!(target_cid, link.proof_cids()[0].to_string());
    }

    /// Registering describes the device in the account space, and the
    /// list serves it back from those facts — the whole loop without an
    /// account service anywhere.
    #[dialog_common::test]
    async fn it_registers_a_device_as_account_space_facts() {
        use dialog_varsig::Principal as _;

        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let account = dialog_credentials::Ed25519Signer::generate().await.unwrap();
        let device = dialog_credentials::Ed25519Signer::generate().await.unwrap();
        let chain = tonk_account::delegations::mint_account_union(
            &dialog_credentials::Signer::from(account),
            &device.did(),
        )
        .await
        .unwrap();
        let delegation_hex = hex::encode(chain.to_bytes().unwrap());

        let answer = register(
            State(state.clone()),
            Json(RegisterDeviceRequest {
                did: device.did().to_string(),
                name: "e2e terminal".into(),
                delegation_hex,
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            answer.0["attachmentId"],
            chain.proof_cids()[0].to_string(),
            "the delegation CID stands in for the attachment id"
        );

        let devices = list(State(state)).await.unwrap();
        let registered = devices
            .0
            .iter()
            .find(|row| row.did == device.did().to_string())
            .expect("the registered device is listed");
        assert_eq!(registered.name, "e2e terminal");
    }

    /// Revoking another device mints from the target's grant retained in
    /// the account space, so a device the space knows nothing about is
    /// refused with what was missing, not with a demand for an artifact.
    #[dialog_common::test]
    async fn it_refuses_to_revoke_a_device_without_a_retained_grant() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let tonk = state.read().await;
            crate::router::account::tests_matching_request(&tonk).await
        };
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request)
                .await
                .unwrap();
        }
        let other = dialog_credentials::Ed25519Signer::generate().await.unwrap();
        let other_did = {
            use dialog_varsig::Principal as _;
            other.did().to_string()
        };
        assert!(
            matches!(
                revoke(State(state), Json(RevokeDeviceRequest { did: other_did })).await,
                Err(TonkWorkerError::NotFound(_))
            ),
            "an unknown device has no retained grant to rebuild the revocation path from"
        );
    }
}
