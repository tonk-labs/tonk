//! Pull the account's backed-up space delegations and mount any space
//! this device does not already have. Best-effort: failures log and are
//! swallowed; nothing here blocks link or boot.

use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use tonk_common::log;

use crate::router::account_backup::{
    ClaimBackup, account_service_url, get_backed_up_chain, list_backed_up_chains,
};
use crate::worker::TonkState;

/// Restore all backed-up spaces for the linked account. No-op when
/// unlinked or when the account service is unreachable.
#[allow(
    dead_code,
    reason = "wired into link/startup triggers in a follow-up task"
)]
pub(crate) async fn restore_spaces(tonk: &TonkState) {
    if let Err(error) = try_restore_spaces(tonk).await {
        log!("restore skipped: {error}");
    }
}

async fn try_restore_spaces(tonk: &TonkState) -> Result<(), crate::TonkWorkerError> {
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return Ok(());
    };
    let Some(service) = account_service_url() else {
        return Ok(());
    };
    let device = tonk.profile.signer().signer().clone();

    let keys = list_backed_up_chains(&device, &link, &service).await?;
    for key in keys {
        if let Err(error) = restore_one(tonk, &device, &link, &service, &key).await {
            // One bad artifact must not stop the rest.
            log!("restore of chain '{key}' skipped: {error}");
        }
    }
    Ok(())
}

async fn restore_one(
    tonk: &TonkState,
    device: &dialog_credentials::Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<(), crate::TonkWorkerError> {
    let bytes = get_backed_up_chain(device, link, service, key).await?;
    let artifact: ClaimBackup = serde_json::from_slice(&bytes)
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad backup artifact: {e}")))?;
    let chain_bytes = hex::decode(&artifact.chain_hex)
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad chain hex: {e}")))?;
    let chain = DelegationChain::try_from(chain_bytes.as_slice())
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad chain: {e}")))?;

    let subject = chain
        .subject()
        .ok_or_else(|| crate::TonkWorkerError::Internal("backup chain has no subject".into()))?
        .clone();

    // Already have it? Nothing to do.
    if crate::router::join::find_replica_for_subject(tonk, &subject).await? {
        return Ok(());
    }

    // Install the delegation so presign's BFS can compose it with the
    // local root -> device link, then mount and let sync bring the roster.
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|e| crate::TonkWorkerError::Internal(format!("save restored delegation: {e}")))?;

    // Restore writes no content-branch roster (`Membership`/`MemberRole`/
    // `MemberName`) — the roster is authoritative on the content branch
    // and arrives over sync. Writing a role here would demote a founder
    // on a space this account created.
    crate::router::join::mount_replica(tonk, &subject, artifact.remote_url.as_deref()).await?;
    crate::router::repository::mark_replica_initialized(tonk, &subject).await?;
    Ok(())
}
