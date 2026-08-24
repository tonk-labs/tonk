//! Proxy the account service's device registry for the linked profile.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan::{Parameters, Scope};
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationChain, promise::Promised};
use dialog_varsig::Did;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_worker_api::{
    AccountDevice, AccountSummary, PasskeyMetadata, RevokeDeviceAcknowledgement,
    RevokeDeviceRequest,
};

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// The linked account provider's base URL, when an account is attached.
pub(crate) async fn account_service_url(tonk: &TonkState) -> Option<String> {
    crate::router::account::provider(tonk).await
}

/// Resolve the stored link and service URL, or explain what's missing.
pub(super) async fn linked_service(
    state: &TonkState,
) -> Result<(dialog_ucan_core::DelegationChain, String), TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let service = account_service_url(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("no account service is attached to this profile".to_string())
    })?;
    Ok((link, service))
}

/// The audience DID dialog recorded for a retained delegation.
///
/// `dialog.ucan/audience` is written onto the delegation's own entity
/// when the chain is retained, so this reads the device's identity from
/// the same record that carries its label — no second source to drift.
async fn delegation_audience(
    branch: &dialog_repository::Branch,
    state: &TonkState,
    entity: &dialog_artifacts::Entity,
) -> Option<String> {
    use dialog_artifacts::{ArtifactSelector, Value};
    use futures_util::StreamExt as _;

    let selector = ArtifactSelector::new()
        .the(dialog_repository::DELEGATION_AUDIENCE.parse().ok()?)
        .of(entity.clone());
    let facts = branch
        .claims()
        .select(selector)
        .perform(&state.operator)
        .await
        .ok()?
        .collect::<Vec<_>>()
        .await;
    for fact in facts.into_iter().flatten() {
        if let Ok(Value::String(did)) = fact.value() {
            return Some(did.to_string());
        }
    }
    None
}

/// This account's device links, from local facts.
///
/// Every field the account service returned is derivable here: dialog
/// decomposes issuer/audience onto each retained delegation's entity,
/// and [`DeviceLink`] adds the label and creation time. That makes the
/// list render offline, and removes the drift a projection invites —
/// the service's row could say "revoked" over a device that still
/// reaches storage, because nothing consulted it when authorizing.
///
/// Revoked devices are absent rather than listed as revoked: revoking
/// retracts the delegation, so the row goes with the authority it
/// described.
async fn local_devices(state: &TonkState) -> Result<Vec<AccountDevice>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};

    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open account branch to list devices: {error}"))
        })?;
    let links: Vec<tonk_schema::DeviceLink> = branch
        .handle()
        .query()
        .select(Query::<tonk_schema::DeviceLink> {
            this: Term::var("this"),
            created_at: Term::var("created_at"),
            title: Term::var("title"),
            reason: Term::var("reason"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("query device links: {error}")))?;

    let this_device = state.profile.did().to_string();
    let mut devices = Vec::with_capacity(links.len());
    for link in links {
        // The audience is the device; dialog wrote it onto the same
        // entity when the chain was retained.
        let Some(did) = delegation_audience(branch.handle(), state, &link.this).await else {
            continue;
        };
        devices.push(AccountDevice {
            attachment_id: link.this.to_string(),
            this_device: did == this_device,
            did,
            name: link.title.0,
            status: "active".to_string(),
            created_at: link.created_at.0,
            delegation_cid: link.this.to_string(),
            delegation_hex: None,
        });
    }
    Ok(devices)
}

/// List the devices authorized under this profile's account.
///
/// Read from local delegation facts rather than the account service: a
/// device IS its `account -> profile` delegation, and dialog retains
/// that with issuer/audience decomposed onto its entity. Serving the
/// list locally makes it work offline and removes a projection that
/// could disagree with the authority it described.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(local_devices(&state).await?))
}

/// `POST /api/account/devices/register` request: a device the approving
/// page just authorized, to be recorded in the account service's
/// registry under this profile's account.
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

