//! Tickets: the delegation chain an audience-open grant hands out, kept
//! in the space's memory under the key it was issued to
//! (`ticket/{key did}`), so that a link carrying only the space and the
//! key's seed can redeem it (see `tonk_invite::ticket`).
//!
//! Whoever can write the space's memory leaves a ticket there ([`issue`])
//! or takes it back ([`withdraw`]). The key's ticket is fetched with
//! [`claim`]: a `/use/get/ticket/claim` invocation whose subject is the
//! key (the ticket's holder) and whose `space` argument names the space
//! that keeps it. The key invokes it itself, needing no proof, or anyone
//! it delegated that command to invokes it with the chain: a device
//! claiming through its account's powerline, say. The access service
//! verifies the invocation as any other and reads the cell named by its
//! subject, so a claim reaches its subject's ticket and no other.
//!
//! A ticket is not secret: it delegates to the key, and is worth nothing
//! to anyone who cannot sign as it or for it. The claim's authority is
//! there so the service answers each holder for its own cell, not to
//! guard the bytes.
//!
//! Taking a ticket back hides it from the next claim. It does not revoke
//! the delegation: a chain already fetched keeps proving until it is
//! revoked.

use std::collections::{BTreeMap, HashMap};

use dialog_capability::access::AuthorizeError;
use dialog_capability::{Fork, Provider};
use dialog_effects::memory::prelude::CellScope;
use dialog_effects::memory::{MemoryError, Publish, Resolve, Retract};
use dialog_repository::{PeersEnv, RemoteSite, SiteAddress};
use dialog_ucan_core::issuer::Issuer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{
    Container, DelegationChain, Invocation, InvocationBuilder, InvocationChain,
};
use dialog_varsig::{AnySignature, Did};
use thiserror::Error;
use url::Url;

use crate::peer::{ConnectReplicaError, connect};

/// The memory space tickets are kept in.
pub const SPACE: &str = "ticket";

/// The command a holder fetches its ticket with: `/use/get/ticket/claim`,
/// invoked on the space that keeps it.
pub const CLAIM: [&str; 4] = ["use", "get", "ticket", "claim"];

/// The cell `space` keeps `holder`'s ticket in.
pub fn cell(space: &Did, holder: &Did) -> CellScope {
    CellScope::new(space.clone().into(), SPACE, holder.to_string())
}

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
    cell(&space, chain.audience())
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
    let ticket = cell(space, holder);
    let Some(edition) = ticket.resolve().perform(&connection).await? else {
        return Ok(false);
    };
    ticket.retract(edition.version).perform(&connection).await?;
    Ok(true)
}

/// Why a claim did not answer a ticket.
#[derive(Debug, Error)]
pub enum ClaimError {
    /// The claim could not be signed or encoded.
    #[error("the claim could not be signed: {0}")]
    Sign(String),
    /// The service could not be reached, or answered something that is
    /// neither a ticket nor a refusal.
    #[error("the ticket could not be fetched: {0}")]
    Transport(String),
    /// The service refused the claim.
    #[error("the claim was refused: {0}")]
    Refused(AuthorizeError),
}

/// The `/use/get/ticket/claim` argument naming the space that keeps the
/// ticket.
pub const SPACE_ARGUMENT: &str = "space";

