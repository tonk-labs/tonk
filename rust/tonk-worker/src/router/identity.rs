//! Persist and validate the provider-neutral local passkey root.

#[cfg(feature = "persona")]
use axum::{Json, extract::State};
#[cfg(feature = "persona")]
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use serde::{Deserialize, Serialize};
#[cfg(feature = "persona")]
use tonk_macros::handler;
use tonk_worker_api::PasskeyMetadata;
#[cfg(feature = "persona")]
use tonk_worker_api::{RootStatus, SaveRootRequest};

#[cfg(feature = "persona")]
use super::AppState;
use crate::TonkWorkerError;
use crate::worker::DefaultProfile;
use crate::worker::TonkState;

pub(crate) const LOCAL_ROOT_SITE: &str = "tonk-local-root-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalRootRecord {
    version: u8,
    credential_id: String,
    delegation: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passkey: Option<PasskeyMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encryption_key: Option<String>,
}

/// A validated local root record.
#[derive(Clone)]
pub(crate) struct LocalRoot {
    /// Passkey-derived root DID.
    pub root_did: dialog_varsig::Did,
    /// Current device DID.
    #[cfg(feature = "persona")]
    pub device_did: dialog_varsig::Did,
    /// Opaque passkey credential ID.
    #[cfg(feature = "persona")]
    pub credential_id: String,
    /// Exact root → device delegation.
    pub delegation: DelegationChain,
    /// Exact serialized delegation bytes.
    pub bytes: Vec<u8>,
    /// Informational creation details recorded by the creating browser.
    #[cfg(feature = "persona")]
    pub passkey: Option<PasskeyMetadata>,
    /// The account's X25519 recipient, when the ceremony that wrote this
    /// record held the secret. Published to the account space by
    /// `seed_sealed_inbox`.
    pub encryption_key: Option<dialog_varsig::Did>,
}

pub(crate) async fn validate_grant(
    bytes: Vec<u8>,
    device_did: &dialog_varsig::Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let chain = DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("invalid root delegation: {error}")))?;
    if chain.proof_cids().len() != 1 {
        return Err(TonkWorkerError::Router(
            "root delegation must contain exactly one proof".to_string(),
        ));
    }
    if chain.audience() != device_did {
        return Err(TonkWorkerError::Forbidden(
            "root delegation audience is not the current profile".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(TonkWorkerError::Router(
            "root delegation must be subject-open".to_string(),
        ));
    }
    let proof = chain.proofs().next().expect("one-proof chain");
    if !proof.command().0.is_empty() {
        return Err(TonkWorkerError::Router(
            "root delegation must be command-open".to_string(),
        ));
    }
    proof
        .verify_signature(&dialog_credentials::DidKeyResolver)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid root delegation signature: {error}"))
        })?;
    Ok(chain)
}

pub(crate) async fn load_record(
    state: &TonkState,
) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
    load_record_from(&state.profile, &state.active_branch).await
}

/// Load and validate the serialized root record belonging to an explicit
/// profile. Account routing uses this without constructing a full TonkState
/// for every inactive roster entry.
async fn load_record_from(
    profile: &DefaultProfile,
    branch: &str,
) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
    let bytes = match profile
        .secrets()
        .site(crate::credential::branch_site(LOCAL_ROOT_SITE, branch).as_str())
        .load::<Vec<u8>>()
        .perform(profile)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load local root: {error}"
            )));
        }
    };
    let record: LocalRootRecord = serde_json::from_slice(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored local root is malformed: {error}"))
    })?;
    if record.version != 1 {
        return Err(TonkWorkerError::Internal(format!(
            "unsupported local root version {}",
            record.version
        )));
    }
    Ok(Some(record))
}

/// Load and validate the local root, failing when it is missing.
pub(crate) async fn local_root(state: &TonkState) -> Result<LocalRoot, TonkWorkerError> {
    let record = load_record(state)
        .await?
        .ok_or(TonkWorkerError::RootRequired)?;
    let device_did = state.profile.did();
    let delegation = validate_grant(record.delegation.clone(), &device_did).await?;
    let encryption_key = record
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    Ok(LocalRoot {
        root_did: delegation.issuer().clone(),
        #[cfg(feature = "persona")]
        device_did,
        #[cfg(feature = "persona")]
        credential_id: record.credential_id,
        delegation,
        bytes: record.delegation,
        #[cfg(feature = "persona")]
        passkey: record.passkey,
        encryption_key,
    })
}

