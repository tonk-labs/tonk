//! The keys a guest kernel reaches through handles: `credential` and
//! `session`. Every key stays here: as its seed, or as a signer whose seed
//! nothing reads, as a browser keeps a session's key. A guest kernel signs
//! the bytes it writes, conceals its own messages to a DID, and backs a key
//! up to a custodian whose key is here too, so no key reaches it. What it
//! asks its representative to delegate, the representative's [`Delegates`]
//! answer.
//!
//! A provisional persona merges into a permanent one once the person signs in
//! with a passkey: the embedder opens the permanent persona's key and lends
//! both to the guest kernel for one call of the lifecycle, and the permanent
//! persona's delegation to the representative links it. Once permanent, the
//! embedder opens the persona's key with a passkey to lend it for one call of
//! `lifecycle.authorize`, as the guest kernel asks.

use std::collections::BTreeMap;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use dialog_common::ConditionalSync;
use dialog_credentials::Ed25519Signer;
use dialog_credentials::Ed25519Verifier;
use dialog_credentials::ed25519::Extractable;
use dialog_credentials::key::{ExtractableKey, KeyExport};
use dialog_credentials::secret::{Context, SealedSecret, SecretExtractableDerive};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::eddsa::Ed25519Signature;
use dialog_varsig::{Did, Principal as _};
use ed25519_dalek::Signer as _;
#[cfg(not(target_arch = "wasm32"))]
use futures::future::BoxFuture;
#[cfg(target_arch = "wasm32")]
use futures::future::LocalBoxFuture;
use ipld_core::ipld::Ipld;
use wcmp::{Accessor, Resource};
use zeroize::Zeroizing;

use crate::bindings::sapling::kernel::credential::{
    self, Access, Credential, CredentialBackup, Custodian, Payload as Signed, PermanentPersona,
    Persona, PersonaAuthority, ProvisionalPersona, Recovery, Representative,
};
use crate::bindings::sapling::kernel::session::{self, PersonaSession, Session, SpaceSession};
use crate::bindings::sapling::kernel::types::Error;
use crate::sapling::{KernelHost, Sapling, did_key, trace};

/// The context every key is backed up in. A guest kernel cannot conceal a
/// message in it, nor reveal one, so it opens a backup only by `restore`.
const CUSTODY: &str = "sapling.kernel/custody";

/// A key a handle names.
#[derive(Clone)]
pub enum Key {
    /// A key whose seed is here, and whether the guest kernel knew the seed,
    /// which makes it no custodian.
    Seed {
        seed: Zeroizing<[u8; 32]>,
        imported: bool,
    },
    /// A key held as a signer whose seed nothing reads, as a browser keeps a
    /// session's key in a `CryptoKey` it cannot export. It signs, reveals and
    /// derives, and is never entrusted.
    Sealed(Box<Ed25519Signer>),
}

impl Key {
    fn generated(seed: Zeroizing<[u8; 32]>) -> Self {
        Self::Seed {
            seed,
            imported: false,
        }
    }

    pub(crate) fn did(&self) -> String {
        match self {
            Self::Seed { seed, .. } => did_key(&ed25519_dalek::SigningKey::from_bytes(seed)),
            Self::Sealed(signer) => signer.did().to_string(),
        }
    }

    /// The key, as a signer of Dialog DB.
    pub(crate) async fn signer(&self) -> Result<Ed25519Signer, Error> {
        match self {
            Self::Seed { seed, .. } => Ed25519Signer::import(&**seed)
                .await
                .map_err(|error| Error::Invalid(error.to_string())),
            Self::Sealed(signer) => Ok((**signer).clone()),
        }
    }

    /// Whether the guest kernel knew this key's seed.
    pub(crate) fn imported(&self) -> bool {
        matches!(self, Self::Seed { imported: true, .. })
    }

    async fn sign(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        match self {
            Self::Seed { seed, .. } => Ok(ed25519_dalek::SigningKey::from_bytes(seed)
                .sign(bytes)
                .to_bytes()
                .to_vec()),
            Self::Sealed(signer) => {
                let signature: Ed25519Signature = dialog_varsig::Signer::sign(&**signer, bytes)
                    .await
                    .map_err(|error| Error::Invalid(error.to_string()))?;
                Ok(signature.0.to_vec())
            }
        }
    }
}

/// A delegation a representative answers with, once it comes: `Send`
/// natively, as a store's futures are, and not in the browser, where a
/// JavaScript promise answers it.
#[cfg(not(target_arch = "wasm32"))]
pub type Answer = BoxFuture<'static, Result<Vec<u8>, Error>>;
#[cfg(target_arch = "wasm32")]
pub type Answer = LocalBoxFuture<'static, Result<Vec<u8>, Error>>;

