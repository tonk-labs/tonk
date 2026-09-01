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

use dialog_artifacts::Entity;
use dialog_credentials::secret::SealedSecret;
use dialog_credentials::{Credential, Ed25519Signer, Signer};
use dialog_effects::credential::CredentialError;
use dialog_effects::credential::prelude::*;
use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
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
/// agreement key reveals the KEK that opens [`ONBOARDING_ENVELOPE_SITE`].
///
/// A key, not a site, because `.key()` stores a `CryptoKeyPair` handle
/// that WebCrypto generates **non-extractable** by default. The KEK is
/// on disk only sealed to this keypair ([`ONBOARDING_KEK_SITE`]), so no
/// bytes on disk can open the envelope without it. That is what makes
/// this a stand-in for a passkey rather than a password sitting next to
/// the thing it locks.
///
/// Separate from the envelope on purpose: accreditation destroys the
/// custodian and leaves the envelope unopenable, which makes "the
/// onboarding custodian can no longer reach the account" a fact about
/// storage rather than a promise about code paths.
const ONBOARDING_CUSTODIAN_KEY: &str = "tonk-onboarding-custodian-v1";

/// Credential site holding the KEK that opens [`ONBOARDING_ENVELOPE_SITE`],
/// sealed to the custodian ([`KekMethod::Custodian`]).
///
/// Bytes on disk, but bytes only the custodian's agreement key turns
/// back into a KEK, so the envelope is exactly as unopenable without
/// the custodian as when the KEK was derived from its signature. What
/// the seal buys is reproducibility: an X25519 agreement gives the same
/// answer on every platform, where a signature does not (Safari
/// randomizes Ed25519). Absent for an envelope written under the legacy
/// [`KekMethod::Local`], which `read` reseals on its first open.
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
/// and leaves the bytes in place for diagnosis. The one exception is a
/// legacy envelope on a platform that cannot reproduce its KEK at all
/// (see [`Opened::Unrecoverable`]).
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
    let sealed_kek = load(state, ONBOARDING_KEK_SITE).await?;
    // Only a legacy envelope's fate depends on how the platform signs,
    // so only that shape pays for the probe.
    let signing = match envelope.method {
        KekMethod::Local => probe_signing(&custodian).await?,
        _ => Signing::Deterministic,
    };
    match open_stored(&custodian, &envelope, sealed_kek.as_deref(), signing).await? {
        Opened::Current(secret) => Ok(Some(secret)),
        Opened::Legacy(secret) => {
            // Converge on the sealed KEK, so this device stops depending
            // on how its platform signs. Best effort: the account opened,
            // and a failed reseal costs one more legacy open next boot.
            if let Err(error) = wrap(state, &custodian, &secret).await {
                log!("onboarding envelope reseal skipped: {error}");
            }
            Ok(Some(secret))
        }
        Opened::Unrecoverable => {
            log!(
                "the onboarding envelope was sealed under a signature this platform never repeats; \
                 minting a replacement"
            );
            Ok(None)
        }
    }
}

/// What opening the stored envelope yielded.
enum Opened {
    /// Opened under the KEK sealed to the custodian: the current shape.
    Current(AccountSecret),
    /// Opened under a signature-derived KEK ([`KekMethod::Local`]): the
    /// caller reseals it.
    Legacy(AccountSecret),
    /// A signature-derived KEK on a platform whose signatures never
    /// repeat. The envelope could never have opened here, so nothing was
    /// ever delegated to its account, and replacing it loses nothing;
    /// keeping it would keep the device locked out for good.
    Unrecoverable,
}

