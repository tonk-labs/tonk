//! The onboarding account: a real account, custodied locally.
//!
//! A device has an account from first boot. Before the user enrolls a
//! passkey it is custodied by a locally generated key rather than by
//! WebAuthn, but it is an account in every other respect: same secret
//! shape, same envelope, same delegations, and it is what spaces and
//! invites delegate their authority to.
//!
//! That uniformity is the point. Enrolling a passkey is not a migration
//! from a placeholder into the real thing, it is an **account key
//! rotation** — an operation needed anyway for a compromised passkey or
//! a lost device. See `plan/onboarding-accreditation.md`.
//!
//! # Why the secret rotates at accreditation
//!
//! Re-wrapping the *same* secret under a passkey would leave whoever
//! held the onboarding custody key holding the account forever, so a
//! compromise before accreditation would reach everything the account
//! acquired after it. Rotating bounds the blast radius to what existed
//! before: an attacker controls the pre-passkey spaces, not the account.

use dialog_credentials::{Credential, Ed25519Signer, Signer};
use dialog_effects::credential::CredentialError;
use dialog_effects::credential::prelude::*;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Signer as VarsigSigner};
use tonk_identity::clearance::Recovery;
use tonk_identity::envelope::{AccountSecret, CUSTODIAN_KEK_CONTEXT, Envelope, Kek, KekMethod};

use tonk_common::log;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Credential site holding the onboarding account's wrapped secret.
///
/// Versioned so a format change is a new site rather than an ambiguous
/// re-read of bytes in the old shape.
const ONBOARDING_ENVELOPE_SITE: &str = "tonk-onboarding-account-v1";

/// Credential key holding the onboarding custodian: the keypair whose
/// signature derives the KEK that opens [`ONBOARDING_ENVELOPE_SITE`].
///
/// A key, not a site, because `.key()` stores a `CryptoKeyPair` handle
/// that WebCrypto generates **non-extractable** by default. The KEK
/// itself is never written anywhere: it is recomputed from a fresh
/// signature on every boot, so no bytes on disk can open the envelope.
/// That is what makes this a stand-in for a passkey rather than a
/// password sitting next to the thing it locks.
///
/// Separate from the envelope on purpose: accreditation destroys the
/// custodian and leaves the envelope unopenable, which makes "the
/// onboarding custodian can no longer reach the account" a fact about
/// storage rather than a promise about code paths.
const ONBOARDING_CUSTODIAN_KEY: &str = "tonk-onboarding-custodian-v1";

/// This device's onboarding account, minting one on first call.
///
/// Idempotent: the secret is written once and read back on every later
/// boot, so the account DID — and every delegation addressed to it — is
/// stable for as long as the device is un-accredited.
pub(crate) async fn account(state: &TonkState) -> Result<AccountSecret, TonkWorkerError> {
    match read(state).await? {
        Some(secret) => Ok(secret),
        None => create(state).await,
    }
}

/// The onboarding account's signing identity, or `None` before one
/// exists. Reads rather than creates, so a caller asking "is there an
/// onboarding account" does not bring one into being by asking.
pub(crate) async fn signer(state: &TonkState) -> Result<Option<Ed25519Signer>, TonkWorkerError> {
    let Some(secret) = read(state).await? else {
        return Ok(None);
    };
    secret
        .signer()
        .await
        .map(Some)
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))
}

/// The onboarding account's DID, or `None` before one exists.
pub(crate) async fn did(state: &TonkState) -> Result<Option<Did>, TonkWorkerError> {
    use dialog_varsig::Principal as _;
    Ok(signer(state).await?.map(|signer| signer.did()))
}

/// Read and unwrap the stored account, or `None` when this device has
/// none yet.
///
/// A record that exists but cannot be opened is NOT treated as absent:
/// minting a replacement would orphan every space and invite delegated
/// to the account the old bytes name, so corruption surfaces as an error
/// and leaves the bytes in place for diagnosis.
async fn read(state: &TonkState) -> Result<Option<AccountSecret>, TonkWorkerError> {
    let Some(envelope) = load(state, ONBOARDING_ENVELOPE_SITE).await? else {
        return Ok(None);
    };
    let Some(custodian) = load_custodian(state).await? else {
        // An envelope outliving its custodian is the shape
        // accreditation leaves behind. Reporting it as absent would
        // send `account()` off to mint a second onboarding account on
        // top of an accredited device, so it is an error: the envelope
        // is deliberately unopenable, not missing.
        return Err(TonkWorkerError::Internal(
            "the onboarding envelope has no custodian; this device is already accredited".into(),
        ));
    };
    let envelope = Envelope::decode(&envelope).map_err(|error| {
        TonkWorkerError::Internal(format!("the onboarding envelope is malformed: {error}"))
    })?;
    derive_kek(&custodian)
        .await?
        .open(&envelope)
        .map(Some)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("the onboarding account did not open: {error}"))
        })
}

