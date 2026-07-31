//! Best-effort backup of a delegation chain to the account service, so a
//! later device can recover the space. Covers both a claimed space's
//! `space -> eph -> root` chain and a created space's one-hop
//! `space -> root` chain.

#[cfg(test)]
use std::cell::Cell;

use dialog_credentials::Ed25519Signer;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_repository::{Repository, RepositoryExt as _};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;
use tonk_common::log;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// What gets backed up per space: the delegation chain plus the invite's
/// sync URL, which the chain itself does not carry. A restoring device
/// needs both to mount and sync the space. The chain is either a claimed
/// space's `space -> eph -> root` chain or a created space's one-hop
/// `space -> root` chain.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ClaimBackup {
    /// Hex-encoded delegation chain: `space -> eph -> root` for a claimed
    /// space, or the one-hop `space -> root` for a created space.
    pub chain_hex: String,
    /// The invite's remote/sync URL, when it carried one.
    pub remote_url: Option<String>,
    /// Explicit invitation-revocation relay, when configured.
    #[serde(default)]
    pub revocation_url: Option<String>,
}

#[cfg(test)]
thread_local! {
    static BACKUP_DISPATCHES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn backup_dispatch_count() -> usize {
    BACKUP_DISPATCHES.with(Cell::get)
}

#[cfg(test)]
fn capture_backup_dispatch() {
    BACKUP_DISPATCHES.with(|count| count.set(count.get() + 1));
}

/// Resolve the account-service base URL attached to this profile.
pub(crate) async fn account_service_url(tonk: &TonkState) -> Option<String> {
    crate::router::account::provider(tonk).await
}

fn endpoint(value: String) -> Result<url::Url, TonkWorkerError> {
    url::Url::parse(&value)
        .map_err(|error| TonkWorkerError::Internal(format!("invalid service endpoint: {error}")))
}

/// List the keys of every chain this account has backed up. Used by
/// restore to discover what can be pulled from the account service.
pub(crate) async fn list_backed_up_chains(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
) -> Result<Vec<String>, TonkWorkerError> {
    let body = tonk_identity::request::build_device_invocation(
        device.clone(),
        link,
        vec!["account".into(), "chain".into(), "list".into()],
        std::collections::BTreeMap::new(),
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build list invocation: {e}")))?;
    let endpoint = endpoint(format!("{}/chains/list", service.trim_end_matches('/')))?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    serde_json::from_slice(&response.body)
        .map_err(|e| TonkWorkerError::Internal(format!("parse chain keys: {e}")))
}

/// Fetch one backed-up chain's raw artifact bytes by key. Used by restore
/// to pull down a chain discovered via [`list_backed_up_chains`].
pub(crate) async fn get_backed_up_chain(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<Vec<u8>, TonkWorkerError> {
    let arguments = [("key".to_owned(), Promised::String(key.to_owned()))]
        .into_iter()
        .collect();
    let body = tonk_identity::request::build_device_invocation(
        device.clone(),
        link,
        vec!["account".into(), "chain".into(), "get".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build get invocation: {e}")))?;
    let endpoint = endpoint(format!("{}/chains/get", service.trim_end_matches('/')))?;
    Ok(super::http::post_cbor(&endpoint, &body).await?.body)
}

/// Build the backup artifact, sign the device invocation, and POST it to
/// the account service. Takes only owned data so it can run detached from
/// the caller (see [`back_up_claim`]).
async fn run_backup(
    device: Ed25519Signer,
    link: DelegationChain,
    service: String,
    chain: DelegationChain,
    remote_url: Option<String>,
    revocation_url: Option<String>,
) -> Result<(), TonkWorkerError> {
    let chain_bytes = chain
        .to_bytes()
        .map_err(|e| TonkWorkerError::Internal(format!("serialize claimed chain: {e}")))?;
    let artifact = ClaimBackup {
        chain_hex: hex::encode(chain_bytes),
        remote_url,
        revocation_url,
    };
    let artifact_bytes = serde_json::to_vec(&artifact)
        .map_err(|e| TonkWorkerError::Internal(format!("serialize backup artifact: {e}")))?;

    let arguments = [(
        "chain".to_owned(),
        Promised::String(hex::encode(artifact_bytes)),
    )]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "chain".into(), "put".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build backup invocation: {e}")))?;

    let endpoint = endpoint(format!("{}/chains/put", service.trim_end_matches('/')))?;
    super::http::post_cbor(&endpoint, &body).await?;
    Ok(())
}

/// Resolve the account link, service URL, and device signer, then hand the
/// backup off to [`run_backup`]. Shared by every backup caller
/// ([`back_up_claim`] and [`back_up_owned_space`]): a no-op when the
/// profile is unlinked or the account service is unknown for this host.
///
/// The lookups here (account link, service URL, device signer) are cheap
/// local reads, so they run inline. The actual network POST is handed off
/// to run detached: on wasm via `spawn_local`, so a slow/hung account
/// service can never stall the caller's `.await`; on native the caller has
/// no UI to stall, so it awaits inline, bounded by the typed transport's
/// request timeout.
async fn dispatch_backup(
    tonk: &TonkState,
    context: &'static str,
    chain: DelegationChain,
    remote_url: Option<String>,
    revocation_url: Option<String>,
) {
    // Only account-holders back up; an unlinked device has no account to
    // escrow under and returns early.
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return;
    };
    let Some(service) = account_service_url(tonk).await else {
        return;
    };
    let device = tonk.profile.signer().signer().clone();

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) =
                run_backup(device, link, service, chain, remote_url, revocation_url).await
            {
                log!("{context} backup failed: {error}");
            }
        });
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        if let Err(error) =
            run_backup(device, link, service, chain, remote_url, revocation_url).await
        {
            log!("{context} backup failed: {error}");
        }
    }
}