/// Have the peer act for the account its branch signed in to. A branch
/// signed in while branches shared one peer recorded its root, but its
/// own peer starts on an onboarding account; handing it over is what
/// signing in did. A branch never signed in, or one already acting for
/// its root, is left as it is.
pub(crate) async fn follow_signed_in_account(state: &TonkState) -> Result<(), TonkWorkerError> {
    if load_record(state).await?.is_none() {
        return Ok(());
    }
    let root = local_root(state).await?;
    // A repair, not a precondition. Its commit can need a tree node only
    // the account remote holds, and a refused fetch failed the whole boot:
    // the worker then served nothing, sign-out included. The next boot
    // tries again.
    if let Err(error) = reconcile_root_delegation(state, &root.delegation).await {
        tonk_common::log!("boot kept its device grants as they were: {error}");
    }
    if state.profile.authority().await.ok().as_ref() == Some(&root.root_did) {
        return Ok(());
    }
    tonk_account::peer::hand_over(&state.profile, &root.delegation)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to hand the branch's account over to its signed-in account: {error}"
            ))
        })
}

/// Return the verified local root DID.
#[allow(dead_code)]
pub(crate) async fn root_did(state: &TonkState) -> Result<dialog_varsig::Did, TonkWorkerError> {
    Ok(local_root(state).await?.root_did)
}

#[cfg(feature = "persona")]
fn status(root: LocalRoot) -> RootStatus {
    RootStatus::Ready {
        root_did: root.root_did.to_string(),
        device_did: root.device_did.to_string(),
        credential_id: root.credential_id,
        delegation_cid: root.delegation.proof_cids()[0].to_string(),
        delegation_hex: hex::encode(root.bytes),
        passkey: root.passkey,
        encryption_key: root.encryption_key.map(|did| did.to_string()),
    }
}

/// `GET /api/identity/root`.
#[cfg(feature = "persona")]
#[handler]
pub async fn get(State(state): State<AppState>) -> Result<Json<RootStatus>, TonkWorkerError> {
    let state = state.read().await;
    match load_record(&state).await? {
        None => Ok(Json(RootStatus::Missing {
            device_did: state.profile.did().to_string(),
        })),
        Some(_) => Ok(Json(status(local_root(&state).await?))),
    }
}

/// Whether this profile is linked to an account without the account's
/// encryption key: it signed in through another deployment, or was linked
/// before that key existed. A profile linked to no account is not.
pub(crate) async fn linked_without_key(state: &TonkState) -> bool {
    matches!(load_record(state).await, Ok(Some(record)) if record.encryption_key.is_none())
}

/// Forget the local root record: the device no longer holds a grant.
///
/// Signing out ends here, after the grant is retracted and its
/// revocation published, so the next sign-in has to reopen the passkey
/// to mint a new one. Idempotent, like the retract it performs.
#[cfg(feature = "persona")]
pub(crate) async fn forget_root(state: &TonkState) -> Result<(), TonkWorkerError> {
    state
        .profile
        .secrets()
        .site(crate::credential::branch_site(LOCAL_ROOT_SITE, &state.active_branch).as_str())
        .retract()
        .perform(&state.profile)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("failed to forget local root: {error}")))
}