/// What answers `representative.delegate`: the representative's key, where
/// the host holds it, or the host of the person's persona, which it asks.
pub trait Delegator: ConditionalSync {
    fn delegate(&self, audience: String, access: Access) -> Answer;
}

impl<F: Fn(String, Access) -> Answer + ConditionalSync> Delegator for F {
    fn delegate(&self, audience: String, access: Access) -> Answer {
        self(audience, access)
    }
}

/// What answers a representative's delegations, shared by each handle on
/// it: an `Rc` in the browser, where a key held as a `CryptoKey` is not
/// `Send`.
#[cfg(not(target_arch = "wasm32"))]
pub type Delegates = Arc<dyn Delegator>;
#[cfg(target_arch = "wasm32")]
pub type Delegates = Rc<dyn Delegator>;

/// `delegator`, shared.
pub fn delegates(delegator: impl Delegator + 'static) -> Delegates {
    #[cfg(not(target_arch = "wasm32"))]
    let shared = Arc::new(delegator);
    #[cfg(target_arch = "wasm32")]
    let shared = Rc::new(delegator);
    shared
}

/// What a representative handle names: its DID, its key when this host holds
/// it, and what answers its delegations.
#[derive(Clone)]
pub struct Representation {
    pub did: String,
    pub key: Option<Key>,
    pub delegates: Delegates,
}

impl Representation {
    /// A representative whose key this host holds, which signs each
    /// delegation of what it is itself the subject of.
    pub fn held(key: Key) -> Self {
        let signing = key.clone();
        Self {
            did: key.did(),
            key: Some(key),
            delegates: delegates(move |audience: String, access: Access| {
                let key = signing.clone();
                Box::pin(async move { delegation(&key, &audience, &access).await }) as Answer
            }),
        }
    }

    /// A representative whose key this host holds, linked to the persona
    /// `persona` by `link`, the persona's delegation to it: it answers a
    /// delegation to itself of what the persona has with the link, and
    /// signs each delegation of what it is itself the subject of.
    pub fn linked(key: Key, persona: String, link: Vec<u8>) -> Self {
        let signing = key.clone();
        let did = key.did();
        let own = did.clone();
        Self {
            did,
            key: Some(key),
            delegates: delegates(move |audience: String, access: Access| {
                let key = signing.clone();
                let link = link.clone();
                let of_persona = access
                    .subject
                    .as_deref()
                    .is_none_or(|subject| subject == persona);
                let to_itself = audience == own;
                Box::pin(async move {
                    if of_persona && to_itself {
                        return Ok(link);
                    }
                    delegation(&key, &audience, &access).await
                }) as Answer
            }),
        }
    }

    /// A representative this host holds no key of, whose delegations
    /// `delegates` answers, such as by asking the host of the persona.
    pub fn remote(did: String, delegates: Delegates) -> Self {
        Self {
            did,
            key: None,
            delegates,
        }
    }
}

/// Whom a worker acts for, as its session says.
#[derive(Clone)]
pub enum Acting {
    /// The persona itself, for the persona's own worker: provisional, with
    /// its key here.
    Persona(Key),
    /// The persona itself, for the persona's own worker, once it is
    /// permanent: its DID, since this host opens its key only to lend it.
    Permanent(String),
    /// The persona of a space's worker, by its DID.
    Space(String),
}

/// What a session handle names: the key it acts under, what it acts
/// through, and whom it acts for.
pub struct Standing {
    pub credential: Key,
    pub representative: Representation,
    pub acting: Acting,
}

impl Standing {
    /// The standing of a persona's own worker, provisional, that acts under
    /// the key of `seed`, which also represents the persona and is it.
    pub fn new(seed: [u8; 32]) -> Self {
        let key = Key::generated(Zeroizing::new(seed));
        Self {
            credential: key.clone(),
            representative: Representation::held(key.clone()),
            acting: Acting::Persona(key),
        }
    }

    /// The standing of a space's worker that acts under the key of `seed`,
    /// for the persona that key is, whose representative answers its first
    /// delegation with `handed`, and every one after it with none.
    pub fn handed(seed: [u8; 32], handed: Option<Vec<u8>>) -> Self {
        let key = Key::generated(Zeroizing::new(seed));
        let handed = Mutex::new(handed);
        let representative = Representation::remote(
            key.did(),
            delegates(move |_, _| {
                let chain = handed.lock().expect("handed").take();
                Box::pin(async move {
                    chain.ok_or_else(|| Error::Denied("nothing more is delegated".into()))
                }) as Answer
            }),
        );
        Self {
            acting: Acting::Space(key.did()),
            credential: key,
            representative,
        }
    }

