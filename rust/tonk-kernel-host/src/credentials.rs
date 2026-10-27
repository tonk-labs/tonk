//! The keys a guest kernel reaches through handles: `credential`, `session`
//! and `authentication`. Every key stays here: as its seed, or as a signer
//! whose seed nothing reads, as a browser keeps a session's key. A guest
//! kernel signs the bytes it writes, conceals its own messages to a DID, and
//! entrusts a key to a custodian whose key is here too, so no key reaches it.
//! What it claims, its host's [`Claims`] answer.
//!
//! Not yet bound: signing a person in, which a browser does with a passkey.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use dialog_common::ConditionalSync;
use dialog_credentials::Ed25519Signer;
use dialog_credentials::Ed25519Verifier;
use dialog_credentials::ed25519::Extractable;
use dialog_credentials::key::{ExtractableKey, KeyExport};
use dialog_credentials::secret::{Context, SealedSecret, SecretExtractableDerive};
use dialog_varsig::Principal as _;
use dialog_varsig::eddsa::Ed25519Signature;
use ed25519_dalek::Signer as _;
#[cfg(not(target_arch = "wasm32"))]
use futures::future::BoxFuture;
#[cfg(target_arch = "wasm32")]
use futures::future::LocalBoxFuture;
use wcmp::{Accessor, Resource};
use zeroize::Zeroizing;

use crate::bindings::sapling::kernel::authentication;
use crate::bindings::sapling::kernel::credential::{
    self, Authority, Credential, Custodian, Payload as Signed,
};
use crate::bindings::sapling::kernel::session::{self, Claim, Session};
use crate::bindings::sapling::kernel::types::Error;
use crate::sapling::{KernelHost, Sapling, did_key, trace};