#[cfg(feature = "persona")]
pub(crate) async fn persist_root(
    state: &TonkState,
    request: SaveRootRequest,
) -> Result<RootStatus, TonkWorkerError> {
    if request.credential_id.is_empty() {
        return Err(TonkWorkerError::Router(
            "credentialId must not be empty".to_string(),
        ));
    }
    let passkey = request
        .passkey
        .map(|mut metadata| {
            metadata.created_on = metadata.created_on.trim().to_string();
            if metadata.created_at == 0 {
                return Err(TonkWorkerError::Router(
                    "passkey createdAt must be greater than zero".to_string(),
                ));
            }
            if metadata.created_on.is_empty()
                || metadata.created_on.chars().count() > 120
                || metadata.created_on.chars().any(char::is_control)
            {
                return Err(TonkWorkerError::Router(
                    "passkey createdOn must be a readable device label".to_string(),
                ));
            }
            Ok(metadata)
        })
        .transpose()?;
    request
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = validate_grant(bytes.clone(), &state.profile.did()).await?;
    let mut record = LocalRootRecord {
        version: 1,
        credential_id: request.credential_id,
        delegation: bytes.clone(),
        passkey,
        encryption_key: request.encryption_key,
    };
    if let Some(existing) = load_record(state).await? {
        let stored_root = DelegationChain::try_from(existing.delegation.as_slice())
            .ok()
            .map(|stored| stored.issuer().clone());
        // Only a ceremony that held the secret records the recipient, so
        // a re-save from one that did not (a link, a re-minted grant)
        // keeps the one already recorded for this root.
        if stored_root.as_ref() == Some(chain.issuer()) && record.encryption_key.is_none() {
            record.encryption_key = existing.encryption_key.clone();
        }
        if existing == record {
            reconcile_root_delegation(state, &chain).await?;
            return Ok(status(local_root(state).await?));
        }
        // Same root, different record — a re-minted grant or refreshed
        // metadata — updates in place; that covers signing back in after
        // sign-out. A DIFFERENT root is another account's passkey: each
        // account keeps its own profile, so it lands through add-account
        // and never overwrites the root this profile's spaces hang off.
        // An unreadable stored delegation refuses too — the roots can't
        // be proven equal.
        //
        // A different root is refused only while the branch the profile
        // is on follows another account: that branch's spaces and
        // delegations hang off the account it follows, and a different
        // account arrives through add-account, on a branch that follows
        // nothing. A branch following nothing takes any root — the
        // half-created record a failed registration leaves behind must
        // not wedge the profile, and after signing out the branch is
        // free for whichever account signs in next.
        if stored_root.as_ref() != Some(chain.issuer()) {
            use tonk_schema::prelude::DidExt as _;
            if let Some(followed) = super::profile::active_account(state).await
                && followed != chain.issuer().this()
            {
                return Err(TonkWorkerError::Conflict(
                    "a different account is signed in on this branch; \
                     use \"Add account\" to sign in with another account"
                        .to_string(),
                ));
            }
            tonk_common::log!(
                "replacing a root the branch does not follow: {} -> {}",
                stored_root
                    .map(|did| did.to_string())
                    .unwrap_or_else(|| "unreadable".to_string()),
                chain.issuer()
            );
        }
    }

    // Retain before persisting the canonical record. Cleanup follows the
    // record write, so an interrupted cleanup retries with the new grant.
    state
        .profile
        .access()
        .save(UcanDelegation(chain.clone()))
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save root delegation: {error}"))
        })?;
    let encoded = serde_json::to_vec(&record).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize local root: {error}"))
    })?;
    state
        .profile
        .secrets()
        .site(crate::credential::branch_site(LOCAL_ROOT_SITE, &state.active_branch).as_str())
        .save(encoded)
        .perform(&state.profile)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save local root: {error}"))
        })?;
    // The peer now acts for the account it signed in to.
    tonk_account::peer::hand_over(&state.profile, &chain)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to hand the profile's account over to the signed-in account: {error}"
            ))
        })?;
    reconcile_root_delegation(state, &chain).await?;

    let encryption_key = record
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    Ok(status(LocalRoot {
        root_did: chain.issuer().clone(),
        device_did: state.profile.did(),
        credential_id: record.credential_id,
        delegation: chain,
        bytes,
        passkey: record.passkey,
        encryption_key,
    }))
}

/// A fresh root record must also be the grant the local prover selects.
/// Serialize with profile writes; a concurrent pull can still move the head,
/// so retry that typed conflict with a refreshed view of retained grants.
pub(super) async fn reconcile_root_delegation(
    state: &TonkState,
    chain: &DelegationChain,
) -> Result<(), TonkWorkerError> {
    use dialog_repository::{CommitError, PublishError};
    use tonk_account::delegations::{DeviceGrantError, reconcile_device_delegation};

    let branch = state
        .reactor
        .profile_repository()
        .branch(&state.active_branch)
        .acquire(&state.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open device grant branch: {error}")))?;
    let _transacting = branch.transactor().lock().await;
    for attempt in 0..=4 {
        match reconcile_device_delegation(branch.handle(), chain, &state.operator).await {
            Ok(()) => return Ok(()),
            Err(DeviceGrantError::Commit(CommitError::Publish(
                PublishError::VersionMismatch { .. },
            ))) if attempt < 4 => continue,
            Err(error) => {
                return Err(TonkWorkerError::Internal(format!(
                    "reconcile device grant: {error}"
                )));
            }
        }
    }
    unreachable!("the last attempt returns its error")
}

/// Parse a ceremony-supplied recipient, refusing anything that is not
/// an X25519 `did:key`: a record holding an Ed25519 DID here would seal
/// every seed to a key nothing can open.
fn parse_encryption_key(did: &str) -> Result<dialog_varsig::Did, TonkWorkerError> {
    let did: dialog_varsig::Did = did
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid encryptionKey: {error}")))?;
    tonk_identity::sealed::RecipientKey::try_from(&did)
        .map_err(|error| TonkWorkerError::Router(format!("invalid encryptionKey: {error}")))?;
    Ok(did)
}

/// `POST /api/identity/root`.
#[cfg(feature = "persona")]
#[handler]
pub async fn save(
    State(state): State<AppState>,
    Json(request): Json<SaveRootRequest>,
) -> Result<Json<RootStatus>, TonkWorkerError> {
    let state = state.read().await;
    let status = persist_root(&state, request).await?;
    // A save that carries creation metadata is a passkey arriving — at
    // signup or at a later custody enrollment. The passkey's own row is
    // written by the ceremony, which is the only place that holds both
    // the custody DID it keys on and the creation label. Seed the
    // sealed-inbox address now rather than on the next sweep, so the
    // dashboard reflects the enrollment immediately.
    if super::account_state::seed_sealed_inbox(&state).await {
        tonk_common::log!("published the account's encryption key in the account space");
    }
    Ok(Json(status))
}
