//! The onboarding account: a real account, custodied locally — the
//! CLI half of the worker's model.
//!
//! A device has an account from first boot. Before it signs in, the
//! account is custodied by a locally generated key rather than a
//! passkey, but it is an account in every other respect: same secret
//! shape, same envelope, same delegations, and it is what unlinked
//! spaces delegate their authority to and seal their seeds for.
//! Signing in is then an **account key rotation** — the shared
//! `tonk_schema::custody::rotate` opens everything the onboarding
//! account custodies and moves it to the signed-in account — not a
//! migration from a placeholder.
//!
//! Same constants and storage discipline as the worker's
//! `onboarding.rs`: custodian key and envelope site are separate so
//! retirement (demoting the custodian to its public half) leaves an
//! envelope that is deliberately unopenable rather than absent.

use anyhow::{Context, Result, bail};
use dialog_credentials::{Credential, Ed25519Signer, Signer};
use dialog_effects::credential::CredentialError;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::{Did, Signer as VarsigSigner};
use tonk_identity::clearance::Recovery;
use tonk_identity::envelope::{AccountSecret, CUSTODIAN_KEK_CONTEXT, Envelope, Kek, KekMethod};

/// Credential site holding the onboarding account's wrapped secret.
const ONBOARDING_ENVELOPE_SITE: &str = "tonk-onboarding-account-v1";

/// Credential key holding the onboarding custodian.
const ONBOARDING_CUSTODIAN_KEY: &str = "tonk-onboarding-custodian-v1";

/// Credential site holding the onboarding account's device grant bytes.
///
/// Operator stores are disjoint: a delegation saved through the
/// install-store operator is out of a site operator's reach. The exact
/// minted bytes live here so every operator can install the SAME grant
/// into its own store — content-addressed, so repeated installs
/// converge where re-minting (fresh nonces) never would.
pub const ONBOARDING_GRANT_SITE: &str = "tonk-onboarding-grant-v1";