/// The context every key is entrusted in. A guest kernel cannot conceal a
/// message in it, nor reveal one, so it opens a custody only by `recover`.
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

    fn did(&self) -> String {
        match self {
            Self::Seed { seed, .. } => did_key(&ed25519_dalek::SigningKey::from_bytes(seed)),
            Self::Sealed(signer) => signer.did().to_string(),
        }
    }

    /// Whether the guest kernel knew this key's seed.
    fn imported(&self) -> bool {
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

/// The delegations a claim is answered with, once they come: `Send`
/// natively, as a store's futures are, and not in the browser, where a
/// JavaScript promise answers it.
#[cfg(not(target_arch = "wasm32"))]
pub type Answer = BoxFuture<'static, Result<Vec<Vec<u8>>, Error>>;
#[cfg(target_arch = "wasm32")]
pub type Answer = LocalBoxFuture<'static, Result<Vec<Vec<u8>>, Error>>;

/// What answers a guest kernel's claims: the delegations its host holds, or
/// obtains, such as from the person's profile.
pub trait Claimant: ConditionalSync {
    fn claim(&self, claim: Claim) -> Answer;
}

impl<F: Fn(Claim) -> Answer + ConditionalSync> Claimant for F {
    fn claim(&self, claim: Claim) -> Answer {
        self(claim)
    }
}

pub type Claims = Arc<dyn Claimant>;

/// What a session handle names: the key it acts under, and what answers its
/// claims.
pub struct Standing {
    pub credential: Key,
    pub claims: Claims,
}

impl Standing {
    /// The standing of a worker that acts under the key of `seed`, whose
    /// first claim is answered with `handed`, and every claim after it with
    /// none.
    pub fn new(seed: [u8; 32], handed: Vec<Vec<u8>>) -> Self {
        let handed = Mutex::new(handed);
        Self {
            credential: Key::generated(Zeroizing::new(seed)),
            claims: Arc::new(move |_| {
                let chains = core::mem::take(&mut *handed.lock().expect("handed"));
                Box::pin(async move { Ok(chains) }) as Answer
            }),
        }
    }

    /// The standing of a worker that acts under `credential`, a key held as
    /// a signer, whose claims `claims` answers.
    pub fn sealed(credential: Ed25519Signer, claims: Claims) -> Self {
        Self {
            credential: Key::Sealed(Box::new(credential)),
            claims,
        }
    }
}

impl KernelHost {
    fn hold(&mut self, key: Key) -> Resource<Credential> {
        self.keys.push(key);
        Resource::new_own((self.keys.len() - 1) as u32)
    }

    fn key(&self, credential: &Resource<Credential>) -> Key {
        self.keys[credential.rep() as usize].clone()
    }

    fn authority(&self, authority: &Resource<Authority>) -> Key {
        self.authorities[authority.rep() as usize].clone()
    }

    /// A handle on `standing`, for the boot.
    pub fn session(&mut self, standing: Standing) -> Resource<Session> {
        self.sessions.push(standing);
        Resource::new_own((self.sessions.len() - 1) as u32)
    }

    fn standing(&self, session: &Resource<Session>) -> &Standing {
        &self.sessions[session.rep() as usize]
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

    async fn entrust(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        custodian: Custodian,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.entrust");
        let (key, custodian) = accessor.with(|mut access| {
            let host = access.get();
            let custodian = match &custodian {
                Custodian::Credential(held) => host.key(held),
                Custodian::Authority(authority) => host.authority(authority),
            };
            (host.key(&credential), custodian)
        });
        if custodian.imported() {
            return Err(Error::Denied(
                "a key whose seed the guest kernel knows is no custodian".into(),
            ));
        }
        let Key::Seed { seed, .. } = &key else {
            return Err(Error::Denied(
                "a key held as a signer is never entrusted".into(),
            ));
        };
        conceal(&custodian.did(), custody_free(CUSTODY), seed.as_slice()).await
    }

    async fn recover(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        custody: Vec<u8>,
    ) -> Result<Resource<Credential>, Error> {
        trace("credential.recover");
        let custodian = accessor.with(|mut access| access.get().key(&credential));
        let key = recover(&custodian, &custody).await?;
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

impl credential::HostAuthority for KernelHost {
    fn drop(&mut self, _authority: Resource<Authority>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> credential::HostAuthorityWithStore<T> for Sapling {
    async fn did(
        accessor: &Accessor<T, Self>,
        authority: Resource<Authority>,
    ) -> Result<String, Error> {
        Ok(accessor.with(|mut access| access.get().authority(&authority).did()))
    }

    async fn sign(
        accessor: &Accessor<T, Self>,
        authority: Resource<Authority>,
        signed: Signed,
    ) -> Result<Vec<u8>, Error> {
        trace("authority.sign");
        let key = accessor.with(|mut access| access.get().authority(&authority));
        let Signed::Bytes(bytes) = signed;
        key.sign(&bytes).await
    }

    async fn confide(
        accessor: &Accessor<T, Self>,
        authority: Resource<Authority>,
        label: Vec<u8>,
        secret: Vec<u8>,
        to: Resource<Authority>,
    ) -> Result<Vec<u8>, Error> {
        trace("authority.confide");
        let context = context(&label)?;
        let (from, to) = accessor.with(|mut access| {
            let host = access.get();
            (host.authority(&authority), host.authority(&to))
        });
        let message = Zeroizing::new(reveal(&from, context, &secret).await?);
        conceal(&to.did(), context, &message).await
    }

    async fn recover(
        accessor: &Accessor<T, Self>,
        authority: Resource<Authority>,
        custody: Vec<u8>,
    ) -> Result<Resource<Credential>, Error> {
        trace("authority.recover");
        let custodian = accessor.with(|mut access| access.get().authority(&authority));
        let key = recover(&custodian, &custody).await?;
        Ok(accessor.with(|mut access| access.get().hold(key)))
    }

    async fn enroll(
        _accessor: &Accessor<T, Self>,
        _authority: Resource<Authority>,
        _id: Option<String>,
    ) -> Result<(), Error> {
        Err(Error::Unavailable("this host enrolls no passkey".into()))
    }
}

impl session::Host for KernelHost {}

impl session::HostSession for KernelHost {
    fn credential(&mut self, session: Resource<Session>) -> Resource<Credential> {
        let key = self.standing(&session).credential.clone();
        self.hold(key)
    }

    fn drop(&mut self, _session: Resource<Session>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> session::HostSessionWithStore<T> for Sapling {
    async fn claim(
        accessor: &Accessor<T, Self>,
        session: Resource<Session>,
        claim: Claim,
    ) -> Result<Vec<Vec<u8>>, Error> {
        trace("session.claim");
        let claims =
            accessor.with(|mut access| Arc::clone(&access.get().standing(&session).claims));
        claims.claim(claim).await
    }
}

impl authentication::Host for KernelHost {}

impl<T: 'static> authentication::HostWithStore<T> for Sapling {
    async fn authenticate(
        _accessor: &Accessor<T, Self>,
        _id: Option<String>,
    ) -> Result<Resource<Authority>, Error> {
        Err(Error::Unavailable("this host signs no one in".into()))
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
    async fn it_recovers_a_key_entrusted_to_a_sealed_key() {
        let sealed = sealed().await;
        let custody = conceal(&sealed.did(), custody_free(CUSTODY), &[6; 32])
            .await
            .unwrap();
        let recovered = recover(&sealed, &custody).await.unwrap();
        assert_eq!(
            recovered.did(),
            Key::generated(Zeroizing::new([6; 32])).did()
        );
    }
}
