//! The credentials of a layer (PDD015): its keys at their addresses, and its
//! secrets for sites. A guest kernel holds a credential as a resource that
//! names one; the seed stays here. Each call acts with the seed through Dialog
//! DB's own credentials, so what a guest kernel seals with Dialog DB this host
//! reveals, and the reverse.
//!
//! Not yet bound: the reach of a handle over its credentials (a handle reaches
//! every key and secret of its subject), and consent for a credential that
//! names a persona.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use dialog_credentials::Ed25519Signer;
use dialog_credentials::ed25519::Extractable;
use dialog_credentials::key::{ExtractableKey, KeyExport};
use dialog_credentials::secret::{Context, SealedSecret, SecretExtractableDerive};
use dialog_varsig::signature::Signer as _;
use ipld_core::ipld::Ipld;
use wcmp::{Accessor, Resource};
use zeroize::Zeroizing;

use crate::bindings::sapling::kernel::credentials::{self, Credential};
use crate::bindings::sapling::kernel::storage::Layer;
use crate::bindings::sapling::kernel::types::Error;
use crate::sapling::{Change, KernelHost, Sapling, did_key, trace};

/// What a `credential` handle names: the key of a subject, by its seed.
pub struct Held {
    subject: String,
    seed: Zeroizing<[u8; 32]>,
}

/// The kind of UCAN a payload is, read from inside the bytes its signature
/// covers: the key of its payload map, such as `ucan/inv@1.0.0-rc.1`.
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Invocation,
    Delegation,
}

impl Kind {
    /// The kind of the UCAN payload `bytes`, if it is one.
    pub fn of(bytes: &[u8]) -> Option<Kind> {
        let Ok(Ipld::Map(map)) = serde_ipld_dagcbor::from_slice::<Ipld>(bytes) else {
            return None;
        };
        let mut kinds = map.keys().filter_map(|key| match key.split_once('@') {
            Some(("ucan/inv", _)) => Some(Kind::Invocation),
            Some(("ucan/dlg", _)) => Some(Kind::Delegation),
            _ => None,
        });
        match (kinds.next(), kinds.next()) {
            (Some(kind), None) if map.len() == 2 && map.contains_key("h") => Some(kind),
            _ => None,
        }
    }
}

impl KernelHost {
    fn hold(&mut self, subject: String, seed: Zeroizing<[u8; 32]>) -> Resource<Credential> {
        self.credentials.push(Held { subject, seed });
        Resource::new_own((self.credentials.len() - 1) as u32)
    }

    fn held(&self, credential: &Resource<Credential>) -> &Held {
        &self.credentials[credential.rep() as usize]
    }

    fn keep_key(&mut self, subject: &str, address: &str, seed: Option<&Zeroizing<[u8; 32]>>) {
        self.journal.push(Change::Key {
            subject: subject.to_owned(),
            address: address.to_owned(),
            seed: seed.map(|seed| seed.to_vec()),
        });
    }
}

/// A context Dialog DB can take: its labels are `'static`, and a guest kernel
/// names a context by text. Each label is kept once.
fn context(label: &str) -> Context {
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

async fn sign<T: 'static>(
    accessor: &Accessor<T, Sapling>,
    credential: &Resource<Credential>,
    payload: Vec<u8>,
    kind: Kind,
) -> Result<Vec<u8>, Error> {
    if Kind::of(&payload).as_ref() != Some(&kind) {
        return Err(Error::Denied(format!("the payload is not a UCAN {kind:?}")));
    }
    let seed = accessor.with(|mut access| access.get().held(credential).seed.clone());
    let signature = signer(&seed)
        .await?
        .sign(&payload)
        .await
        .map_err(|error| Error::Invalid(error.to_string()))?;
    Ok(signature.to_bytes().to_vec())
}

/// Sign `tag` and then `payload` with the key of `credential`, as Dialog DB
/// signs a head or a revision record.
fn sign_tagged<T: 'static>(
    accessor: &Accessor<T, Sapling>,
    credential: &Resource<Credential>,
    tag: &str,
    payload: &[u8],
) -> Vec<u8> {
    use ed25519_dalek::Signer as _;
    let seed = accessor.with(|mut access| access.get().held(credential).seed.clone());
    let mut message = tag.as_bytes().to_vec();
    message.extend_from_slice(payload);
    ed25519_dalek::SigningKey::from_bytes(&seed)
        .sign(&message)
        .to_bytes()
        .to_vec()
}

impl credentials::Host for KernelHost {}

impl credentials::HostCredential for KernelHost {
    fn did(&mut self, credential: Resource<Credential>) -> String {
        let seed = &self.held(&credential).seed;
        did_key(&ed25519_dalek::SigningKey::from_bytes(seed))
    }

