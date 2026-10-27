//! A space of Dialog DB on the kernel's world: its archive, memory and blobs
//! in a layer of the host, and its keys and site secrets in that layer's
//! cells, under the session's credential.
//!
//! Dialog DB opens each part of a space from a [`Location`] alone. A location
//! that names a mounted layer (see [`crate::mount`]) opens that layer; any
//! other is the worker's own, and opens the layer of its stack: the layer
//! itself for the stack's subject, and a part of it for any other location
//! (see [`crate::stack`]), so that every part of a space shares one layer.
//!
//! A space's credential store keeps no key: a key is made in the host
//! ([`credential::Generate`]) and entrusted to the session's credential, and
//! its custody is what the store keeps, in a cell. Loading the key recovers
//! it from the custody, as a handle, so its seed never reaches the guest
//! kernel. A site's secret is concealed to the session's credential and kept
//! in a cell the same way. What a space keeps of a key it does not hold is
//! its public identity, in a cell too.
//!
//! Not yet: certificates (retained delegation chains) are kept in memory, so a
//! worker that starts again proves from none until it retains them again.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use dialog_capability::{Capability, Provider};
use dialog_credentials::{Credential, Verifier};
use dialog_effects::credential::prelude::{
    GenerateCredentialExt as _, LoadCredentialExt as _, LoadSecretExt as _,
    RetractCredentialExt as _, RetractSecretExt as _, SaveCredentialExt as _, SaveSecretExt as _,
};
use dialog_effects::credential::{CredentialError, Generate, Load, Retract, Save, Secret};
use dialog_effects::storage::{Location, StorageError};
use dialog_storage::provider::{Space, Volatile};
use dialog_storage::resource::Resource;
use dialog_varsig::Principal as _;

use crate::credentials;
use crate::layer::{LayerError, SharedLayer, share};
use crate::naming::layer_name;
use crate::session;
use crate::store::{Store, Witness};
use crate::world::sapling::kernel::credential::{self as host, Custodian};

/// A space of Dialog DB on a layer of the kernel's host.
pub type KernelSpace = Space<Store, Store, Credentials, Volatile, Store>;

/// The namespace of the cells that keep the public identity of a key a space
/// does not hold.
const IDENTITIES: &str = "credential";
/// The namespace of the cells that keep each key's custody, by its address.
const CUSTODIES: &str = "custody";
/// The namespace of the cells that keep each site's secret, by the site.
const SECRETS: &str = "secret";
/// The context a site's secret is concealed in, as Dialog DB conceals one.
const SITE_SECRET: &[u8] = b"dialog.secret/message";

/// The store over a layer opened for a location.
#[derive(Clone)]
struct Opened(Store);

// SAFETY: a component on WASI has no threads, so what an entry holds is never
// reached from another thread.
unsafe impl Send for Opened {}

static OPENED: Mutex<BTreeMap<String, Opened>> = Mutex::new(BTreeMap::new());

async fn opened(location: &Location) -> Result<Opened, StorageError> {
    let name = layer_name(location);
    if let Some(opened) = OPENED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&name)
    {
        return Ok(opened.clone());
    }
    // Every location of this worker's own is in the layer of its stack: the
    // stack's subject is that layer, and any other location a part of it.
    let stack = crate::stack::layer().ok_or_else(|| {
        StorageError::Storage("the boot handed this worker no layer of its own".into())
    })?;
    let layer = if stack.subject().to_string() == name {
        stack
    } else {
        share(crate::stack::Within::new(stack, name.clone()))
    };
    let opened = Opened(Store::new(layer, Witness::default()));
    OPENED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name, opened.clone());
    Ok(opened)
}

/// The store a location opens: a mounted layer, or the host's layer for it.
pub(crate) async fn store(location: &Location) -> Result<Store, StorageError> {
    if let Some(store) = crate::mount::mounted(location) {
        return Ok(store);
    }
    Ok(opened(location).await?.0)
}

/// The credentials of a layer: the custody of each of its keys, and its site
/// secrets, in its cells.
#[derive(Clone)]
pub struct Credentials {
    cells: SharedLayer,
}

#[async_trait(?Send)]
impl Resource<Location> for Credentials {
    type Error = StorageError;

    async fn open(location: &Location) -> Result<Self, StorageError> {
        let opened = opened(location).await?;
        Ok(Self {
            cells: Arc::clone(opened.0.layer()),
        })
    }
}

fn failed(error: impl std::fmt::Debug) -> CredentialError {
    CredentialError::Storage(format!("{error:?}"))
}

impl Credentials {
    async fn read(&self, namespace: &str, cell: &str) -> Result<Option<Vec<u8>>, CredentialError> {
        Ok(self
            .cells
            .resolve(namespace, cell)
            .await
            .map_err(failed)?
            .map(|edition| edition.content))
    }

