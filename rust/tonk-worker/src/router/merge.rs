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

use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use dialog_repository::{Repository, secrets};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use serde::Deserialize;
use tonk_account::customer::Receipt;
use tonk_common::log;
use tonk_kernel_world::application;
use tonk_kernel_world::credentials;
use tonk_kernel_world::session;
use tonk_kernel_world::world::sapling::kernel::credential::{Custodian, Persona, PersonaAuthority};
use tonk_schema::AccountDisplayName;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::registration::kind;
use tonk_worker_api::{AccountLinkRequest, PasskeyMetadata, SaveRootRequest};

use super::registration;
use super::{account, customer, identity, rotation};
use crate::router::AppState;
use crate::worker::TonkState;
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

/// The provisional persona merged into the permanent one, lent as
/// `permanent`, which delegated `link` to the representative: record the
/// account, carry the provisional persona's spaces over to it, and say
/// where the panel is now.
pub async fn merged(
    state: &AppState,
    link: Vec<u8>,
    provisional: &PersonaAuthority,
    permanent: &PersonaAuthority,
) -> Result<(), TonkWorkerError> {
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
    carry_spaces(&tonk, provisional, permanent, &root).await;
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

/// Bring every space the provisional persona holds under the account, the
/// permanent persona: each space's key is backed up to the permanent persona
/// too, and the space delegates to the account, in place of the
/// provisional persona. Best effort per space: one that fails is logged and
/// left under the provisional persona.
async fn carry_spaces(
    tonk: &TonkState,
    provisional: &PersonaAuthority,
    permanent: &PersonaAuthority,
    account: &Did,
) {
    let previous: Did = match provisional.did().parse() {
        Ok(previous) => previous,
        Err(error) => {
            log!("merge: the provisional persona names no DID: {error:?}");
            return;
        }
    };
    let held =
        match secrets::keys_for(tonk.profile.state(), &tonk.profile.did(), &tonk.profile).await {
            Ok(held) => held,
            Err(error) => {
                log!("merge: the spaces held here were not read: {error}");
                return;
            }
        };
    for (space, backup) in held {
        match carry_space(tonk, permanent, &previous, account, &space, backup).await {
            Ok(()) => log!("merge: {space} is the account's"),
            Err(error) => log!("merge: {space} stays the provisional persona's: {error}"),
        }
    }
}

async fn carry_space(
    tonk: &TonkState,
    permanent: &PersonaAuthority,
    previous: &Did,
    account: &Did,
    space: &Did,
    backup: Vec<u8>,
) -> Result<(), TonkWorkerError> {
    let failed = |what: &str, error: &dyn std::fmt::Debug| {
        TonkWorkerError::Internal(format!("{what}: {error:?}"))
    };
    let key = session::credential()
        .restore(session::backup_to_credential(backup))
        .await
        .map_err(|error| failed("the space's key was not restored", &error))?;
    let resealed = key
        .backup(Custodian::Persona(permanent))
        .await
        .map_err(|error| failed("the space's key was not backed up to the account", &error))?;
    secrets::grant(
        tonk.profile.state(),
        space,
        account,
        resealed.secret,
        &tonk.profile,
    )
    .await
    .map_err(|error| failed("the account's copy of the key was not kept", &error))?;
    let signer = credentials::signer(Rc::new(key))
        .map_err(|error| failed("the space's key is not a signer", &error))?;
    let minter = Repository::from(signer);
    let chain = minter
        .access()
        .claim(&minter)
        .delegate(account.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| failed("the space did not delegate to the account", &error))?
        .into_chain();
    rotation::settle_reissued_space(tonk, account, previous, space, &chain).await
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