    fn drop(&mut self, _credential: Resource<Credential>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> credentials::HostCredentialWithStore<T> for Sapling {
    async fn sign_invocation(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        payload: Vec<u8>,
        _consent: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        trace("sign_invocation");
        sign(accessor, &credential, payload, Kind::Invocation).await
    }

    async fn sign_delegation(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        payload: Vec<u8>,
        _consent: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        trace("sign_delegation");
        sign(accessor, &credential, payload, Kind::Delegation).await
    }

    async fn sign_head(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.sign_head");
        Ok(sign_tagged(
            accessor,
            &credential,
            "dialog/head@1\n",
            &payload,
        ))
    }

    async fn sign_revision_record(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("credential.sign_revision_record");
        Ok(sign_tagged(
            accessor,
            &credential,
            "dialog/revision-record@1\n",
            &payload,
        ))
    }

    async fn reveal(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        label: String,
        sealed: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("reveal");
        let seed = accessor.with(|mut access| access.get().held(&credential).seed.clone());
        let sealed =
            SealedSecret::from_bytes(&sealed).map_err(|error| Error::Invalid(error.to_string()))?;
        signer(&seed)
            .await?
            .secret(context(&label))
            .reveal(&sealed)
            .await
            .map_err(|_| Error::Denied("the secret was not sealed to this credential".into()))
    }

    async fn derive(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        label: String,
        derivation: Vec<u8>,
        _consent: Option<Vec<u8>>,
    ) -> Result<Resource<Credential>, Error> {
        trace("derive");
        let (subject, seed) = accessor.with(|mut access| {
            let held = access.get().held(&credential);
            (held.subject.clone(), held.seed.clone())
        });
        let signer = signer(&seed).await?;
        let derived = SecretExtractableDerive::derive(&signer.secret(context(&label)), &derivation)
            .await
            .map_err(|error| Error::Invalid(error.to_string()))?;
        let seed = seed_of(&derived).await?;
        Ok(accessor.with(|mut access| access.get().hold(subject, seed)))
    }

    async fn seal(
        accessor: &Accessor<T, Self>,
        credential: Resource<Credential>,
        to: String,
        label: String,
        _consent: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        trace("seal");
        let seed = accessor.with(|mut access| access.get().held(&credential).seed.clone());
        let recipient: dialog_credentials::Ed25519Verifier = to
            .parse()
            .map_err(|_| Error::Invalid(format!("{to} is not an Ed25519 did:key")))?;
        let sealed = recipient
            .secret(context(&label))
            .conceal(seed.as_slice())
            .await
            .map_err(|error| Error::Invalid(error.to_string()))?;
        Ok(sealed.to_bytes())
    }
}

impl<T: 'static> credentials::HostWithStore<T> for Sapling {
    async fn load(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        address: String,
    ) -> Result<Option<Resource<Credential>>, Error> {
        trace("credentials.load");
        Ok(accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            let seed = host.subject(&at).keys.get(&address).cloned()?;
            Some(host.hold(subject, seed))
        }))
    }

    async fn generate(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        address: String,
    ) -> Result<Resource<Credential>, Error> {
        trace("credentials.generate");
        let seed = fresh_seed()?;
        Ok(accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            host.keep_key(&subject, &address, Some(&seed));
            host.subject(&at).keys.insert(address, seed.clone());
            host.hold(subject, seed)
        }))
    }

    async fn adopt(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        address: String,
        sealed: Vec<u8>,
        by: Resource<Credential>,
        label: String,
    ) -> Result<Resource<Credential>, Error> {
        trace("credentials.adopt");
        let opener = accessor.with(|mut access| access.get().held(&by).seed.clone());
        let sealed =
            SealedSecret::from_bytes(&sealed).map_err(|error| Error::Invalid(error.to_string()))?;
        let revealed = Zeroizing::new(
            signer(&opener)
                .await?
                .secret(context(&label))
                .reveal(&sealed)
                .await
                .map_err(|_| Error::Denied("the credential was not sealed to `by`".into()))?,
        );
        let seed = <[u8; 32]>::try_from(revealed.as_slice())
            .map(Zeroizing::new)
            .map_err(|_| Error::Invalid("a sealed credential holds a 32-byte seed".into()))?;
        Ok(accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            host.keep_key(&subject, &address, Some(&seed));
            host.subject(&at).keys.insert(address, seed.clone());
            host.hold(subject, seed)
        }))
    }

    async fn forget(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        address: String,
    ) -> Result<(), Error> {
        trace("credentials.forget");
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            host.keep_key(&subject, &address, None);
            host.subject(&at).keys.remove(&address);
        });
        Ok(())
    }

    async fn load_secret(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        site: String,
    ) -> Result<Option<Vec<u8>>, Error> {
        trace("credentials.load_secret");
        Ok(accessor.with(|mut access| access.get().subject(&at).secrets.get(&site).cloned()))
    }

    async fn save_secret(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        site: String,
        secret: Vec<u8>,
    ) -> Result<(), Error> {
        trace("credentials.save_secret");
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            host.journal.push(Change::Secret {
                subject,
                site: site.clone(),
                secret: Some(secret.clone()),
            });
            host.subject(&at).secrets.insert(site, secret);
        });
        Ok(())
    }

    async fn retract_secret(
        accessor: &Accessor<T, Self>,
        at: Resource<Layer>,
        site: String,
    ) -> Result<(), Error> {
        trace("credentials.retract_secret");
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&at);
            host.journal.push(Change::Secret {
                subject,
                site: site.clone(),
                secret: None,
            });
            host.subject(&at).secrets.remove(&site);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(kind: &str) -> Vec<u8> {
        let mut map = BTreeMap::new();
        map.insert("h".to_owned(), Ipld::Bytes(vec![0x34, 0xed, 0x01]));
        map.insert(kind.to_owned(), Ipld::Map(BTreeMap::new()));
        serde_ipld_dagcbor::to_vec(&Ipld::Map(map)).unwrap()
    }

    #[test]
    fn it_reads_the_kind_inside_a_ucan_payload() {
        assert_eq!(
            Kind::of(&payload("ucan/inv@1.0.0-rc.1")),
            Some(Kind::Invocation)
        );
        assert_eq!(
            Kind::of(&payload("ucan/dlg@1.0.0-rc.1")),
            Some(Kind::Delegation)
        );
    }

    #[test]
    fn it_names_no_kind_for_bytes_that_are_not_a_ucan_payload() {
        assert_eq!(Kind::of(b"dialog/head@1\nsomething"), None);
        assert_eq!(Kind::of(&payload("dialog/head@1")), None);
    }
}