    async fn write(
        &self,
        namespace: &str,
        cell: &str,
        content: Vec<u8>,
    ) -> Result<(), CredentialError> {
        let current = self.cells.resolve(namespace, cell).await.map_err(failed)?;
        self.cells
            .publish(
                namespace,
                cell,
                content,
                current.map(|edition| edition.version),
            )
            .await
            .map(|_| ())
            .map_err(failed)
    }

    async fn erase(&self, namespace: &str, cell: &str) -> Result<(), CredentialError> {
        let Some(edition) = self.cells.resolve(namespace, cell).await.map_err(failed)? else {
            return Ok(());
        };
        match self.cells.retract(namespace, cell, edition.version).await {
            Ok(()) | Err(LayerError::Conflict) => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }

    /// The key whose custody is kept at `address`, recovered by the session's
    /// credential, as a signer.
    async fn held(&self, address: &str) -> Result<Option<Credential>, CredentialError> {
        let Some(custody) = self.read(CUSTODIES, address).await? else {
            return Ok(None);
        };
        let credential = session::credential()
            .recover(custody)
            .await
            .map_err(failed)?;
        let signer = credentials::signer(Rc::new(credential)).map_err(failed)?;
        Ok(Some(Credential::Signer(signer)))
    }

    /// The public identity kept at `address`, of a key the space does not hold.
    async fn identity(&self, address: &str) -> Result<Option<Credential>, CredentialError> {
        let Some(did) = self.read(IDENTITIES, address).await? else {
            return Ok(None);
        };
        let did = String::from_utf8(did).map_err(failed)?;
        let verifier: Verifier = did
            .parse()
            .map_err(|_| CredentialError::Corrupted(format!("{did} is not a verifier")))?;
        Ok(Some(Credential::Verifier(verifier.into())))
    }
}

#[async_trait(?Send)]
impl Provider<Generate> for Credentials {
    async fn execute(&self, input: Capability<Generate>) -> Result<Credential, CredentialError> {
        let credential = host::generate().await.map_err(failed)?;
        let custodian = session::credential();
        let custody = credential
            .entrust(Custodian::Credential(&custodian))
            .await
            .map_err(failed)?;
        self.write(CUSTODIES, input.address(), custody).await?;
        let signer = credentials::signer(Rc::new(credential)).map_err(failed)?;
        Ok(Credential::Signer(signer))
    }
}

#[async_trait(?Send)]
impl Provider<Load<Credential>> for Credentials {
    async fn execute(
        &self,
        input: Capability<Load<Credential>>,
    ) -> Result<Credential, CredentialError> {
        let address = input.address();
        if let Some(signer) = self.held(address).await? {
            return Ok(signer);
        }
        self.identity(address)
            .await?
            .ok_or_else(|| CredentialError::NotFound(address.to_owned()))
    }
}

#[async_trait(?Send)]
impl Provider<Save<Credential>> for Credentials {
    async fn execute(&self, input: Capability<Save<Credential>>) -> Result<(), CredentialError> {
        let address = input.address();
        match input.credential() {
            Credential::Verifier(verifier) => {
                self.write(IDENTITIES, address, verifier.did().to_string().into_bytes())
                    .await
            }
            // A key whose custody is kept at this address is already kept;
            // any other was made outside the host, and a kernel keeps none.
            Credential::Signer(signer) => match self.held(address).await? {
                Some(held) if held.did() == signer.did() => Ok(()),
                _ => Err(CredentialError::Withheld(format!(
                    "a kernel keeps no key it is handed: generate the key at {address} in its host"
                ))),
            },
        }
    }
}

#[async_trait(?Send)]
impl Provider<Retract<Credential>> for Credentials {
    async fn execute(&self, input: Capability<Retract<Credential>>) -> Result<(), CredentialError> {
        let address = input.address();
        self.erase(CUSTODIES, address).await?;
        self.erase(IDENTITIES, address).await
    }
}

#[async_trait(?Send)]
impl Provider<Load<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Load<Secret>>) -> Result<Secret, CredentialError> {
        let site = input.address().to_string();
        let Some(secret) = self.read(SECRETS, &site).await? else {
            return Err(CredentialError::NotFound(site));
        };
        session::credential()
            .reveal(SITE_SECRET.to_vec(), secret)
            .await
            .map(Secret::from)
            .map_err(failed)
    }
}

#[async_trait(?Send)]
impl Provider<Save<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Save<Secret>>) -> Result<(), CredentialError> {
        let secret = host::conceal(
            session::credential().did(),
            SITE_SECRET.to_vec(),
            input.secret().as_bytes().to_vec(),
        )
        .await
        .map_err(failed)?;
        self.write(SECRETS, input.address().as_ref(), secret).await
    }
}

#[async_trait(?Send)]
impl Provider<Retract<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Retract<Secret>>) -> Result<(), CredentialError> {
        self.erase(SECRETS, input.address().as_ref()).await
    }
}
