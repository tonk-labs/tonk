//! The keys a guest kernel reaches through handles: `credential` and
//! `session`. Every key's seed stays here. A guest kernel signs what it can
//! name as a payload, conceals its own messages to a DID, and entrusts a key
//! to a custodian whose key is here too, so no seed reaches it.
//!
//! Not yet bound: an authority's key is held here as a credential's is, where
//! a browser would open it only with a passkey.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use dialog_credentials::Ed25519Signer;
use dialog_credentials::Ed25519Verifier;
use dialog_credentials::ed25519::Extractable;
use dialog_credentials::key::{ExtractableKey, KeyExport};
use dialog_credentials::secret::{Context, SealedSecret, SecretExtractableDerive};
use ed25519_dalek::Signer as _;
use tonk_kernel_world::payload::{self, Payload};
use wcmp::{Accessor, Resource};
use zeroize::Zeroizing;

use crate::bindings::sapling::kernel::credential::{
    self, Authority, Credential, Custodian, Payload as Signed,
};
use crate::bindings::sapling::kernel::session::{self, Session};
use crate::bindings::sapling::kernel::types::Error;
use crate::sapling::{KernelHost, Sapling, did_key, trace};

/// The context every key is entrusted in. A guest kernel cannot conceal a
/// message in it, nor reveal one, so it opens a custody only by `recover`.
const CUSTODY: &str = "sapling.kernel/custody";

/// A key a handle names: its seed, and whether the guest kernel knew the
/// seed, which makes it no custodian.
#[derive(Clone)]
pub struct Key {
    seed: Zeroizing<[u8; 32]>,
    imported: bool,
}

impl Key {
    fn generated(seed: Zeroizing<[u8; 32]>) -> Self {
        Self {
            seed,
            imported: false,
        }
    }

    fn did(&self) -> String {
        did_key(&ed25519_dalek::SigningKey::from_bytes(&self.seed))
    }
}

/// What a session handle names: the key it acts under, the authority it acts
/// for, and the delegations between them.
pub struct Standing {
    pub credential: Key,
    pub authority: Key,
    pub proofs: Vec<Vec<u8>>,
}

impl Standing {
    /// The standing of a worker that acts under the key of `seed`, for the
    /// authority of `authority`, with `proofs`.
    pub fn new(seed: [u8; 32], authority: [u8; 32], proofs: Vec<Vec<u8>>) -> Self {
        Self {
            credential: Key::generated(Zeroizing::new(seed)),
            authority: Key::generated(Zeroizing::new(authority)),
            proofs,
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

    fn hold_authority(&mut self, key: Key) -> Resource<Authority> {
        self.authorities.push(key);
        Resource::new_own((self.authorities.len() - 1) as u32)
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
    signer(&key.seed)
        .await?
        .secret(context)
        .reveal(&sealed)
        .await
        .map_err(|_| Error::Denied("it is not concealed to this key, in this context".into()))
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

/// `signed`, as the payload the guest kernel names.
fn payload_of(signed: Signed) -> Result<Payload, Error> {
    let hash = |bytes: Vec<u8>| {
        <[u8; 32]>::try_from(bytes).map_err(|_| Error::Invalid("a hash is 32 bytes".into()))
    };
    Ok(match signed {
        Signed::Invocation(invocation) => Payload::Invocation(payload::Invocation {
            audience: invocation.audience,
            subject: invocation.subject,
            command: invocation.command,
            arguments: invocation.arguments,
            proofs: invocation.proofs,
            cause: invocation.cause,
            issued_at: invocation.issued_at,
            expiration: invocation.expiration,
            meta: invocation.meta,
            nonce: invocation.nonce,
        }),
        Signed::Delegation(delegation) => Payload::Delegation(payload::Delegation {
            audience: delegation.audience,
            subject: delegation.subject,
            command: delegation.command,
            policy: delegation.policy,
            expiration: delegation.expiration,
            not_before: delegation.not_before,
            meta: delegation.meta,
            nonce: delegation.nonce,
        }),
        Signed::Head(head) => Payload::Head(payload::Head {
            branch: head.branch,
            tree: hash(head.tree)?,
            edition: head.edition,
            context: head
                .context
                .map(|context| {
                    context
                        .into_iter()
                        .map(|watermark| {
                            Ok(payload::Watermark {
                                origin: hash(watermark.origin)?,
                                edition: watermark.edition,
                                count: watermark.count,
                            })
                        })
                        .collect::<Result<_, Error>>()
                })
                .transpose()?,
        }),
        Signed::RevisionRecord(record) => {
            Payload::RevisionRecord(payload::RevisionRecord { body: record.body })
        }
    })
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
        let key = Key {
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
        let bytes = payload::write(&payload_of(signed)?, &key.did()).map_err(Error::Invalid)?;
        Ok(ed25519_dalek::SigningKey::from_bytes(&key.seed)
            .sign(&bytes)
            .to_bytes()
            .to_vec())
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
        if custodian.imported {
            return Err(Error::Denied(
                "a key whose seed the guest kernel knows is no custodian".into(),
            ));
        }
        conceal(&custodian.did(), custody_free(CUSTODY), key.seed.as_slice()).await
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
        let signer = signer(&key.seed).await?;
        let derived = SecretExtractableDerive::derive(&signer.secret(context), &derivation)
            .await
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
}

impl session::Host for KernelHost {}

impl session::HostSession for KernelHost {
    fn credential(&mut self, session: Resource<Session>) -> Resource<Credential> {
        let key = self.standing(&session).credential.clone();
        self.hold(key)
    }

    fn authority(&mut self, session: Resource<Session>) -> Resource<Authority> {
        let key = self.standing(&session).authority.clone();
        self.hold_authority(key)
    }

    fn proofs(&mut self, session: Resource<Session>) -> Vec<Vec<u8>> {
        self.standing(&session).proofs.clone()
    }

    fn drop(&mut self, _session: Resource<Session>) -> wcmp::Result<()> {
        Ok(())
    }
}