/// Fetch the ticket `space` keeps for `holder` from the access service at
/// `remote` (its `/ucan/` endpoint).
///
/// `issuer` signs the claim. It is the holder itself, with `proof`
/// `None`, or a principal the holder delegated `/use/get/ticket/claim` to
/// (or any command above it), with `proof` the chain from the holder to
/// it.
///
/// Answers the ticket's bytes (a delegation container), or `None` when
/// the space keeps no ticket for `holder`. The ticket carries its own
/// signatures, so nothing the service answers is taken on trust.
///
/// # Errors
///
/// [`ClaimError`] when the claim cannot be signed, the service cannot be
/// reached, or it refuses the claim.
pub async fn claim<I>(
    remote: &Url,
    issuer: I,
    holder: &Did,
    proof: Option<&DelegationChain>,
    space: &Did,
) -> Result<Option<Vec<u8>>, ClaimError>
where
    I: Issuer<AnySignature> + 'static,
{
    let arguments = BTreeMap::from([(
        SPACE_ARGUMENT.to_owned(),
        Promised::String(space.to_string()),
    )]);
    let (proofs, delegations) = match proof {
        Some(chain) => (chain.proof_cids().to_vec(), chain.export().collect()),
        None => (vec![], HashMap::new()),
    };
    let invocation = InvocationBuilder::new()
        .issuer(issuer)
        .audience(holder)
        .subject(holder)
        .command(CLAIM.iter().map(|segment| (*segment).to_string()).collect())
        .arguments(arguments)
        .proofs(proofs)
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .map_err(|error| ClaimError::Sign(format!("{error:?}")))?;
    let body = InvocationChain::new(invocation, delegations)
        .to_bytes()
        .map_err(|error| ClaimError::Sign(error.to_string()))?;

    let response = reqwest::Client::new()
        .post(remote.clone())
        .header("content-type", "application/cbor")
        .body(body)
        .send()
        .await
        .map_err(|error| ClaimError::Transport(error.to_string()))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ClaimError::Transport(error.to_string()))?;
    match status {
        200..300 => Ok(Some(bytes.to_vec())),
        404 => Ok(None),
        _ => Err(match serde_json::from_slice::<AuthorizeError>(&bytes) {
            Ok(refusal) => ClaimError::Refused(refusal),
            Err(_) => ClaimError::Transport(format!(
                "the service answered {status}: {}",
                String::from_utf8_lossy(&bytes)
            )),
        }),
    }
}

/// What a claim asks for: the holder whose ticket, kept by which space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The space that keeps the ticket: the claim's `space` argument.
    pub space: Did,
    /// The key the ticket is kept for: the claim's subject.
    pub holder: Did,
}

impl Claim {
    /// Read what a claim asks for from its subject and arguments, or
    /// `None` when it names no space. Says nothing about whether the
    /// claim is authorized: the service verifies the invocation first.
    pub fn read(subject: &Did, arguments: &BTreeMap<String, Promised>) -> Option<Self> {
        let Some(Promised::String(space)) = arguments.get(SPACE_ARGUMENT) else {
            return None;
        };
        Some(Self {
            space: space.parse().ok()?,
            holder: subject.clone(),
        })
    }

    /// The object key the ticket is stored at, relative to the bucket:
    /// `{space}/ticket/{holder}`.
    pub fn object_key(&self) -> String {
        format!("{}/{SPACE}/{}", self.space, self.holder)
    }
}

/// Whether `container` holds a `/use/get/ticket/claim` invocation. Says
/// nothing about whether it is a valid one.
pub fn is_claim(container: &[u8]) -> bool {
    first_invocation(container)
        .is_some_and(|invocation| invocation.command().0.iter().map(String::as_str).eq(CLAIM))
}

/// What a verified claim chain asks for, or a refusal when it names no
/// space.
///
/// # Errors
///
/// [`AuthorizeError::Malformed`] when the claim names no space.
pub fn claimed(chain: &InvocationChain<AnySignature>) -> Result<Claim, AuthorizeError> {
    Claim::read(chain.subject(), chain.arguments()).ok_or_else(|| AuthorizeError::Malformed {
        detail: format!("a ticket claim names its space as a '{SPACE_ARGUMENT}' DID argument"),
    })
}

fn first_invocation(container: &[u8]) -> Option<Invocation<AnySignature>> {
    let container = Container::from_bytes(container).ok()?;
    let token = container.into_tokens().into_iter().next()?;
    serde_ipld_dagcbor::from_slice(&token).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A claim reads the ticket of its subject, in the space it names.
    #[dialog_common::test]
    async fn it_reads_a_claim_as_its_subjects_ticket_in_the_named_space() {
        let holder = Ed25519Signer::import(&[3u8; 32]).await.unwrap().did();
        let space = Ed25519Signer::import(&[4u8; 32]).await.unwrap().did();
        let arguments = BTreeMap::from([(
            SPACE_ARGUMENT.to_owned(),
            Promised::String(space.to_string()),
        )]);
        let claim = Claim::read(&holder, &arguments).unwrap();
        assert_eq!(claim.space, space);
        assert_eq!(claim.holder, holder);
        assert_eq!(claim.object_key(), format!("{space}/ticket/{holder}"));
        assert_eq!(Claim::read(&holder, &BTreeMap::new()), None);
    }
}