/// Mint, wrap, and store a fresh onboarding account.
async fn create(state: &TonkState) -> Result<AccountSecret, TonkWorkerError> {
    let secret =
        AccountSecret::generate().map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;

    // The default `generate` is what we want: on wasm it produces a
    // non-extractable WebCrypto keypair, and the extractable variant is
    // an explicit opt-in we deliberately do not take.
    let custodian = Ed25519Signer::generate().await.map_err(|error| {
        TonkWorkerError::Internal(format!(
            "failed to generate the onboarding custodian: {error}"
        ))
    })?;
    let envelope = derive_kek(&custodian)
        .await?
        .seal(&secret, KekMethod::Local)
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;

    // Custodian first, envelope second: a custodian with no envelope
    // reads as absent, while an envelope with no custodian is the
    // unopenable shape accreditation leaves. Neither half alone can be
    // mistaken for a usable account.
    save_custodian(state, custodian).await?;
    save(state, ONBOARDING_ENVELOPE_SITE, envelope.encode()).await?;
    Ok(secret)
}

/// Ensure the onboarding account has granted this device a powerline,
/// minting and saving one if it has not.
///
/// Mirrors the grant a passkey account makes at sign-in: subject-open
/// and command-open, so anything the account can prove, the device can
/// prove. That symmetry is what lets a space delegate to the ACCOUNT
/// while the device is what actually signs.
///
/// Idempotent by construction — the grant is derived from two stable
/// keys, so re-minting produces an equivalent delegation rather than a
/// conflicting one.
pub(crate) async fn grant_device(state: &TonkState) -> Result<DelegationChain, TonkWorkerError> {
    let secret = account(state).await?;
    let account_signer = secret
        .signer()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;
    // `mint_account_union` is named for its original direction
    // (`profile -> account`) but is generic over both ends: the first
    // argument signs, the second receives. Here that is
    // `account -> profile`, the powerline.
    let chain = tonk_account::delegations::mint_account_union(
        &Signer::from(account_signer),
        &state.profile.did(),
    )
    .await
    .map_err(|error| {
        TonkWorkerError::Internal(format!("failed to mint the onboarding grant: {error}"))
    })?;
    state
        .profile
        .access()
        .save(UcanDelegation(chain.clone()))
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save the onboarding grant: {error}"))
        })?;
    describe_device_link(state, &chain).await;
    Ok(chain)
}

/// This device's label, from the worker's own navigator.
///
/// The service worker has `WorkerNavigator` rather than `window`, and no
/// `platform` or touch-point count, so the label is coarser than the
/// page's — browser and OS families still come out of the user agent.
fn device_title() -> String {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast as _;
        let agent = js_sys::global()
            .dyn_into::<web_sys::WorkerGlobalScope>()
            .ok()
            .and_then(|scope| scope.navigator().user_agent().ok())
            .unwrap_or_default();
        tonk_common::device_label::from_navigator(&agent, "", 0)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        tonk_common::device_label::from_navigator("", "", 0)
    }
}

/// Describe a device link as facts on the delegation's own entity.
///
/// Retaining the chain is what creates that entity: dialog stores each
/// certificate under its blob hash and decomposes issuer, audience, and
/// subject onto it, then hands the entities back. These are the fields
/// it does not carry — a label and a creation time, so a device list
/// renders without asking the account service.
///
/// Best effort: the grant is already saved and usable by this point, so
/// a missing description costs a row's label, not access.
async fn describe_device_link(state: &TonkState, chain: &DelegationChain) {
    let branch = match state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("describe device link: open profile branch: {error}");
            return;
        }
    };
    let entities = match branch
        .handle()
        .delegations()
        .retain(UcanDelegation(chain.clone()))
        .perform(&state.operator)
        .await
    {
        Ok(entities) => entities,
        Err(error) => {
            log!("describe device link: retain: {error}");
            return;
        }
    };
    let at = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let title = device_title();
    // The chain's certificates each get an entity; the link itself is
    // the last one, the hop that names this device as the audience.
    let Some(entity) = entities.last() else {
        return;
    };
    let transaction = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(tonk_schema::DeviceLink::new(entity.clone(), title, at));
    if let Err(error) = transaction.commit().perform(&state.operator).await {
        log!("describe device link: commit: {error}");
    }
}