    /// The standing of a persona's own worker, provisional, under
    /// `credential`, a key held as a signer, which represents the persona and
    /// is it, until the person makes themselves known.
    pub fn persona(credential: Ed25519Signer) -> Self {
        let key = Key::Sealed(Box::new(credential));
        Self {
            credential: key.clone(),
            representative: Representation::held(key.clone()),
            acting: Acting::Persona(key),
        }
    }

    /// The standing of a persona's own worker under `credential`, a key held
    /// as a signer, which represents the permanent persona `persona` through
    /// `link`, the persona's delegation to it.
    pub fn permanent(credential: Ed25519Signer, persona: String, link: Vec<u8>) -> Self {
        let key = Key::Sealed(Box::new(credential));
        Self {
            credential: key.clone(),
            representative: Representation::linked(key, persona.clone(), link),
            acting: Acting::Permanent(persona),
        }
    }

    /// The standing of a space's worker under `credential`, a key held as a
    /// signer, acting for the persona `persona` through `representative`.
    pub fn space(
        credential: Ed25519Signer,
        representative: Representation,
        persona: String,
    ) -> Self {
        Self {
            credential: Key::Sealed(Box::new(credential)),
            representative,
            acting: Acting::Space(persona),
        }
    }
}

/// A delegation from `key` to `audience` of what `access` asks, of a subject
/// that is `key` itself, or of whatever `key` has authority over.
async fn delegation(key: &Key, audience: &str, access: &Access) -> Result<Vec<u8>, Error> {
    let issuer = key.did();
    if let Some(subject) = &access.subject
        && *subject != issuer
    {
        return Err(Error::Denied(format!(
            "{issuer} delegates only itself, not {subject}"
        )));
    }
    delegated(key, audience, access).await
}