/// Open `envelope` with `custodian`, by whichever method it names.
async fn open_stored(
    custodian: &Ed25519Signer,
    envelope: &Envelope<Recovery>,
    sealed_kek: Option<&[u8]>,
    signing: Signing,
) -> Result<Opened, TonkWorkerError> {
    match envelope.method {
        KekMethod::Custodian => {
            let Some(sealed) = sealed_kek else {
                return Err(TonkWorkerError::Internal(
                    "the onboarding envelope names a sealed KEK that is not stored".into(),
                ));
            };
            let sealed = SealedSecret::from_bytes(sealed).map_err(|error| {
                TonkWorkerError::Internal(format!("the onboarding KEK is malformed: {error}"))
            })?;
            Kek::<Recovery>::from_custodian_sealed(custodian, &sealed)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("the onboarding KEK did not reveal: {error}"))
                })?
                .open(envelope)
                .map(Opened::Current)
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "the onboarding account did not open: {error}"
                    ))
                })
        }
        KekMethod::Local => match derive_kek(custodian).await?.open(envelope) {
            Ok(secret) => Ok(Opened::Legacy(secret)),
            Err(_) if signing == Signing::Randomized => Ok(Opened::Unrecoverable),
            Err(error) => Err(TonkWorkerError::Internal(format!(
                "the onboarding account did not open: {error}"
            ))),
        },
        KekMethod::Passkey | KekMethod::Phrase => Err(TonkWorkerError::Internal(format!(
            "the onboarding envelope names {:?}, which is not a local custody method",
            envelope.method
        ))),
    }
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

    // Custodian first, envelope last: a custodian with no envelope reads
    // as absent, while an envelope with no custodian is the unopenable
    // shape accreditation leaves. Neither half alone can be mistaken for
    // a usable account.
    save_custodian(state, custodian.clone()).await?;
    wrap(state, &custodian, &secret).await?;
    Ok(secret)
}

/// Wrap `secret` under a fresh KEK sealed to `custodian`, and store the
/// sealed KEK and the envelope. Replaces whatever envelope was there.
async fn wrap(
    state: &TonkState,
    custodian: &Ed25519Signer,
    secret: &AccountSecret,
) -> Result<(), TonkWorkerError> {
    let (kek, sealed) = Kek::<Recovery>::seal_to_custodian(custodian)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;
    let envelope = kek
        .seal(secret, KekMethod::Custodian)
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;
    // KEK first, envelope second: until the envelope names the sealed
    // KEK, a legacy one being replaced still opens the old way.
    save(state, ONBOARDING_KEK_SITE, sealed.to_bytes()).await?;
    save(state, ONBOARDING_ENVELOPE_SITE, envelope.encode()).await
}