/// The recovery-clearance KEK this custodian derives, via a signature
/// over [`CUSTODIAN_KEK_CONTEXT`].
///
/// Recovery clearance because this key wraps the account secret itself:
/// the onboarding custodian is the pre-passkey stand-in at the top of
/// the hierarchy, not something derived from what it protects.
///
/// Ed25519 signatures are deterministic, so the same custodian yields
/// the same KEK on every call without it ever being written down.
async fn derive_kek(custodian: &Ed25519Signer) -> Result<Kek<Recovery>, TonkWorkerError> {
    let signature = VarsigSigner::sign(custodian, CUSTODIAN_KEK_CONTEXT)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("the onboarding custodian did not sign: {error}"))
        })?;
    Ok(Kek::from_custodian(signature.to_bytes().as_ref()))
}

/// The stored custodian, or `None` when this device has none.
async fn load_custodian(state: &TonkState) -> Result<Option<Ed25519Signer>, TonkWorkerError> {
    let credential = match state
        .profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .load()
        .perform(&state.operator)
        .await
    {
        Ok(credential) => credential,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load the onboarding custodian: {error}"
            )));
        }
    };
    // A demoted record holds no signer: `destroy` overwrites the
    // custodian with its own public half, so the private key is gone
    // and this device is accredited. `None` reads as "no custodian",
    // which is exactly right; the public half is kept only so the state
    // stays distinguishable from a device that never onboarded.
    let Some(signer) = credential.signer() else {
        return Ok(None);
    };
    // `Signer` gains arms only when `dialog-credentials` is built with
    // another algorithm, which this crate never enables, so ed25519 is
    // exhaustive here and a wrong-algorithm arm would be dead code.
    let Signer::Ed25519(signer) = signer;
    Ok(Some(signer.clone()))
}

async fn save_custodian(
    state: &TonkState,
    custodian: Ed25519Signer,
) -> Result<(), TonkWorkerError> {
    state
        .profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .save(Credential::from(custodian))
        .perform(&state.operator)
        .await
        .map_err(|error: CredentialError| {
            TonkWorkerError::Internal(format!("failed to save the onboarding custodian: {error}"))
        })
}

async fn load(state: &TonkState, site: &str) -> Result<Option<Vec<u8>>, TonkWorkerError> {
    match state
        .profile
        .credential()
        .site(site)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => Ok(None),
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if crate::credential::is_missing(&error) => Ok(None),
        Err(error) => Err(TonkWorkerError::Internal(format!(
            "failed to load {site}: {error}"
        ))),
    }
}

async fn save(state: &TonkState, site: &str, bytes: Vec<u8>) -> Result<(), TonkWorkerError> {
    state
        .profile
        .credential()
        .site(site)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error: CredentialError| {
            TonkWorkerError::Internal(format!("failed to save {site}: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Granting a device describes the link as facts, so a device list
    /// renders without asking the account service.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_describes_the_device_link() {
        use dialog_query::{Output as _, Query, Term};

        let tonk = crate::router::tests::test_state_without_account().await;

        grant_device(&tonk).await.expect("the grant mints");

        let branch = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("profile branch opens");
        let rows: Vec<tonk_schema::DeviceLink> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::DeviceLink> {
                this: Term::var("this"),
                created_at: Term::var("created_at"),
                title: Term::var("title"),
                reason: Term::var("reason"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("device-link query runs");

        assert_eq!(rows.len(), 1, "exactly one device link described");
        assert_eq!(rows[0].reason.0, tonk_schema::DEVICE_LINK);
        assert!(!rows[0].title.0.is_empty(), "a device carries a label");
        assert!(rows[0].created_at.0 > 0, "a real timestamp");
    }

    /// The whole custodian design rests on Ed25519 signatures being
    /// deterministic (RFC 8032): the KEK is never stored, so a second
    /// signature over the same context has to reproduce it exactly or
    /// the account becomes unopenable on the next boot.
    #[dialog_common::test]
    async fn it_derives_the_same_kek_from_one_custodian() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let secret = AccountSecret::generate().unwrap();

        let envelope = derive_kek(&custodian)
            .await
            .unwrap()
            .seal(&secret, KekMethod::Local)
            .unwrap();

        // A separate signing call, as a later boot would make.
        assert!(
            derive_kek(&custodian)
                .await
                .unwrap()
                .open(&envelope)
                .is_ok(),
            "a custodian must reopen its own envelope across calls"
        );
    }

    #[dialog_common::test]
    async fn it_cannot_open_another_custodians_envelope() {
        let envelope = derive_kek(&Ed25519Signer::generate().await.unwrap())
            .await
            .unwrap()
            .seal(&AccountSecret::generate().unwrap(), KekMethod::Local)
            .unwrap();

        let stranger = Ed25519Signer::generate().await.unwrap();
        assert!(
            derive_kek(&stranger)
                .await
                .unwrap()
                .open(&envelope)
                .is_err()
        );
    }
}