/// A delegation from `key` to `audience` of what `access` asks, of any
/// subject: whether `key` has authority over it is for the chain it extends
/// to show.
async fn delegated(key: &Key, audience: &str, access: &Access) -> Result<Vec<u8>, Error> {
    if !access.arguments.is_empty() {
        return Err(Error::Invalid(
            "a delegation that constrains arguments is not made here yet".into(),
        ));
    }
    let audience: Did = audience
        .parse()
        .map_err(|_| Error::Invalid(format!("{audience} is not a DID")))?;
    let subject = match &access.subject {
        Some(subject) => UcanSubject::Specific(
            subject
                .parse()
                .map_err(|_| Error::Invalid(format!("{subject} is not a DID")))?,
        ),
        None => UcanSubject::Any,
    };
    let command = access
        .command
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let meta = access
        .meta
        .iter()
        .map(|entry| {
            serde_ipld_dagcbor::from_slice::<Ipld>(&entry.value)
                .map(|value| (entry.key.clone(), value))
                .map_err(|_| Error::Invalid(format!("the meta {} is no DAG-CBOR", entry.key)))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let signer = key.signer().await?;
    let mut builder = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(signer))
        .audience(&audience)
        .subject(subject)
        .command(command)
        .meta(meta);
    if let Some(at) = access.not_before {
        builder = builder.not_before(timestamp(at)?);
    }
    if let Some(at) = access.expiration {
        builder = builder.expiration(timestamp(at)?);
    }
    let delegation = builder
        .try_build()
        .await
        .map_err(|error| Error::Invalid(format!("{error:?}")))?;
    DelegationChain::new(delegation)
        .to_bytes()
        .map_err(|error| Error::Invalid(format!("{error:?}")))
}

/// The time `seconds` after the Unix epoch, as a UCAN says it.
fn timestamp(seconds: u64) -> Result<Timestamp, Error> {
    Timestamp::try_from(i128::from(seconds))
        .map_err(|_| Error::Invalid(format!("{seconds} is out of a UCAN's range")))
}

/// What a persona handle names: its key, where this host holds it.
#[derive(Clone)]
pub(crate) struct Held {
    pub(crate) did: String,
    pub(crate) key: Option<Key>,
}

impl KernelHost {
    /// A persona's key, lent to the guest kernel for one call of the
    /// lifecycle: what a handle on it represents.
    pub(crate) fn lend(&mut self, key: Key) -> u32 {
        self.keep_persona(Held {
            did: key.did(),
            key: Some(key),
        })
    }

    /// Take back the key lent as `authority`: the call it was lent for
    /// returned.
    pub(crate) fn withdraw(&mut self, authority: u32) {
        self.personas[authority as usize].key = None;
    }

    /// The persona's own worker merged into the permanent persona that
    /// linked its representative: from now on it acts for that persona,
    /// through the link. Answers the persona and the link, if one was made.
    pub(crate) fn merged(&mut self) -> Option<(String, Vec<u8>)> {
        let (persona, link) = self.linked.take()?;
        for standing in &mut self.sessions {
            if !matches!(standing.acting, Acting::Persona(_) | Acting::Permanent(_)) {
                continue;
            }
            let Some(key) = standing.representative.key.clone() else {
                continue;
            };
            standing.representative = Representation::linked(key, persona.clone(), link.clone());
            standing.acting = Acting::Permanent(persona.clone());
        }
        Some((persona, link))
    }

    /// The permanent persona the persona's own worker acts for, if it acts
    /// for one.
    pub(crate) fn permanent(&self) -> Option<String> {
        self.sessions
            .iter()
            .find_map(|standing| match &standing.acting {
                Acting::Permanent(persona) => Some(persona.clone()),
                _ => None,
            })
    }

    /// Have the persona's own worker act for the permanent persona `persona`
    /// through `link`, its delegation to the representative, or, with none,
    /// for the provisional persona its representative's key is, as the
    /// worker said it now does, such as on signing out. A link that is not
    /// the persona's delegation to the representative is refused.
    pub(crate) fn stand(&mut self, standing: Option<(String, Vec<u8>)>) -> Result<(), String> {
        for held in &mut self.sessions {
            if !matches!(held.acting, Acting::Persona(_) | Acting::Permanent(_)) {
                continue;
            }
            let Some(key) = held.representative.key.clone() else {
                continue;
            };
            match &standing {
                None => {
                    held.representative = Representation::held(key.clone());
                    held.acting = Acting::Persona(key);
                }
                Some((persona, link)) => {
                    let chain = DelegationChain::try_from(link.as_slice())
                        .map_err(|error| format!("the link is no delegation: {error}"))?;
                    if chain.issuer().to_string() != *persona
                        || chain.audience().to_string() != key.did()
                    {
                        return Err(format!(
                            "the link is not {persona}'s delegation to {}",
                            key.did()
                        ));
                    }
                    held.representative =
                        Representation::linked(key, persona.clone(), link.clone());
                    held.acting = Acting::Permanent(persona.clone());
                }
            }
        }
        Ok(())
    }

    fn hold(&mut self, key: Key) -> Resource<Credential> {
        self.keys.push(key);
        Resource::new_own((self.keys.len() - 1) as u32)
    }

    fn key(&self, credential: &Resource<Credential>) -> Key {
        self.keys[credential.rep() as usize].clone()
    }

    fn represent(&mut self, representation: Representation) -> Resource<Representative> {
        self.representatives.push(representation);
        Resource::new_own((self.representatives.len() - 1) as u32)
    }

    fn representation(&self, representative: &Resource<Representative>) -> Representation {
        self.representatives[representative.rep() as usize].clone()
    }

    fn keep_persona(&mut self, held: Held) -> u32 {
        self.personas.push(held);
        (self.personas.len() - 1) as u32
    }

    fn persona_of(&self, rep: u32) -> Held {
        self.personas[rep as usize].clone()
    }

    /// A handle on `standing`, for the boot.
    pub fn session(&mut self, standing: Standing) -> Session {
        let persona = matches!(standing.acting, Acting::Persona(_) | Acting::Permanent(_));
        self.sessions.push(standing);
        let rep = (self.sessions.len() - 1) as u32;
        if persona {
            Session::Persona(Resource::new_own(rep))
        } else {
            Session::Space(Resource::new_own(rep))
        }
    }

    fn standing(&self, rep: u32) -> &Standing {
        &self.sessions[rep as usize]
    }

    /// The DID of `custodian`, and whether a key backed up to it is one the
    /// guest kernel could restore with a seed it knows.
    fn custodian(&self, custodian: &Custodian) -> Result<String, Error> {
        match custodian {
            Custodian::Credential(held) => {
                let key = self.key(held);
                if key.imported() {
                    return Err(Error::Denied(
                        "a key whose seed the guest kernel knows is no custodian".into(),
                    ));
                }
                Ok(key.did())
            }
            Custodian::Representative(representative) => {
                Ok(self.representation(representative).did)
            }
            Custodian::Provisional(persona) => Ok(self.persona_of(persona.rep()).did),
            Custodian::Permanent(persona) => Ok(self.persona_of(persona.rep()).did),
            Custodian::Persona(authority) => Ok(self.persona_of(authority.rep()).did),
        }
    }
}

/// A context Dialog DB can take: its labels are `'static` text, and a guest
/// kernel names a context by its bytes. Each label is kept once. The custody
/// context is refused.
fn context(label: &[u8]) -> Result<Context, Error> {
    let label =
        std::str::from_utf8(label).map_err(|_| Error::Invalid("a context is UTF-8".into()))?;
    if label == CUSTODY {
        return Err(Error::Denied(
            "a guest kernel names no custody context".into(),
        ));
    }
    Ok(custody_free(label))
}

fn custody_free(label: &str) -> Context {
    static LABELS: OnceLock<Mutex<BTreeMap<String, &'static str>>> = OnceLock::new();
    let mut labels = LABELS.get_or_init(Default::default).lock().expect("labels");
    let label = *labels
        .entry(label.to_owned())
        .or_insert_with(|| Box::leak(label.to_owned().into_boxed_str()));
    Context::new(label)
}

async fn signer(seed: &[u8; 32]) -> Result<Ed25519Signer<Extractable>, Error> {
    <Ed25519Signer<Extractable> as ExtractableKey>::import(seed)
        .await
        .map_err(|error| Error::Invalid(error.to_string()))
}

fn recipient(did: &str) -> Result<Ed25519Verifier, Error> {
    did.parse()
        .map_err(|_| Error::Invalid(format!("{did} is not an Ed25519 did:key")))
}

async fn conceal(to: &str, context: Context, message: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(recipient(to)?
        .secret(context)
        .conceal(message)
        .await
        .map_err(|error| Error::Invalid(error.to_string()))?
        .to_bytes())
}

async fn reveal(key: &Key, context: Context, secret: &[u8]) -> Result<Vec<u8>, Error> {
    let sealed =
        SealedSecret::from_bytes(secret).map_err(|error| Error::Invalid(error.to_string()))?;
    let revealed = match key {
        Key::Seed { seed, .. } => signer(seed).await?.secret(context).reveal(&sealed).await,
        Key::Sealed(signer) => signer.secret(context).reveal(&sealed).await,
    };
    revealed.map_err(|_| Error::Denied("it is not concealed to this key, in this context".into()))
}

/// The key `custody` holds, entrusted to `custodian`.
async fn recover(custodian: &Key, custody: &[u8]) -> Result<Key, Error> {
    let seed = Zeroizing::new(reveal(custodian, custody_free(CUSTODY), custody).await?);
    let seed = <[u8; 32]>::try_from(seed.as_slice())
        .map(Zeroizing::new)
        .map_err(|_| Error::Invalid("a custody holds a 32-byte seed".into()))?;
    Ok(Key::generated(seed))
}

async fn seed_of(signer: &Ed25519Signer<Extractable>) -> Result<Zeroizing<[u8; 32]>, Error> {
    match signer.export().await {
        Ok(KeyExport::Extractable(bytes)) => <[u8; 32]>::try_from(bytes.as_slice())
            .map(Zeroizing::new)
            .map_err(|_| Error::Invalid("a key's seed is 32 bytes".into())),
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        Ok(_) => Err(Error::Invalid("the key's seed cannot be read".into())),
        Err(error) => Err(Error::Invalid(error.to_string())),
    }
}

fn fresh_seed() -> Result<Zeroizing<[u8; 32]>, Error> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(seed.as_mut()).map_err(|error| Error::Unavailable(error.to_string()))?;
    Ok(seed)
}

