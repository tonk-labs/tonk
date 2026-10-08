//! Leaving a ticket in a space: the delegation chain an audience-open
//! invite grants, kept in the space's memory under the key it was issued
//! to (`ticket/{key did}`), so that a link carrying only the space and
//! the key's seed can redeem it (see `tonk_invite::ticket`).

use dialog_capability::{Fork, Provider};
use dialog_effects::memory::prelude::{
    PublishCellExt as _, ResolveCellExt as _, RetractCellExt as _,
};
use dialog_effects::memory::{MemoryError, Publish, Resolve, Retract};
use dialog_effects::ticket;
use dialog_repository::{PeersEnv, RemoteSite, SiteAddress};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use thiserror::Error;

use crate::peer::{ConnectReplicaError, connect};

/// Why a ticket was not left in the space.
#[derive(Debug, Error)]
pub enum IssueTicketError {
    /// The chain grants no specific space to keep it in.
    #[error("the delegation grants no specific space")]
    NoSubject,
    /// The chain did not encode.
    #[error("the delegation did not encode: {0}")]
    Encode(String),
    /// The space's service could not be reached.
    #[error(transparent)]
    Connect(#[from] ConnectReplicaError),
    /// The service did not keep the ticket.
    #[error("the space did not keep the ticket: {0}")]
    Publish(#[from] MemoryError),
}

/// Keep `chain` in the space it grants, at the service reached at
/// `address`, as the ticket of the key it was issued to (its audience).
///
/// A key is minted for each link, so its cell is new: the write expects
/// it empty, and a ticket already there is never replaced.
///
/// # Errors
///
/// [`IssueTicketError`] when the chain names no space, the service cannot
/// be reached, or it does not keep the ticket.
pub async fn issue<Env>(
    address: SiteAddress,
    chain: &DelegationChain,
    env: &Env,
) -> Result<(), IssueTicketError>
where
    Env: PeersEnv + Provider<Fork<RemoteSite, Publish>>,
{
    let space = chain.subject().ok_or(IssueTicketError::NoSubject)?.clone();
    let content = chain
        .to_bytes()
        .map_err(|error| IssueTicketError::Encode(error.to_string()))?;
    let remote = connect(address, space.clone(), env).await?;
    ticket::writer(&space, chain.audience())
        .publish(content, None)
        .perform(&remote.connection(env))
        .await?;
    Ok(())
}

/// Take back the ticket `space` keeps for `holder`, at the service reached
/// at `address`, so the next claim finds none.
///
/// This hides the grant from whoever has not claimed it yet. It does not
/// revoke it: a chain already claimed keeps proving until the delegation
/// is revoked. Returns whether there was a ticket to take back.
///
/// # Errors
///
/// [`IssueTicketError`] when the service cannot be reached or does not
/// empty the cell.
pub async fn withdraw<Env>(
    address: SiteAddress,
    space: &Did,
    holder: &Did,
    env: &Env,
) -> Result<bool, IssueTicketError>
where
    Env: PeersEnv + Provider<Fork<RemoteSite, Resolve>> + Provider<Fork<RemoteSite, Retract>>,
{
    let remote = connect(address, space.clone(), env).await?;
    let connection = remote.connection(env);
    let Some(edition) = ticket::reader(space, holder)
        .resolve()
        .perform(&connection)
        .await?
    else {
        return Ok(false);
    };
    ticket::remover(space, holder)
        .retract(edition.version)
        .perform(&connection)
        .await?;
    Ok(true)
}
