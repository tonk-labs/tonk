//! What a space's own worker asks of the person's persona, on a kernel: what
//! only the persona can do for a space, since it holds the lasting authority
//! over it. The persona's host passes each ask on from the space's host, as
//! the profile's worker answered them in a browser (see `inviteSpace`,
//! `spaceAuthority` and `revokeGrant` there).

use dialog_varsig::Did;
use tonk_schema::prelude::DidExt as _;

use super::revoke_invite::{self, RevocationRequest};
use crate::TonkWorkerError;
use crate::router::AppState;

fn space_of(space: &str) -> Result<Did, TonkWorkerError> {
    space
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("{space} is no space: {error:?}")))
}

/// Mint an invite to `space`, as though its share control had been clicked
/// at `time`.
pub async fn invite_space(state: &AppState, space: &str, time: f64) -> Result<(), TonkWorkerError> {
    let space = space_of(space)?;
    super::repository::invite_space(state, space.repo_key(), time).await;
    Ok(())
}

/// The person's account's authority over `space`, as the persona has it on
/// record, base58.
pub async fn space_authority(state: &AppState, space: &str) -> Result<String, TonkWorkerError> {
    let space = space_of(space)?;
    let tonk = state.read().await;
    revoke_invite::authority_for_space(&tonk, &space).await
}

/// Sign and publish the revocation of a grant on `space` its own worker
/// asked for, `request`. Answers the access service's receipt.
pub async fn revoke_grant(
    state: &AppState,
    space: &str,
    request: serde_json::Value,
) -> Result<serde_json::Value, TonkWorkerError> {
    let space = space_of(space)?;
    let request: RevocationRequest = serde_json::from_value(request)
        .map_err(|error| TonkWorkerError::Router(format!("the revocation request: {error}")))?;
    let tonk = state.read().await;
    let receipt = revoke_invite::revoke_for_space(&tonk, &space, request).await?;
    serde_json::to_value(receipt)
        .map_err(|error| TonkWorkerError::Internal(format!("the revocation receipt: {error}")))
}