/// Back up a claimed space's delegation to the account service.
/// Best-effort: any failure logs and is swallowed — the claiming device
/// already works, and the roster keys on the root regardless.
pub(crate) async fn back_up_claim(
    tonk: &TonkState,
    chain: &DelegationChain,
    remote_url: Option<&str>,
    revocation_url: Option<&str>,
) {
    #[cfg(test)]
    capture_backup_dispatch();
    dispatch_backup(
        tonk,
        "claim",
        chain.clone(),
        remote_url.map(str::to_owned),
        revocation_url.map(str::to_owned),
    )
    .await;
}

/// Back up every existing owned root prefix after provider attachment.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn back_up_existing_spaces(tonk: &TonkState) {
    for key in crate::router::profile_name::real_space_keys(tonk).await {
        let repository = match tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(repository) => repository,
            Err(_) => continue,
        };
        let remote = match crate::router::create_invite::resolve_remote_url(tonk, &repository).await
        {
            Ok(crate::router::create_invite::RemoteRequirement::Ready(remote)) => remote,
            _ => continue,
        };
        back_up_owned_space(
            tonk,
            &repository,
            remote.access_url.as_str(),
            Some(remote.revocation_url.as_str()),
        )
        .await;
    }
}

/// Back up a created space's `space -> root` delegation so another of the
/// account's devices can restore it. Best-effort and fire-and-forget; a
/// no-op when the profile is unlinked, or when `repository` doesn't hold a
/// signer (a joined/verifier-only space has nothing to delegate from —
/// only the space that created it can mint this).
///
/// Only called from the wasm worker today (its one hook is
/// `enable_sync_inner`, which is itself worker-only), so this — like that
/// hook — is worker-only rather than carrying dead code on native.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn back_up_owned_space(
    tonk: &TonkState,
    repository: &Repository,
    remote_url: &str,
    revocation_url: Option<&str>,
) {
    if let Err(error) = try_back_up_owned_space(tonk, repository, remote_url, revocation_url).await
    {
        log!("created-space backup skipped: {error}");
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn try_back_up_owned_space(
    tonk: &TonkState,
    repository: &Repository,
    remote_url: &str,
    revocation_url: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let prefix = crate::router::repository::space_root_prefix(tonk, &repository.did()).await?;
    dispatch_backup(
        tonk,
        "created-space",
        prefix,
        Some(remote_url.to_owned()),
        revocation_url.map(str::to_owned),
    )
    .await;
    Ok(())
}
