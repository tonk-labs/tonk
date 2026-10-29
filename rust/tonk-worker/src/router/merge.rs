//! Signing in, on a kernel: the provisional persona merges into the
//! permanent persona the person signs in as.
//!
//! The panel's `CreateAccount` or `LogIn` asks the provisional persona to
//! merge into the persona its address names. The embedder runs the passkey
//! ceremony, and creates or opens the account with it, then lends this
//! worker both personas' keys for one call of `lifecycle.merge`. In that
//! call the permanent persona delegates to the representative, which links
//! this device to the account; the embedder says what the ceremony made of
//! the account, and enrolls an account it created at the access service.
//! This worker records it all as a passkey's hand-off to the browser's worker
//! recorded it.

use std::sync::{Mutex, PoisonError};

use dialog_ucan_core::DelegationChain;
use serde::Deserialize;
use tonk_account::customer::Receipt;
use tonk_common::log;
use tonk_kernel_world::application;
use tonk_kernel_world::session;
use tonk_kernel_world::world::sapling::kernel::credential::Persona;
use tonk_schema::AccountDisplayName;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::registration::kind;
use tonk_worker_api::{AccountLinkRequest, PasskeyMetadata, SaveRootRequest};

use super::registration;
use super::{account, customer, identity};
use crate::router::AppState;
use crate::{TonkWorkerError, detach};

/// What the panel asked to sign in to, while the embedder runs the
/// ceremony.
struct Pending {
    address: String,
    name: Option<String>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// Ask the provisional persona to merge into the persona `address` names,
/// for the ceremony `kind`, naming a new account `name`. An error, for the
/// person, when this worker cannot.
pub(crate) fn ask(kind: &str, address: &str, name: Option<String>) -> Result<(), String> {
    match session::persona() {
        Some(Persona::Provisional(persona)) => {
            log!("merge: asking to {kind} as {address}");
            *PENDING.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pending {
                address: address.to_owned(),
                name,
            });
            persona.merge(address);
            Ok(())
        }
        Some(Persona::Permanent(_)) => Err("This profile is signed in already.".to_owned()),
        None => Err("This worker cannot sign anyone in.".to_owned()),
    }
}

/// What the embedder says the passkey ceremony made of the account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ceremony {
    /// `create` for an account the ceremony created, `log-in` for one it
    /// opened.
    kind: String,
    /// The passkey's credential, in hex.
    credential_id: String,
    /// Where the account syncs.
    remote: String,
    /// The account's key for sealed messages, as a DID.
    encryption_key: Option<String>,
    /// When and where a passkey the ceremony created was made.
    passkey: Option<PasskeyMetadata>,
    /// The passkey's custody DID, and the account's secret sealed to it, in
    /// hex, for an account the ceremony created.
    custody: Option<String>,
    sealed_hex: Option<String>,
}

/// The provisional persona merged into the permanent one that delegated
/// `link` to the representative: record the account, and where the panel
/// is now.
pub async fn merged(state: &AppState, link: Vec<u8>) -> Result<(), TonkWorkerError> {
    let pending = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let unavailable = |error| TonkWorkerError::Internal(format!("the embedder: {error:?}"));
    let answer = application::ceremony(&link).await.map_err(unavailable)?;
    let ceremony: Ceremony = serde_json::from_str(&answer.body).map_err(|error| {
        TonkWorkerError::Internal(format!("the ceremony's answer is unreadable: {error}"))
    })?;
    let created = ceremony.kind == kind::CREATE;
    let kind = if created { kind::CREATE } else { kind::LOG_IN };
    let root = DelegationChain::try_from(link.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("the link is no delegation: {error}")))?
        .issuer()
        .clone();
    let delegation_hex = hex::encode(&link);
    let address = pending
        .as_ref()
        .map(|pending| pending.address.clone())
        .unwrap_or_default();

    let tonk = state.read().await;
    identity::persist_root(
        &tonk,
        SaveRootRequest {
            credential_id: ceremony.credential_id.clone(),
            delegation_hex: delegation_hex.clone(),
            passkey: ceremony.passkey.clone(),
            encryption_key: ceremony.encryption_key.clone(),
        },
    )
    .await?;
    account::persist_link(
        &tonk,
        &AccountLinkRequest {
            provider: ceremony.remote.clone(),
            root_did: root.to_string(),
            credential_id: ceremony.credential_id.clone(),
            delegation_hex,
            remote: ceremony.remote.clone(),
            initialize_name: created,
        },
    )
    .await?;
    if created {
        if let Some(name) = pending.and_then(|pending| pending.name) {
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .transaction()
                .assert(AccountDisplayName::new(root.this(), name))
                .commit()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("the display name was not recorded: {error}"))
                })?;
        }
        if let (Some(custody), Some(sealed)) = (&ceremony.custody, &ceremony.sealed_hex) {
            customer::record_custody_cell(
                &tonk,
                custody,
                sealed,
                ceremony.passkey.clone(),
                &ceremony.credential_id,
                Some(&address)
                    .filter(|address| !address.is_empty())
                    .map(String::as_str),
            )
            .await?;
        }
        let answer = application::enroll().await.map_err(unavailable)?;
        let receipt: Receipt = serde_json::from_str(&answer.body).map_err(|error| {
            TonkWorkerError::Internal(format!("the enrollment's receipt is unreadable: {error}"))
        })?;
        customer::enrolled(&tonk, &receipt, address).await?;
    }
    drop(tonk);
    registration::settle(state, kind, Ok(())).await;
    if !created {
        let state = state.clone();
        detach(async move {
            let tonk = state.read().await;
            if let Err(error) = account::finish_link(&tonk).await {
                log!("merge: the account's link did not finish: {error}");
            }
        });
    }
    Ok(())
}

/// The person refused the passkey ceremony `kind` a merge asked for, as the
/// browser names the refusal: `name`.
pub async fn refused(state: &AppState, kind: &str, name: &str) {
    PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let tonk = state.read().await;
    registration::ceremony_refused(&tonk, kind, name).await;
}

/// The sign-in the passkey ceremony `kind` began failed, saying `message`,
/// with the custody service's `code` for a refusal it knows: the panel waits
/// for the emailed link, or says why.
pub async fn failed(state: &AppState, kind: &str, code: Option<&str>, message: &str) {
    PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let kind = if kind == kind::CREATE {
        kind::CREATE
    } else {
        kind::LOG_IN
    };
    log!("merge: the {kind} sign-in failed: {message}");
    registration::settle(state, kind, Err((code, message))).await;
}
