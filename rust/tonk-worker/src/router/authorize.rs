//! Acting as the account, on a kernel: what a passkey-gated command does
//! once the person lets this worker act as their permanent persona.
//!
//! A command that needs the account's key asks the permanent persona to
//! authorize it (`permanent-persona.authorize`), with what it needs, the
//! custody intent the browser's worker asked a page for, as the context.
//! The embedder asks the page for the passkey, opens the account with it,
//! and lends its key for one call of `lifecycle.authorize` with that
//! context. In that call this worker does what the browser's worker did with
//! the key it opened: delegates the account to a waiting device, purges the
//! account, or mints the delegation a page asked for.

use std::sync::{Mutex, PoisonError};

use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use ipld_core::ipld::Ipld;
use tonk_common::log;
use tonk_identity::request::{PURGE_COMMAND, build_device_invocation};
use tonk_invite::home_address_meta;
use tonk_kernel_world::application;
use tonk_kernel_world::session;
use tonk_kernel_world::world::sapling::kernel::credential::{
    Access, Argument, Persona, PersonaAuthority,
};
use tonk_schema::{ceremony, ceremony_state};
use tonk_worker_api::{CustodyIntent, DeviceAuthorization, RootDelegation};
use url::Url;

use super::ClientId;
use super::ceremony::{approvable, delivered, grant_remote, report, report_authorization};
use crate::TonkWorkerError;
use crate::router::AppState;

/// What was asked to be authorized: the ceremony it reports as, and the page
/// that asked.
struct Pending {
    ceremony: String,
    client: Option<ClientId>,
    intent: CustodyIntent,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// How long the account lets this worker's session purge it.
const PURGE_WITHIN_SECONDS: u64 = 300;

/// Ask the permanent persona to authorize what `intent` needs, for the page
/// `client`, reporting as `ceremony`. An error, for the person, when this
/// worker acts for no permanent persona.
pub(crate) fn ask(
    client: Option<&ClientId>,
    ceremony: &str,
    intent: CustodyIntent,
) -> Result<(), String> {
    let Some(Persona::Permanent(persona)) = session::persona() else {
        return Err("no account is signed in on this profile".to_owned());
    };
    let context = serde_json::to_vec(&intent).map_err(|error| error.to_string())?;
    let reason = reason_for(&intent);
    log!("authorize: asking to act as the account {reason}");
    *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pending {
        ceremony: ceremony.to_owned(),
        client: client.cloned(),
        intent,
    });
    persona.authorize(&reason, &context);
    Ok(())
}

/// Why the account is asked for, in words the embedder can show a person.
fn reason_for(intent: &CustodyIntent) -> String {
    match intent {
        CustodyIntent::AuthorizeDevice(authorization) => {
            format!("to connect {} to your account", authorization.name)
        }
        CustodyIntent::PurgeAccount(_) => "to delete your account".to_owned(),
        CustodyIntent::Delegate(_) => "to share a space with a member as its admin".to_owned(),
        CustodyIntent::AddPasskey(_) => "to add a passkey to your account".to_owned(),
        CustodyIntent::Enroll(_) | CustodyIntent::CreateAccount(_) | CustodyIntent::Login(_) => {
            "to act as your account".to_owned()
        }
    }
}

/// A page asked the account to delegate, behind the passkey it already ran:
/// ask the permanent persona for it, as a command would.
pub async fn delegate_asked(subject: String, command: String, audience: String) {
    let intent = CustodyIntent::Delegate(RootDelegation {
        subject,
        command,
        audience,
    });
    if let Err(error) = ask(None, ceremony::DELEGATE, intent) {
        let _ = application::delegated(Err(&error)).await;
    }
}

/// The authorization asked for could not be had: report it as the ceremony
/// that asked.
pub async fn unauthorized(state: &AppState, message: &str) {
    let Some(pending) = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    else {
        return;
    };
    log!(
        "authorize: {} was not authorized: {message}",
        pending.ceremony
    );
    let tonk = state.read().await;
    match &pending.intent {
        CustodyIntent::AuthorizeDevice(authorization) => {
            report_authorization(&tonk, authorization, ceremony_state::FAILED, message).await;
        }
        CustodyIntent::Delegate(_) => {
            drop(tonk);
            let _ = application::delegated(Err(message)).await;
        }
        _ => report(&tonk, &pending.ceremony, ceremony_state::FAILED, message).await,
    }
}

/// The person let this worker act as their permanent persona, lent as
/// `persona`, for what `context`, a custody intent, needs.
pub async fn authorized(
    state: &AppState,
    persona: &PersonaAuthority,
    context: &[u8],
) -> Result<(), TonkWorkerError> {
    let intent: CustodyIntent = serde_json::from_slice(context)
        .map_err(|error| TonkWorkerError::Router(format!("no authorization is {error}")))?;
    let pending = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let client = pending.and_then(|pending| pending.client);
    let done = match intent {
        CustodyIntent::AuthorizeDevice(authorization) => {
            authorize_device(state, persona, &authorization, client.as_ref()).await
        }
        CustodyIntent::PurgeAccount(_) => purge(state, persona, client.as_ref()).await,
        CustodyIntent::Delegate(delegation) => {
            let minted = delegate(persona, &delegation).await;
            let told = match &minted {
                Ok(chain) => application::delegated(Ok(chain)).await,
                Err(error) => application::delegated(Err(error)).await,
            };
            if let Err(error) = told {
                log!("authorize: the delegation was not handed over: {error:?}");
            }
            minted.map(|_| ())
        }
        other => Err(format!("{} is not done on this host", reason_for(&other))),
    };
    done.map_err(TonkWorkerError::Router)
}