/// Ensure the onboarding account has granted this device a powerline,
/// minting and saving one if it has not.
///
/// Mirrors the grant a passkey account makes at sign-in: subject-open
/// and command-open, so anything the account can prove, the device can
/// prove. That symmetry is what lets a space delegate to the ACCOUNT
/// while the device is what actually signs.
///
/// Convergent rather than idempotent: minting is NOT repeatable — every
/// delegation carries a fresh nonce, so a second mint would be a second
/// grant with its own entity and its own device row. An existing grant
/// is therefore proven from the retained facts and reused, and only a
/// device that cannot prove one mints.
pub(crate) async fn grant_device(state: &TonkState) -> Result<DelegationChain, TonkWorkerError> {
    use dialog_varsig::Principal as _;
    let secret = account(state).await?;
    let account_signer = secret
        .signer()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?;
    if let Some(chain) = existing_grant(state, &account_signer.did()).await {
        return Ok(chain);
    }
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
    // Describe BEFORE saving. `retain` returns only the entities it
    // newly wrote, skipping any certificate the tree already holds, and
    // the entity is derived from the digest the blob store reports
    // rather than computed on the side. Saving first would retain the
    // chain, leaving nothing for `retain` to return and no entity to
    // hang the description on.
    if let Err(error) = describe_device_link(state, &chain, device_title()).await {
        log!("describe device link: {error}");
    }
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

/// The powerline this profile already holds from `account`, rebuilt from
/// the retained facts, or `None` when nothing proves.
///
/// Every failure reads as "no grant": proving is an optimisation over
/// minting, and minting is always safe.
async fn existing_grant(state: &TonkState, account: &Did) -> Option<DelegationChain> {
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .ok()?;
    let scope = Scope {
        subject: UcanSubject::Specific(account.clone()),
        command: Command::parse("/").expect("the root command always parses"),
        parameters: Parameters::default(),
    };
    let proof = branch
        .handle()
        .delegations()
        .prove(state.profile.did(), scope)
        .perform(&state.operator)
        .await
        .ok()?;
    let mut certificates = proof.proofs.into_iter();
    let mut chain = DelegationChain::new(certificates.next()?.0);
    for certificate in certificates {
        chain = chain.push(certificate.0).ok()?;
    }
    Some(chain)
}

/// This device's label, from the worker's own navigator.
///
/// The service worker has `WorkerNavigator` rather than `window`, and no
/// `platform` or touch-point count, so the label is coarser than the
/// page's — browser and OS families still come out of the user agent.
pub(crate) fn device_title() -> String {
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
/// subject onto it. These are the fields it does not carry — a label and
/// a creation time, so a device list renders without asking an account
/// service.
///
/// The link's entity is inferred from those facts: the retained
/// certificates whose `dialog.ucan/audience` is the chain's audience and
/// whose subject is the powerline wildcard, since a device IS its
/// powerline. Inference rather than re-deriving the blob hash on the
/// side keeps this reading the same record the list reads, works for a
/// chain retained long before it is described (saving a grant into the
/// access store retains it as a side effect), and heals historical
/// duplicate grants by describing each of them.
///
/// An existing row wins. `created_at` is history, so re-describing an
/// already-described link changes nothing rather than asserting a
/// conflicting claim.
///
/// The caller decides what a failure means: onboarding treats it as
/// best-effort (the grant is already saved and usable, so a missing
/// description costs a row's label, not access), while an approving
/// page registering another device wants to know the row did not land.
pub(crate) async fn describe_device_link(
    state: &TonkState,
    chain: &DelegationChain,
    title: String,
) -> Result<(), String> {
    use dialog_query::{Output as _, Query, Term};

    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| format!("open profile branch: {error}"))?;
    branch
        .handle()
        .delegations()
        .retain(UcanDelegation(chain.clone()))
        .perform(&state.operator)
        .await
        .map_err(|error| format!("retain: {error}"))?;
    let audience = chain.audience().to_string();
    let entities = link_entities(state, branch.handle(), &audience).await?;
    if entities.is_empty() {
        return Err(format!("no retained powerline names {audience}"));
    }
    let at = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut transaction = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction();
    let mut asserting = false;
    for entity in entities {
        let existing: Vec<tonk_schema::DeviceLink> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::DeviceLink> {
                this: Term::from(entity.clone()),
                created_at: Term::var("created_at"),
                title: Term::var("title"),
                reason: Term::var("reason"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .map_err(|error| format!("query the link row: {error}"))?;
        if !existing.is_empty() {
            continue;
        }
        transaction = transaction.assert(tonk_schema::DeviceLink::new(entity, title.clone(), at));
        asserting = true;
    }
    if !asserting {
        return Ok(());
    }
    transaction
        .commit()
        .perform(&state.operator)
        .await
        .map(|_| ())
        .map_err(|error| format!("commit: {error}"))
}

/// Entities of every retained powerline addressed to `audience`.
///
/// Both facts are dialog's own decomposition, written when the chain was
/// retained: `dialog.ucan/audience` names the device, and a subject of
/// [`ANY_SUBJECT`] is what makes the certificate a powerline rather than
/// a grant scoped to one space.
///
/// [`ANY_SUBJECT`]: dialog_capability::ANY_SUBJECT
async fn link_entities(
    state: &TonkState,
    branch: &dialog_repository::Branch,
    audience: &str,
) -> Result<Vec<Entity>, String> {
    use dialog_artifacts::{ArtifactSelector, Value};
    use futures_util::StreamExt as _;

    let selector = ArtifactSelector::new()
        .the(
            dialog_repository::DELEGATION_AUDIENCE
                .parse()
                .map_err(|error| format!("audience attribute: {error:?}"))?,
        )
        .is(Value::String(audience.into()));
    let facts = branch
        .claims()
        .select(selector)
        .perform(&state.operator)
        .await
        .map_err(|error| format!("select link facts: {error}"))?
        .collect::<Vec<_>>()
        .await;
    let mut entities = Vec::new();
    for fact in facts.into_iter().flatten() {
        let bytes = fact
            .of_bytes()
            .map_err(|error| format!("read a link fact's entity: {error}"))?;
        let entity: Entity = String::from_utf8_lossy(&bytes)
            .parse()
            .map_err(|error| format!("parse a link fact's entity: {error:?}"))?;
        if is_powerline(state, branch, &entity).await? {
            entities.push(entity);
        }
    }
    Ok(entities)
}

/// Whether the retained certificate at `entity` is subject-open.
async fn is_powerline(
    state: &TonkState,
    branch: &dialog_repository::Branch,
    entity: &Entity,
) -> Result<bool, String> {
    use dialog_artifacts::{ArtifactSelector, Value};
    use futures_util::StreamExt as _;

    let selector = ArtifactSelector::new()
        .the(
            dialog_repository::DELEGATION_SUBJECT
                .parse()
                .map_err(|error| format!("subject attribute: {error:?}"))?,
        )
        .of(entity.clone());
    let facts = branch
        .claims()
        .select(selector)
        .perform(&state.operator)
        .await
        .map_err(|error| format!("select the link's subject: {error}"))?
        .collect::<Vec<_>>()
        .await;
    for fact in facts.into_iter().flatten() {
        if let Ok(Value::String(subject)) = fact.value() {
            return Ok(subject == dialog_capability::ANY_SUBJECT);
        }
    }
    Ok(false)
}

/// The recovery-clearance KEK this custodian derives, via a signature
/// over [`CUSTODIAN_KEK_CONTEXT`]: the legacy [`KekMethod::Local`].
///
/// Recovery clearance because this key wraps the account secret itself:
/// the onboarding custodian is the pre-passkey stand-in at the top of
/// the hierarchy, not something derived from what it protects.
///
/// Only opens envelopes written before the KEK was sealed to the
/// custodian instead. It reproduces the KEK where signing is
/// deterministic (RFC 8032: native, Chrome) and never where it is not
/// (Safari's WebCrypto), which is why nothing writes this method anymore.
async fn derive_kek(custodian: &Ed25519Signer) -> Result<Kek<Recovery>, TonkWorkerError> {
    let signature = VarsigSigner::sign(custodian, CUSTODIAN_KEK_CONTEXT)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("the onboarding custodian did not sign: {error}"))
        })?;
    Ok(Kek::from_custodian(signature.to_bytes().as_ref()))
}

