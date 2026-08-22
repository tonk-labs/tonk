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

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::credential::CredentialError;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use tonk_identity::envelope::{AccountSecret, Envelope, Kek, KekMethod};
use zeroize::Zeroizing;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Credential site holding the onboarding account's wrapped secret.
///
/// Versioned so a format change is a new site rather than an ambiguous
/// re-read of bytes in the old shape.
const ONBOARDING_ENVELOPE_SITE: &str = "tonk-onboarding-account-v1";

/// Credential site holding the KEK that opens [`ONBOARDING_ENVELOPE_SITE`].
///
/// Separate from the envelope on purpose: accreditation destroys the KEK
/// and leaves the envelope unopenable, which is what makes "the
/// onboarding custodian can no longer reach the account" a fact about
/// storage rather than a promise about code paths.
const ONBOARDING_KEK_SITE: &str = "tonk-onboarding-kek-v1";

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
    let Some(kek) = load(state, ONBOARDING_KEK_SITE).await? else {
        // The envelope outliving its KEK is the shape accreditation
        // leaves behind, and it is deliberately unopenable.
        return Ok(None);
    };
    let envelope = Envelope::decode(&envelope).map_err(|error| {
        TonkWorkerError::Internal(format!("the onboarding envelope is malformed: {error}"))
    })?;
    let kek: [u8; 32] = kek.as_slice().try_into().map_err(|_| {
        TonkWorkerError::Internal(format!("the onboarding KEK is {} bytes, not 32", kek.len()))
    })?;
    Kek::from_bytes(Zeroizing::new(kek))
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
    let mut kek_bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(kek_bytes.as_mut())
        .map_err(|error| TonkWorkerError::Internal(format!("no entropy for a KEK: {error}")))?;
    let kek = Kek::from_bytes(Zeroizing::new(*kek_bytes));
    let envelope = kek
        .seal(&secret, KekMethod::Local)
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;

    // Envelope first, KEK second: a KEK with no envelope reads as
    // absent, while an envelope with no KEK is the unopenable shape
    // accreditation leaves. Neither half alone can be mistaken for a
    // usable account.
    save(state, ONBOARDING_ENVELOPE_SITE, envelope.encode()).await?;
    save(state, ONBOARDING_KEK_SITE, kek_bytes.to_vec()).await?;
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
    Ok(chain)
}

/// Destroy the onboarding custody.
///
/// Called at the END of accreditation, once every space and invite has
/// been re-issued under the new account. The KEK goes first: with it
/// gone the envelope cannot be opened, so a failure between the two
/// leaves an account nobody can reach rather than one an interrupted
/// rotation could still act as.
pub(crate) async fn destroy(state: &TonkState) -> Result<(), TonkWorkerError> {
    save(state, ONBOARDING_KEK_SITE, Vec::new()).await?;
    save(state, ONBOARDING_ENVELOPE_SITE, Vec::new()).await
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