/// Register a freshly authorized device in the account service's
/// registry. The service only accepts registration from a device that is
/// already an active member, which this browser is — a device authorized
/// over a callback cannot register itself.
#[wasm_compat]
pub async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterDeviceRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let state = state.read().await;
    let (link, service) = linked_service(&state).await?;
    let device = state.profile.signer().signer().clone();
    let arguments = [
        ("did".to_owned(), Promised::String(request.did)),
        ("name".to_owned(), Promised::String(request.name)),
        (
            "delegation".to_owned(),
            Promised::String(request.delegation_hex),
        ),
    ]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "register".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-register invocation: {e}")))?;
    let endpoint = url::Url::parse(&format!(
        "{}/devices/register",
        service.trim_end_matches('/')
    ))
    .map_err(|error| TonkWorkerError::Internal(format!("invalid account provider URL: {error}")))?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let answer: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| TonkWorkerError::Internal(format!("parse registration answer: {e}")))?;
    Ok(Json(answer))
}

/// The account service's `POST /account/summary` response.
///
/// Deliberately its own type rather than [`AccountSummary`]: the provider hop
/// and the local hop no longer carry the same shape, and decoding the provider
/// straight into the local DTO is what would silently re-couple them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSummary {
    email: String,
    passkey: Option<PasskeyMetadata>,
}

/// Prefer the portable account-space fact; fall back to what the provider
/// recorded at account creation.
///
/// The provider row is not a legacy-only path. Every account still writes it at
/// creation, and it answers three live cases: an account created before the
/// space fact existed, a device that never held the passkey and so cannot seed
/// it, and a fresh account read in the window between account creation and the
/// first sweep that seeds the space.
fn merge_summary(
    email: Option<String>,
    space: Option<PasskeyMetadata>,
    provider: Option<PasskeyMetadata>,
) -> AccountSummary {
    AccountSummary {
        email,
        passkey: space.or(provider),
    }
}