/// Whether this platform's Ed25519 signatures repeat for one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signing {
    /// RFC 8032: the same key and message always give the same bytes.
    Deterministic,
    /// A fresh nonce per signature (Safari's WebCrypto). Still valid
    /// signatures; just never the same twice.
    Randomized,
}

/// Sign the legacy context twice and compare. Two matching signatures
/// do not prove determinism in general, but a randomized signer has a
/// negligible chance of repeating, so a match is as good as proof.
async fn probe_signing(custodian: &Ed25519Signer) -> Result<Signing, TonkWorkerError> {
    let mut signatures = Vec::with_capacity(2);
    for _ in 0..2 {
        let signature = VarsigSigner::sign(custodian, CUSTODIAN_KEK_CONTEXT)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("the onboarding custodian did not sign: {error}"))
            })?;
        signatures.push(signature.to_bytes());
    }
    Ok(if signatures[0] == signatures[1] {
        Signing::Deterministic
    } else {
        Signing::Randomized
    })
}

/// The stored custodian, or `None` when this device has none.
/// Retire the onboarding account: demote its custodian to the public
/// half, so the envelope can never be opened again on this device.
///
/// The envelope stays. An envelope with no custodian is what `read`
/// reports as "already accredited", which is exactly the state that must
/// never be mistaken for "no onboarding account yet" — that would mint a
/// second onboarding account on top of an accredited device. There is no
/// retract for keys in the credential API, so demotion overwrites the
/// record with a verifier.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn retire(state: &TonkState) -> Result<(), TonkWorkerError> {
    use dialog_credentials::Ed25519Verifier;
    use dialog_varsig::Principal as _;

    let Some(custodian) = load_custodian(state).await? else {
        return Ok(());
    };
    let verifier: Ed25519Verifier = custodian.did().to_string().parse().map_err(|error| {
        TonkWorkerError::Internal(format!(
            "the custodian DID is not an Ed25519 key: {error:?}"
        ))
    })?;
    state
        .profile
        .did()
        .credential()
        .key(ONBOARDING_CUSTODIAN_KEY)
        .save(Credential::from(verifier))
        .perform(&state.operator)
        .await
        .map_err(|error: CredentialError| {
            TonkWorkerError::Internal(format!(
                "failed to demote the onboarding custodian: {error}"
            ))
        })
}

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

        // The pre-root state is where `grant_device` runs for real: with a
        // root persisted, its grant is a second powerline to this profile
        // and would be described as well.
        let tonk = crate::router::tests::test_state_without_root().await;

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
        assert_eq!(rows[0].reason, tonk_schema::device_link_reason());
        assert!(!rows[0].title.0.is_empty(), "a device carries a label");
        assert!(rows[0].created_at.0 > 0, "a real timestamp");
    }

    /// Granting again reuses the grant already retained. Minting is not
    /// repeatable — a fresh nonce means a fresh delegation — so without
    /// the proof check every space creation would add another grant and
    /// another device row for the same device.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_grants_the_device_once() {
        use dialog_query::{Output as _, Query, Term};

        // The pre-root state is where `grant_device` runs for real: with a
        // root persisted, its grant is a second powerline to this profile
        // and would be described as well.
        let tonk = crate::router::tests::test_state_without_root().await;

        let first = grant_device(&tonk).await.expect("the grant mints");
        let second = grant_device(&tonk).await.expect("the grant re-proves");
        assert_eq!(
            first.proof_cids(),
            second.proof_cids(),
            "a second grant is the first one, proven rather than re-minted"
        );

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
        assert_eq!(rows.len(), 1, "one device, one row");
    }

    /// The legacy method rests on Ed25519 signatures being deterministic
    /// (RFC 8032): the KEK is never stored, so a second signature over
    /// the same context has to reproduce it exactly. True natively and in
    /// Chrome; Safari is why it is legacy.
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

    /// The current shape: a KEK sealed to the custodian opens the
    /// envelope on every read, however the platform signs.
    #[dialog_common::test]
    async fn it_opens_an_envelope_under_a_kek_sealed_to_its_custodian() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let (kek, sealed) = Kek::<Recovery>::seal_to_custodian(&custodian)
            .await
            .unwrap();
        let envelope = kek
            .seal(&AccountSecret::generate().unwrap(), KekMethod::Custodian)
            .unwrap();

        for signing in [Signing::Deterministic, Signing::Randomized] {
            assert!(
                matches!(
                    open_stored(&custodian, &envelope, Some(&sealed.to_bytes()), signing).await,
                    Ok(Opened::Current(_))
                ),
                "a sealed KEK opens regardless of signing ({signing:?})"
            );
        }
        assert!(
            open_stored(&custodian, &envelope, None, Signing::Deterministic)
                .await
                .is_err(),
            "the sealed KEK is required, not optional"
        );
    }

    /// An envelope from before the sealed KEK still opens where signing
    /// is deterministic, and is reported as legacy so `read` reseals it.
    #[dialog_common::test]
    async fn a_legacy_envelope_opens_where_signing_is_deterministic() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let envelope = derive_kek(&custodian)
            .await
            .unwrap()
            .seal(&AccountSecret::generate().unwrap(), KekMethod::Local)
            .unwrap();

        assert!(matches!(
            open_stored(&custodian, &envelope, None, Signing::Deterministic).await,
            Ok(Opened::Legacy(_))
        ));
    }

    /// A legacy envelope that does not open is corruption where signing
    /// is deterministic, and replaceable only where it is not: there the
    /// envelope never opened, so nothing depends on its account.
    #[dialog_common::test]
    async fn an_unopenable_legacy_envelope_is_replaced_only_where_signing_is_randomized() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let envelope = Kek::<Recovery>::from_custodian(b"a signature this custodian never made")
            .seal(&AccountSecret::generate().unwrap(), KekMethod::Local)
            .unwrap();

        assert!(
            open_stored(&custodian, &envelope, None, Signing::Deterministic)
                .await
                .is_err(),
            "where signing repeats, a legacy envelope that does not open is corrupt"
        );
        assert!(matches!(
            open_stored(&custodian, &envelope, None, Signing::Randomized).await,
            Ok(Opened::Unrecoverable)
        ));
    }

    /// Native signing is RFC 8032 deterministic; the probe must say so,
    /// or every legacy envelope would read as replaceable.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[dialog_common::test]
    async fn it_finds_native_signing_deterministic() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        assert_eq!(
            probe_signing(&custodian).await.unwrap(),
            Signing::Deterministic
        );
    }

    /// The account minted on first call is the one read back after.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_reads_back_the_account_it_minted() {
        use dialog_varsig::Principal as _;

        let tonk = crate::router::tests::test_state_without_root().await;
        let minted = account(&tonk).await.expect("the account mints");
        let read_back = read(&tonk)
            .await
            .expect("the stored account reads")
            .expect("an account is stored");
        assert_eq!(
            minted.signer().await.unwrap().did(),
            read_back.signer().await.unwrap().did(),
        );
        let stored = load(&tonk, ONBOARDING_ENVELOPE_SITE)
            .await
            .unwrap()
            .expect("an envelope is stored");
        assert_eq!(
            Envelope::<Recovery>::decode(&stored).unwrap().method,
            KekMethod::Custodian
        );
    }

    /// A device that onboarded under the signature-derived KEK keeps its
    /// account, and its next boot no longer depends on how it signs.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn a_legacy_onboarding_account_is_resealed_on_open() {
        use dialog_varsig::Principal as _;

        let tonk = crate::router::tests::test_state_without_root().await;
        let custodian = Ed25519Signer::generate().await.unwrap();
        let secret = AccountSecret::generate().unwrap();
        let envelope = derive_kek(&custodian)
            .await
            .unwrap()
            .seal(&secret, KekMethod::Local)
            .unwrap();
        save_custodian(&tonk, custodian).await.unwrap();
        save(&tonk, ONBOARDING_ENVELOPE_SITE, envelope.encode())
            .await
            .unwrap();
        let expected = secret.signer().await.unwrap().did();

        let opened = account(&tonk).await.expect("the legacy envelope opens");
        assert_eq!(opened.signer().await.unwrap().did(), expected);

        let stored = load(&tonk, ONBOARDING_ENVELOPE_SITE)
            .await
            .unwrap()
            .expect("an envelope is stored");
        assert_eq!(
            Envelope::<Recovery>::decode(&stored).unwrap().method,
            KekMethod::Custodian,
            "the first open reseals under the sealed KEK"
        );
        assert!(load(&tonk, ONBOARDING_KEK_SITE).await.unwrap().is_some());

        let again = account(&tonk).await.expect("the resealed envelope opens");
        assert_eq!(again.signer().await.unwrap().did(), expected);
    }
}