/// This device's onboarding account, minting one on first call.
pub async fn account(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<AccountSecret> {
    match read(profile, operator).await? {
        Some(secret) => Ok(secret),
        None => create(profile, operator).await,
    }
}

/// The onboarding account's DID, or `None` before one exists (or after
/// retirement). Reads rather than creates.
pub async fn did(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<Option<Did>> {
    use dialog_varsig::Principal as _;
    let Some(secret) = read_if_openable_in(profile, operator).await? else {
        return Ok(None);
    };
    let signer = secret.signer().await.map_err(|error| {
        anyhow::anyhow!("the onboarding account's signer did not derive: {error}")
    })?;
    Ok(Some(signer.did()))
}

/// The stored account, or `None` when this device has none yet.
///
/// An envelope outliving its custodian is the shape retirement leaves
/// behind — deliberately unopenable, not missing — and reads as an
/// error rather than as absence, so nothing mints a second onboarding
/// account on top of a rotated device.
pub async fn read(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<AccountSecret>> {
    let Some(envelope) = load_site(profile, operator, ONBOARDING_ENVELOPE_SITE).await? else {
        return Ok(None);
    };
    let Some(custodian) = load_custodian(profile, operator).await? else {
        bail!("the onboarding envelope has no custodian; this device already rotated its account");
    };
    open(&custodian, &envelope).await.map(Some)
}

/// [`read`], answering `None` instead of erroring on a retired
/// account — for callers asking "is there anything left to rotate".
pub async fn read_if_openable_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<AccountSecret>> {
    let Some(envelope) = load_site(profile, operator, ONBOARDING_ENVELOPE_SITE).await? else {
        return Ok(None);
    };
    let Some(custodian) = load_custodian(profile, operator).await? else {
        return Ok(None);
    };
    open(&custodian, &envelope).await.map(Some)
}

/// Retire the onboarding account: demote its custodian to the public
/// half, so the envelope can never be opened again on this device.
pub async fn retire(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<()> {
    use dialog_credentials::Ed25519Verifier;
    use dialog_effects::credential::prelude::*;
    use dialog_varsig::Principal as _;

    let Some(custodian) = load_custodian(profile, operator).await? else {
        return Ok(());
    };
    let verifier: Ed25519Verifier =
        custodian.did().to_string().parse().map_err(|error| {
            anyhow::anyhow!("the custodian DID is not an Ed25519 key: {error:?}")
        })?;
    profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .save(Credential::from(verifier))
        .perform(operator)
        .await
        .context("failed to demote the onboarding custodian")
}

/// Mint, wrap, and store a fresh onboarding account.
async fn create(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<AccountSecret> {
    use dialog_effects::credential::prelude::*;

    let secret = AccountSecret::generate()
        .map_err(|error| anyhow::anyhow!("failed to generate the onboarding account: {error}"))?;
    let custodian = Ed25519Signer::generate()
        .await
        .map_err(|error| anyhow::anyhow!("failed to generate the onboarding custodian: {error}"))?;
    let envelope = derive_kek(&custodian)
        .await?
        .seal(&secret, KekMethod::Local)
        .map_err(|error| anyhow::anyhow!("failed to seal the onboarding account: {error}"))?;

    // Custodian first, envelope second: a custodian with no envelope
    // reads as absent, while an envelope with no custodian is the
    // unopenable shape retirement leaves. Neither half alone can be
    // mistaken for a usable account.
    profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .save(Credential::from(custodian))
        .perform(operator)
        .await
        .context("failed to save the onboarding custodian")?;
    profile
        .credential()
        .site(ONBOARDING_ENVELOPE_SITE)
        .save(envelope.encode())
        .perform(operator)
        .await
        .context("failed to save the onboarding envelope")?;

    // The device grant, minted once at the account's birth: subject-open
    // and command-open, the same powerline a passkey account issues at
    // sign-in. Spaces delegate to the ACCOUNT while the device signs —
    // and minting here, not per use, is what keeps it one grant rather
    // than one per space (delegations carry fresh nonces, so re-minting
    // never converges).
    let account_signer = secret
        .signer()
        .await
        .map_err(|error| anyhow::anyhow!("the onboarding signer did not derive: {error}"))?;
    let grant = tonk_account::delegations::mint_account_union(
        &dialog_credentials::Signer::from(account_signer),
        &profile.did(),
    )
    .await
    .map_err(|error| anyhow::anyhow!("failed to mint the onboarding grant: {error}"))?;
    let bytes = grant
        .to_bytes()
        .map_err(|error| anyhow::anyhow!("the onboarding grant does not serialize: {error}"))?;
    profile
        .credential()
        .site(ONBOARDING_GRANT_SITE)
        .save(bytes)
        .perform(operator)
        .await
        .context("failed to persist the onboarding grant")?;
    profile
        .access()
        .save(dialog_ucan::UcanDelegation(grant))
        .perform(operator)
        .await
        .context("failed to save the onboarding grant")?;
    Ok(secret)
}

/// Install the onboarding device grant into `operator`'s own reach,
/// answering the chain. `None` before an onboarding account exists.
pub async fn install_grant(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<dialog_ucan_core::DelegationChain>> {
    let Some(bytes) = load_site(profile, operator, ONBOARDING_GRANT_SITE).await? else {
        return Ok(None);
    };
    let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| anyhow::anyhow!("the stored onboarding grant is malformed: {error}"))?;
    profile
        .access()
        .save(dialog_ucan::UcanDelegation(chain.clone()))
        .perform(operator)
        .await
        .context("failed to install the onboarding grant")?;
    Ok(Some(chain))
}

async fn open(custodian: &Ed25519Signer, envelope: &[u8]) -> Result<AccountSecret> {
    let envelope = Envelope::decode(envelope)
        .map_err(|error| anyhow::anyhow!("the onboarding envelope is malformed: {error}"))?;
    derive_kek(custodian)
        .await?
        .open(&envelope)
        .map_err(|error| anyhow::anyhow!("the onboarding account did not open: {error}"))
}

/// The KEK that wraps the onboarding secret, recomputed from a fresh
/// custodian signature on every use — never written anywhere.
async fn derive_kek(custodian: &Ed25519Signer) -> Result<Kek<Recovery>> {
    let signature = VarsigSigner::sign(custodian, CUSTODIAN_KEK_CONTEXT)
        .await
        .map_err(|error| anyhow::anyhow!("the onboarding custodian did not sign: {error}"))?;
    Ok(Kek::from_custodian(signature.to_bytes().as_ref()))
}

async fn load_site(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    site: &str,
) -> Result<Option<Vec<u8>>> {
    match profile
        .credential()
        .site(site)
        .load::<Vec<u8>>()
        .perform(operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => Ok(None),
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if missing_credential(&error) => Ok(None),
        Err(error) => Err(error).context("failed to load the onboarding envelope"),
    }
}

/// The stored custodian, or `None` when this device has none — a
/// demoted (verifier-only) record also reads as `None`, which is the
/// retired state.
async fn load_custodian(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<Ed25519Signer>> {
    use dialog_effects::credential::prelude::*;

    let credential = match profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .load()
        .perform(operator)
        .await
    {
        Ok(credential) => credential,
        Err(error) if missing_credential(&error) => return Ok(None),
        Err(error) => return Err(error).context("failed to load the onboarding custodian"),
    };
    let Some(signer) = credential.signer() else {
        return Ok(None);
    };
    let Signer::Ed25519(signer) = signer;
    Ok(Some(signer.clone()))
}

fn missing_credential(error: &CredentialError) -> bool {
    matches!(error, CredentialError::NotFound(_))
        || matches!(error, CredentialError::Storage(message) if message.contains("No such file or directory"))
}