/// Verified account facts for the linked profile, preferring what the account
/// repository carries over what the provider recorded.
///
/// Shared by the HTTP route and the roster hooks that capture the
/// account email best-effort at link time.
pub(crate) async fn account_summary(state: &TonkState) -> Result<AccountSummary, TonkWorkerError> {
    let (link, service) = linked_service(state).await?;
    let space = super::account_state::passkey_facts(state).await;
    let body = tonk_identity::request::build_device_invocation(
        state.profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "summary".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|error| {
        TonkWorkerError::Internal(format!("build account-summary invocation: {error}"))
    })?;
    let endpoint = url::Url::parse(&format!(
        "{}/account/summary",
        service.trim_end_matches('/')
    ))
    .map_err(|error| TonkWorkerError::Internal(format!("invalid account provider URL: {error}")))?;
    match super::http::post_cbor(&endpoint, &body).await {
        Ok(response) => {
            let provider: ProviderSummary =
                serde_json::from_slice(&response.body).map_err(|error| {
                    TonkWorkerError::Internal(format!("parse account summary: {error}"))
                })?;
            Ok(merge_summary(Some(provider.email), space, provider.passkey))
        }
        // The account repository already answered the passkey question, so an
        // unreachable provider costs the email and nothing else. With no space
        // fact there is nothing to serve, and the caller keeps the real error.
        Err(error) if space.is_some() => {
            log!("account summary falling back to account-space facts: {error}");
            Ok(merge_summary(None, space, None))
        }
        Err(error) => Err(error.into()),
    }
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
/// The scope a device grant proves: the account subject, root command.
///
/// A device link is a powerline, so its subject is the account rather
/// than any one space, and `/` is the command it carries.
fn account_scope(link: &DelegationChain) -> Scope {
    let account = link
        .subject()
        .cloned()
        .unwrap_or_else(|| link.issuer().clone());
    Scope {
        subject: UcanSubject::Specific(account),
        command: Command::parse("/").expect("the root command always parses"),
        parameters: Parameters::default(),
    }
}

async fn delegated_revocation(
    state: &TonkState,
    link: &DelegationChain,
    target_did: &Did,
) -> Result<String, TonkWorkerError> {
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
    Ok(hex::encode(
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
    ))
}

async fn self_revocation(
    state: &TonkState,
    link: &DelegationChain,
) -> Result<String, TonkWorkerError> {
    let target = link.proof_cids()[0];
    Ok(hex::encode(
        tonk_identity::revocation::mint_self_revocation(
            state.profile.signer().signer().clone(),
            link,
            &target,
        )
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("build self-revocation: {error}")))?,
    ))
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
async fn publish_revocation(state: &TonkState, artifact: &[u8]) -> Result<(), TonkWorkerError> {
    use dialog_remote_ucan_s3::UcanAddress;
    use std::collections::BTreeSet;

    let mut endpoints: BTreeSet<String> = BTreeSet::new();

    // The account's own service, from its signed descriptor rather than
    // from configuration.
    if let Some(descriptor) = super::account::descriptor(state).await {
        endpoints.insert(
            UcanAddress::new(descriptor.remote().as_str())
                .endpoint()
                .to_string(),
        );
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

/// Revoke a device, using device-signed self-revocation for the caller or a
/// passkey/root-signed artifact for another device.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<RevokeDeviceAcknowledgement>, TonkWorkerError> {
    let state = state.read().await;
    let own = request.did == state.profile.did().to_string();
    let target_did = request.did.clone();
    let (link, service) = linked_service(&state).await?;
    let revocation = if own {
        self_revocation(&state, &link).await?
    } else if request.revocation.is_empty() {
        // A caller-supplied artifact is no longer required: this device's
        // own account grant is a powerline, so it can prove for the
        // account subject and mint the revocation itself.
        let target: Did = target_did.parse().map_err(|error| {
            TonkWorkerError::Conflict(format!("'{target_did}' is not a did:key: {error:?}"))
        })?;
        delegated_revocation(&state, &link, &target).await?
    } else {
        // Still honoured when supplied, so a passkey-signed artifact
        // minted elsewhere keeps working.
        request.revocation
    };
    // Publish first: the account service's row is a projection of this,
    // and a row saying "revoked" over a device that still reaches storage
    // is worse than no row at all.
    let artifact = hex::decode(&revocation).map_err(|error| {
        TonkWorkerError::Conflict(format!("revocation artifact is not valid hex: {error}"))
    })?;
    publish_revocation(&state, &artifact).await?;

    let device = state.profile.signer().signer().clone();
    let arguments = [
        (
            "attachmentId".to_owned(),
            Promised::String(request.attachment_id),
        ),
        ("did".to_owned(), Promised::String(request.did)),
        ("revocation".to_owned(), Promised::String(revocation)),
    ]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-revoke invocation: {e}")))?;
    let endpoint = url::Url::parse(&format!("{}/devices/revoke", service.trim_end_matches('/')))
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invalid account provider URL: {error}"))
        })?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let acknowledgement: RevokeDeviceAcknowledgement = serde_json::from_slice(&response.body)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("parse device-revoke acknowledgement: {error}"))
        })?;
    if acknowledgement.target_did != target_did {
        return Err(TonkWorkerError::Internal(
            "account provider acknowledged a different device".to_string(),
        ));
    }
    if acknowledgement.target_cid.is_empty() || !acknowledgement.published {
        return Err(TonkWorkerError::Internal(
            "account provider did not confirm canonical revocation publication".to_string(),
        ));
    }
    Ok(Json(acknowledgement))
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

    #[dialog_common::test]
    async fn it_refuses_to_list_devices_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        assert!(matches!(
            list(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
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
    fn it_prefers_the_account_space_passkey_fact_over_the_provider_row() {
        let space = PasskeyMetadata {
            created_at: 1_754_380_800,
            created_on: "Chrome on macOS".to_string(),
        };
        let provider = PasskeyMetadata {
            created_at: 1_600_000_000,
            created_on: "Safari on iOS".to_string(),
        };

        let merged = merge_summary(
            Some("person@example.com".to_string()),
            Some(space.clone()),
            Some(provider.clone()),
        );
        assert_eq!(merged.passkey, Some(space.clone()));

        let fallback = merge_summary(
            Some("person@example.com".to_string()),
            None,
            Some(provider.clone()),
        );
        assert_eq!(
            fallback.passkey,
            Some(provider),
            "an account created before the space fact existed still has the provider row"
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
        let artifact = hex::decode(self_revocation(&tonk, &link).await.unwrap()).unwrap();
        let verified = tonk_identity::revocation::verify(&artifact).await.unwrap();
        assert_eq!(verified.target_cid, link.proof_cids()[0].to_string());
    }

    #[dialog_common::test]
    async fn it_refuses_to_revoke_without_a_signed_revocation() {
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
        assert!(
            matches!(
                revoke(
                    State(state),
                    Json(RevokeDeviceRequest {
                        attachment_id: "test-attachment".to_string(),
                        did: "did:key:zOtherDevice".to_string(),
                        revocation: String::new(),
                    })
                )
                .await,
                Err(TonkWorkerError::Conflict(_))
            ),
            "cutting off another device takes a passkey-signed revocation"
        );
    }
}
