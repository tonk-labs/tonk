//! Reading spaces anyone may read.
//!
//! A published space keeps a `/use/get` ticket for the public principal
//! (see [`tonk_invite::public`]). The account holds a powerline from that
//! principal, so once a space's public ticket is saved in the profile it
//! proves for the account, and from there for this device, the same way
//! a membership does.

use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::Did;
use tonk_common::log;

use crate::{TonkWorkerError, worker::TonkState};

/// The account's powerline from the public principal, minted and saved
/// in the profile when it holds none.
///
/// Every account gets one when it is created; an account created before
/// public spaces gets it the first time it opens one. Minting is not
/// repeatable (each delegation carries a fresh nonce), so one already
/// held is proven and reused rather than minted again.
///
/// # Errors
///
/// When the powerline cannot be minted or saved.
pub(crate) async fn ensure_powerline(
    tonk: &TonkState,
    account: &Did,
) -> Result<(), TonkWorkerError> {
    let public = tonk_invite::public::did()
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    if proves(tonk, account, &public, "/").await {
        return Ok(());
    }
    let powerline = tonk_invite::public::powerline(account)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    save(tonk, powerline).await
}

/// Whether this profile can already read `subject`: a membership, or a
/// public ticket saved here before.
pub(crate) async fn can_read(tonk: &TonkState, subject: &Did) -> bool {
    proves(tonk, &tonk.profile.did(), subject, "/use/get").await
}

/// Save `chain` in the profile, where every proof this device makes is
/// searched for.
///
/// # Errors
///
/// When the profile does not keep it.
pub(crate) async fn save(tonk: &TonkState, chain: DelegationChain) -> Result<(), TonkWorkerError> {
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("failed to save access: {error}")))
}

/// Whether the profile's delegations prove `command` on `subject` for
/// `audience`. Every failure reads as "no": the caller's next step is to
/// go and get the access, which is always safe.
async fn proves(tonk: &TonkState, audience: &Did, subject: &Did, command: &str) -> bool {
    let Ok(command) = Command::parse(command) else {
        return false;
    };
    let branch = match tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("public access: profile branch did not open: {error}");
            return false;
        }
    };
    branch
        .handle()
        .delegations()
        .prove(
            audience.clone(),
            Scope {
                subject: UcanSubject::Specific(subject.clone()),
                command,
                parameters: Parameters::default(),
            },
        )
        .perform(&tonk.operator)
        .await
        .is_ok()
}