impl credential::Host for KernelHost {}

impl<T: 'static> credential::HostWithStore<T> for Sapling {
    async fn generate(accessor: &Accessor<T, Self>) -> Result<Resource<Credential>, Error> {
        trace("credential.generate");
        let key = Key::generated(fresh_seed()?);
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }

    async fn import(
        accessor: &Accessor<T, Self>,
        seed: Vec<u8>,
    ) -> Result<Resource<Credential>, Error> {
        trace("credential.import");
        let seed = <[u8; 32]>::try_from(seed.as_slice())
            .map(Zeroizing::new)
            .map_err(|_| Error::Invalid("a seed is 32 bytes".into()))?;
        let key = Key::Seed {
            seed,
            imported: true,
        };
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }

    async fn conceal(
        _accessor: &Accessor<T, Self>,
        to: String,
        label: Vec<u8>,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.conceal");
        conceal(&to, context(&label)?, &message).await
    }
}

impl credential::HostCredential for KernelHost {
    fn did(&mut self, credential: Resource<Credential>) -> String {
        self.key(&credential).did()
    }

    fn drop(&mut self, _credential: Resource<Credential>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> credential::HostCredentialWithStore<T> for Sapling {
    async fn sign(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        signed: Signed,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.sign");
        let key = accessor.with(|mut access| access.get().key(&credential));
        let Signed::Bytes(bytes) = signed;
        key.sign(&bytes).await
    }

    async fn reveal(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        label: Vec<u8>,
        secret: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.reveal");
        let key = accessor.with(|mut access| access.get().key(&credential));
        reveal(&key, context(&label)?, &secret).await
    }

    async fn backup(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        custodian: Custodian,
    ) -> Result<CredentialBackup, Error> {
        trace("credential.backup");
        let (key, custodian) = accessor.with(|mut access| {
            let host = access.get();
            Ok::<_, Error>((host.key(&credential), host.custodian(&custodian)?))
        })?;
        let Key::Seed { seed, .. } = &key else {
            return Err(Error::Denied(
                "a key held as a signer is never backed up".into(),
            ));
        };
        let secret = conceal(&custodian, custody_free(CUSTODY), seed.as_slice()).await?;
        Ok(CredentialBackup { custodian, secret })
    }

    async fn restore(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        backup: CredentialBackup,
    ) -> Result<Resource<Credential>, Error> {
        trace("credential.restore");
        let custodian = accessor.with(|mut access| access.get().key(&credential));
        let key = restored(&custodian, &backup).await?;
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }

    async fn derive(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        label: Vec<u8>,
        derivation: Vec<u8>,
    ) -> Result<Resource<Credential>, Error> {
        trace("credential.derive");
        let context = context(&label)?;
        let key = accessor.with(|mut access| access.get().key(&credential));
        let derived = match &key {
            Key::Seed { seed, .. } => {
                SecretExtractableDerive::derive(&signer(seed).await?.secret(context), &derivation)
                    .await
            }
            Key::Sealed(signer) => {
                SecretExtractableDerive::derive(&signer.secret(context), &derivation).await
            }
        }
        .map_err(|error| Error::Invalid(error.to_string()))?;
        let key = Key::generated(seed_of(&derived).await?);
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }
}

/// The key `backup` holds, restored by `custodian`, if it is backed up to
/// it.
async fn restored(custodian: &Key, backup: &CredentialBackup) -> Result<Key, Error> {
    if backup.custodian != custodian.did() {
        return Err(Error::Denied(format!(
            "the backup is for {}, not {}",
            backup.custodian,
            custodian.did()
        )));
    }
    recover(custodian, &backup.secret).await
}

/// `secret`, concealed to `from` in `context`, concealed to `to` instead.
async fn confided(from: &Key, label: &[u8], secret: &[u8], to: &str) -> Result<Vec<u8>, Error> {
    let context = context(label)?;
    let message = Zeroizing::new(reveal(from, context, secret).await?);
    conceal(to, context, &message).await
}

impl credential::HostRepresentative for KernelHost {
    fn did(&mut self, representative: Resource<Representative>) -> String {
        self.representation(&representative).did
    }

    fn drop(&mut self, _representative: Resource<Representative>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> credential::HostRepresentativeWithStore<T> for Sapling {
    async fn delegate(
        accessor: &Accessor<T, Self>,
        representative: Resource<Representative>,
        audience: String,
        access: Access,
    ) -> Result<Vec<u8>, Error> {
        trace("representative.delegate");
        let delegates = accessor.with(|mut access| {
            Delegates::clone(&access.get().representation(&representative).delegates)
        });
        delegates.delegate(audience, access).await
    }

    async fn confide(
        accessor: &Accessor<T, Self>,
        representative: Resource<Representative>,
        label: Vec<u8>,
        secret: Vec<u8>,
        to: Custodian,
    ) -> Result<Vec<u8>, Error> {
        trace("representative.confide");
        let (from, to) = accessor.with(|mut access| {
            let host = access.get();
            Ok::<_, Error>((
                host.representation(&representative).key,
                host.custodian(&to)?,
            ))
        })?;
        let from = from.ok_or_else(|| {
            Error::Unavailable("this host holds no key of the representative".into())
        })?;
        confided(&from, &label, &secret, &to).await
    }

    async fn restore(
        accessor: &Accessor<T, Self>,
        representative: Resource<Representative>,
        backup: CredentialBackup,
    ) -> Result<Resource<Credential>, Error> {
        trace("representative.restore");
        let custodian =
            accessor.with(|mut access| access.get().representation(&representative).key);
        let custodian = custodian.ok_or_else(|| {
            Error::Unavailable("this host holds no key of the representative".into())
        })?;
        let key = restored(&custodian, &backup).await?;
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }
}

impl credential::HostProvisionalPersona for KernelHost {
    fn did(&mut self, persona: Resource<ProvisionalPersona>) -> String {
        self.persona_of(persona.rep()).did
    }

    fn merge(&mut self, _persona: Resource<ProvisionalPersona>, id: String) {
        let taken = self
            .merges
            .as_ref()
            .is_some_and(|merges| merges.unbounded_send(id).is_ok());
        if !taken {
            crate::platform::log("kernel", "a merge was asked for, and no one takes it", true);
        }
    }

    fn drop(&mut self, _persona: Resource<ProvisionalPersona>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl credential::HostPermanentPersona for KernelHost {
    fn did(&mut self, persona: Resource<PermanentPersona>) -> String {
        self.persona_of(persona.rep()).did
    }

    fn authorize(
        &mut self,
        _persona: Resource<PermanentPersona>,
        reason: String,
        context: Vec<u8>,
    ) {
        let taken = self
            .authorizations
            .as_ref()
            .is_some_and(|authorizations| authorizations.unbounded_send((reason, context)).is_ok());
        if !taken {
            crate::platform::log(
                "kernel",
                "an authorization was asked for, and no one takes it",
                true,
            );
        }
    }

    fn drop(&mut self, _persona: Resource<PermanentPersona>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl credential::HostPersonaAuthority for KernelHost {
    fn did(&mut self, authority: Resource<PersonaAuthority>) -> String {
        self.persona_of(authority.rep()).did
    }

    fn drop(&mut self, _authority: Resource<PersonaAuthority>) -> wcmp::Result<()> {
        Ok(())
    }
}

/// The key a persona authority lends, which the host holds for as long as
/// the lifecycle call it was lent for.
fn lent<T: 'static>(
    accessor: &Accessor<T, Sapling>,
    authority: &Resource<PersonaAuthority>,
) -> Result<Key, Error> {
    accessor
        .with(|mut access| access.get().persona_of(authority.rep()).key)
        .ok_or_else(|| Error::Unavailable("the persona's key is not open".into()))
}

impl<T: 'static> credential::HostPersonaAuthorityWithStore<T> for Sapling {
    async fn delegate(
        accessor: &Accessor<T, Self>,
        authority: Resource<PersonaAuthority>,
        audience: String,
        access: Access,
    ) -> Result<Vec<u8>, Error> {
        trace("persona-authority.delegate");
        let key = lent(accessor, &authority)?;
        let chain = delegated(&key, &audience, &access).await?;
        // A delegation of everything the persona has, to the representative
        // of the persona's own worker, links that representative.
        if access.subject.is_none() {
            accessor.with(|mut access| {
                let host = access.get();
                let represents = host.sessions.iter().any(|standing| {
                    matches!(standing.acting, Acting::Persona(_) | Acting::Permanent(_))
                        && standing.representative.did == audience
                });
                if represents {
                    host.linked = Some((key.did(), chain.clone()));
                }
            });
        }
        Ok(chain)
    }

    async fn confide(
        accessor: &Accessor<T, Self>,
        authority: Resource<PersonaAuthority>,
        label: Vec<u8>,
        secret: Vec<u8>,
        to: Custodian,
    ) -> Result<Vec<u8>, Error> {
        trace("persona-authority.confide");
        let from = lent(accessor, &authority)?;
        let to = accessor.with(|mut access| access.get().custodian(&to))?;
        confided(&from, &label, &secret, &to).await
    }

    async fn restore(
        accessor: &Accessor<T, Self>,
        authority: Resource<PersonaAuthority>,
        backup: CredentialBackup,
    ) -> Result<Resource<Credential>, Error> {
        trace("persona-authority.restore");
        let custodian = lent(accessor, &authority)?;
        let key = restored(&custodian, &backup).await?;
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }

    async fn add_recovery(
        _accessor: &Accessor<T, Self>,
        _authority: Resource<PersonaAuthority>,
        _recovery: Recovery,
    ) -> Result<CredentialBackup, Error> {
        Err(Error::Unavailable(
            "this host sets up no recovery yet".into(),
        ))
    }

    async fn unlink(
        _accessor: &Accessor<T, Self>,
        _authority: Resource<PersonaAuthority>,
        _representative: String,
    ) -> Result<(), Error> {
        Err(Error::Unavailable(
            "this host unlinks no representative yet".into(),
        ))
    }
}

impl session::Host for KernelHost {}

impl session::HostPersonaSession for KernelHost {
    fn credential(&mut self, session: Resource<PersonaSession>) -> Resource<Credential> {
        let key = self.standing(session.rep()).credential.clone();
        self.hold(key)
    }

    fn representative(&mut self, session: Resource<PersonaSession>) -> Resource<Representative> {
        let representation = self.standing(session.rep()).representative.clone();
        self.represent(representation)
    }

    fn persona(&mut self, session: Resource<PersonaSession>) -> Persona {
        match self.standing(session.rep()).acting.clone() {
            Acting::Persona(key) => {
                let rep = self.keep_persona(Held {
                    did: key.did(),
                    key: Some(key),
                });
                Persona::Provisional(Resource::new_own(rep))
            }
            Acting::Permanent(did) => {
                let rep = self.keep_persona(Held { did, key: None });
                Persona::Permanent(Resource::new_own(rep))
            }
            Acting::Space(_) => unreachable!("a persona session acts for the persona itself"),
        }
    }

    fn drop(&mut self, _session: Resource<PersonaSession>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl session::HostSpaceSession for KernelHost {
    fn credential(&mut self, session: Resource<SpaceSession>) -> Resource<Credential> {
        let key = self.standing(session.rep()).credential.clone();
        self.hold(key)
    }

    fn representative(&mut self, session: Resource<SpaceSession>) -> Resource<Representative> {
        let representation = self.standing(session.rep()).representative.clone();
        self.represent(representation)
    }

    fn persona(&mut self, session: Resource<SpaceSession>) -> String {
        match &self.standing(session.rep()).acting {
            Acting::Space(did) | Acting::Permanent(did) => did.clone(),
            Acting::Persona(key) => key.did(),
        }
    }

    fn drop(&mut self, _session: Resource<SpaceSession>) -> wcmp::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [5; 32];

    async fn sealed() -> Key {
        Key::Sealed(Box::new(Ed25519Signer::import(&SEED).await.unwrap()))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_signs_with_a_sealed_key_as_with_its_seed() {
        let seed = Key::generated(Zeroizing::new(SEED));
        let sealed = sealed().await;
        assert_eq!(sealed.did(), seed.did());
        let signature = sealed.sign(b"payload").await.unwrap();
        let verifier = ed25519_dalek::SigningKey::from_bytes(&SEED).verifying_key();
        let signature = ed25519_dalek::Signature::from_slice(&signature).unwrap();
        assert!(verifier.verify_strict(b"payload", &signature).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_reveals_with_a_sealed_key() {
        let sealed = sealed().await;
        let context = custody_free("test/reveal");
        let secret = conceal(&sealed.did(), context, b"message").await.unwrap();
        assert_eq!(reveal(&sealed, context, &secret).await.unwrap(), b"message");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_restores_a_key_backed_up_to_a_sealed_key() {
        let sealed = sealed().await;
        let secret = conceal(&sealed.did(), custody_free(CUSTODY), &[6; 32])
            .await
            .unwrap();
        let backup = CredentialBackup {
            custodian: sealed.did(),
            secret,
        };
        let recovered = restored(&sealed, &backup).await.unwrap();
        assert_eq!(
            recovered.did(),
            Key::generated(Zeroizing::new([6; 32])).did()
        );
    }

    fn access(subject: Option<String>) -> Access {
        Access {
            subject,
            command: "/".to_owned(),
            arguments: Vec::new(),
            meta: Vec::new(),
            not_before: None,
            expiration: None,
            reason: "a test".to_owned(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_signs_what_a_delegation_says_of_itself_and_when_it_holds() {
        let persona = Key::generated(Zeroizing::new([1; 32]));
        let audience = Key::generated(Zeroizing::new([2; 32])).did();
        let address = Ipld::String("https://tonk.test/ucan/".to_owned());
        let asked = Access {
            meta: vec![crate::bindings::sapling::kernel::credential::Argument {
                key: "home.address".to_owned(),
                value: serde_ipld_dagcbor::to_vec(&address).unwrap(),
            }],
            not_before: Some(1_000),
            expiration: Some(2_000),
            ..access(None)
        };
        let chain = delegated(&persona, &audience, &asked).await.unwrap();
        let chain = DelegationChain::try_from(chain.as_slice()).unwrap();
        assert_eq!(chain.meta("home.address"), Some(&address));
        let delegation = chain.proofs().next().unwrap();
        assert_eq!(
            delegation.expiration().map(|at| at.to_unix()),
            Some(2_000),
            "the delegation holds until its expiration"
        );
        assert_eq!(delegation.not_before().map(|at| at.to_unix()), Some(1_000));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_has_a_lent_persona_delegate_what_it_is_not_and_a_key_only_itself() {
        let persona = Key::generated(Zeroizing::new([1; 32]));
        let audience = Key::generated(Zeroizing::new([2; 32])).did();
        let space = Key::generated(Zeroizing::new([3; 32])).did();
        assert!(
            delegated(&persona, &audience, &access(Some(space.clone())))
                .await
                .is_ok(),
            "a persona delegates a space it has authority over"
        );
        assert!(matches!(
            delegation(&persona, &audience, &access(Some(space))).await,
            Err(Error::Denied(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn it_acts_for_the_persona_whose_link_it_is_given_and_no_other() {
        let mut host = KernelHost::default();
        host.sessions.push(Standing::new([4; 32]));
        let representative = host.sessions[0].representative.did.clone();
        let account = Key::generated(Zeroizing::new([5; 32]));
        let link = delegated(&account, &representative, &access(None))
            .await
            .unwrap();
        host.stand(Some((account.did(), link.clone()))).unwrap();
        assert_eq!(host.permanent(), Some(account.did()));

        let stranger = Key::generated(Zeroizing::new([6; 32])).did();
        assert!(
            host.stand(Some((stranger, link))).is_err(),
            "a link is not another persona's"
        );
        host.stand(None).unwrap();
        assert_eq!(host.permanent(), None, "signed out, it acts for no account");
    }
}