/// The account's `meta` entry `key`, its value `value` as DAG-CBOR.
fn meta(key: &str, value: &Ipld) -> Result<Argument, String> {
    Ok(Argument {
        key: key.to_owned(),
        value: serde_ipld_dagcbor::to_vec(value).map_err(|error| error.to_string())?,
    })
}

/// Delegate the account to the waiting device `authorization` names, register
/// the device, and send the page that asked to the callback that delivers
/// the grant, reporting how it went.
async fn authorize_device(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: &DeviceAuthorization,
    client: Option<&ClientId>,
) -> Result<(), String> {
    let outcome = device_grant(state, persona, authorization).await;
    let tonk = state.read().await;
    match &outcome {
        Ok(target) => {
            report_authorization(&tonk, authorization, ceremony_state::DONE, target).await;
            super::navigate::notify_replace(client, target);
        }
        Err(error) => {
            report_authorization(&tonk, authorization, ceremony_state::FAILED, error).await;
        }
    }
    outcome.map(|_| ())
}

/// The account's grant to the device `authorization` names, with where the
/// account syncs in its meta, registered: answers the callback that delivers
/// it.
async fn device_grant(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: &DeviceAuthorization,
) -> Result<String, String> {
    {
        let tonk = state.read().await;
        report_authorization(&tonk, authorization, ceremony_state::WORKING, "").await;
    }
    let root: Did = persona
        .did()
        .parse()
        .map_err(|error| format!("the account is no DID: {error:?}"))?;
    approvable(state, authorization, &root).await?;
    let remote = grant_remote(state).await?;
    let address: Url = remote
        .parse()
        .map_err(|error| format!("the account sync endpoint is not a URL: {error}"))?;
    let meta = home_address_meta(&address)
        .iter()
        .map(|(key, value)| meta(key, value))
        .collect::<Result<Vec<_>, _>>()?;
    let grant = persona
        .delegate(
            authorization.audience.clone(),
            Access {
                subject: None,
                command: "/".to_owned(),
                arguments: Vec::new(),
                meta,
                not_before: None,
                expiration: None,
                reason: reason_for(&CustodyIntent::AuthorizeDevice(authorization.clone())),
            },
        )
        .await
        .map_err(|error| format!("the device grant did not sign: {error:?}"))?;
    delivered(
        state,
        authorization,
        &hex::encode(&grant),
        root.as_str(),
        &remote,
    )
    .await
}

/// Mint the single hop `delegation` asks for, from the account: answered as
/// the base58 of its chain.
async fn delegate(
    persona: &PersonaAuthority,
    delegation: &RootDelegation,
) -> Result<String, String> {
    let chain = persona
        .delegate(
            delegation.audience.clone(),
            Access {
                subject: Some(delegation.subject.clone()),
                command: delegation.command.clone(),
                arguments: Vec::new(),
                meta: Vec::new(),
                not_before: None,
                expiration: None,
                reason: reason_for(&CustodyIntent::Delegate(delegation.clone())),
            },
        )
        .await
        .map_err(|error| format!("the delegation did not sign: {error:?}"))?;
    Ok(bs58::encode(chain).into_string())
}

/// Purge the account, as the browser's worker purged it, but signed by this
/// worker's session through the account's delegation to it of the purge
/// alone, for a few minutes; then send the page that asked home.
async fn purge(
    state: &AppState,
    persona: &PersonaAuthority,
    client: Option<&ClientId>,
) -> Result<(), String> {
    let outcome = purge_inner(state, persona, client).await;
    let tonk = state.read().await;
    match &outcome {
        Ok(()) => {
            report(&tonk, ceremony::DELETE_ACCOUNT, ceremony_state::DONE, "/").await;
            super::navigate::notify_replace(client, "/");
        }
        Err(error) => {
            report(
                &tonk,
                ceremony::DELETE_ACCOUNT,
                ceremony_state::FAILED,
                error,
            )
            .await;
        }
    }
    outcome
}

async fn purge_inner(
    state: &AppState,
    persona: &PersonaAuthority,
    client: Option<&ClientId>,
) -> Result<(), String> {
    let (operator, device) = {
        let tonk = state.read().await;
        report(&tonk, ceremony::DELETE_ACCOUNT, ceremony_state::WORKING, "").await;
        (
            tonk.profile.credential().signer().clone(),
            tonk.profile.did(),
        )
    };
    let root = persona.did();
    let expiration = (super::sync::now_millis() / 1000.0) as u64 + PURGE_WITHIN_SECONDS;
    let link = persona
        .delegate(
            device.to_string(),
            Access {
                subject: Some(root.clone()),
                command: PURGE_COMMAND.map(|segment| format!("/{segment}")).concat(),
                arguments: Vec::new(),
                meta: Vec::new(),
                not_before: None,
                expiration: Some(expiration),
                reason: reason_for(&CustodyIntent::PurgeAccount(Default::default())),
            },
        )
        .await
        .map_err(|error| format!("the purge was not delegated: {error:?}"))?;
    let link = DelegationChain::try_from(link.as_slice())
        .map_err(|error| format!("the purge's delegation: {error}"))?;
    let invocation = build_device_invocation(
        operator,
        &link,
        PURGE_COMMAND.map(str::to_owned).to_vec(),
        Default::default(),
    )
    .await
    .map_err(|error| format!("the purge did not sign: {error:#}"))?;
    super::account_deletion::purged(state, client, &root, &invocation).await
}
