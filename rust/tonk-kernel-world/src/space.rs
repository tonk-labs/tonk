//! A space of Dialog DB on the kernel's world: its archive, memory and blobs
//! in a layer of the host, and its keys and site secrets in the credentials
//! of that layer (PDD015).
//!
//! Dialog DB opens each part of a space from a [`Location`] alone. A location
//! that names a mounted layer (see [`crate::mount`]) opens that layer; any
//! other opens a layer of the host named for the location, as a DID of the
//! method `local`, and every part of the space shares it. The host keeps a
//! layer per name.
//!
//! A space's credential store keeps no key it is handed: a key is made in the
//! host ([`credential::Generate`]), and a signer loaded from it signs through
//! the host. What a space keeps of a key it does not hold is its public
//! identity, in a cell of the layer.
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
use dialog_effects::storage::{Directory, Location, StorageError};
use dialog_storage::provider::{Space, Volatile};
use dialog_storage::resource::Resource;
use dialog_varsig::Principal as _;

use crate::credentials;
use crate::guest::HandleLayer;
use crate::layer::{LayerError, SharedLayer, share};
use crate::store::{Store, Witness};
use crate::world::sapling::kernel::credentials as host;
use crate::world::sapling::kernel::{session, storage};

/// A space of Dialog DB on a layer of the kernel's host.
pub type KernelSpace = Space<Store, Store, Credentials, Volatile, Store>;

/// The namespace of the cells that keep the public identity of a key a space
/// does not hold.
const IDENTITIES: &str = "credential";

/// A layer opened for a location: the store over it, and a handle of its own
/// for the calls of `credentials`, which take a handle of the world.
#[derive(Clone)]
struct Opened {
    store: Store,
    handle: Rc<storage::Layer>,
}

// SAFETY: a component on WASI has no threads, so what an entry holds is never
// reached from another thread.
unsafe impl Send for Opened {}

static OPENED: Mutex<BTreeMap<String, Opened>> = Mutex::new(BTreeMap::new());

/// The name of the layer `location` opens. A space, which Tonk opens in the
/// current directory under its subject's DID, opens the layer of that
/// subject: the one its host boots it over, which holds its boot cell and its
/// kernel. Anything else opens a layer named by a DID of the method `local`,
/// with each character a DID does not allow escaped.
pub fn layer_name(location: &Location) -> String {
    if location.directory == Directory::Current
        && location.name.starts_with("did:")
        && !location.name.starts_with("did:local:")
    {
        return location.name.clone();
    }
    let directory = match &location.directory {
        Directory::Profile => "profile".to_owned(),
        Directory::Current => "current".to_owned(),
        Directory::Temp => "temp".to_owned(),
        Directory::At(path) => format!("at:{path}"),
    };
    let escape = |text: &str| {
        text.bytes()
            .map(|byte| match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                    (byte as char).to_string()
                }
                other => format!("%{other:02X}"),
            })
            .collect::<String>()
    };
    format!(
        "did:local:{}:{}",
        escape(&directory),
        escape(&location.name)
    )
}

async fn opened(location: &Location) -> Result<Opened, StorageError> {
    let name = layer_name(location);
    if let Some(opened) = OPENED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&name)
    {
        return Ok(opened.clone());
    }
    let refused = |error| StorageError::Storage(format!("the host opened no layer: {error:?}"));
    let layer = storage::open(name.clone(), session::chain())
        .await
        .map_err(refused)?;
    let handle = storage::open(name.clone(), session::chain())
        .await
        .map_err(refused)?;
    let layer =
        HandleLayer::new(layer).map_err(|error| StorageError::Storage(error.to_string()))?;
    let opened = Opened {
        store: Store::new(share(layer), Witness::default()),
        handle: Rc::new(handle),
    };
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
    Ok(opened(location).await?.store)
}

/// The credentials of a layer: its keys, made and kept by the host, and its
/// site secrets.
#[derive(Clone)]
pub struct Credentials {
    cells: SharedLayer,
    handle: Rc<storage::Layer>,
}

#[async_trait(?Send)]
impl Resource<Location> for Credentials {
    type Error = StorageError;

    async fn open(location: &Location) -> Result<Self, StorageError> {
        let opened = opened(location).await?;
        Ok(Self {
            cells: Arc::clone(opened.store.layer()),
            handle: opened.handle,
        })
    }
}

fn failed(error: impl std::fmt::Debug) -> CredentialError {
    CredentialError::Storage(format!("{error:?}"))
}

fn cell(address: &str) -> String {
    format!("key/{address}")
}

impl Credentials {
    /// The key the host holds at `address`, as a signer.
    async fn held(&self, address: &str) -> Result<Option<Credential>, CredentialError> {
        let Some(credential) = host::load(&self.handle, address.to_owned())
            .await
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let signer = credentials::signer(credential).map_err(failed)?;
        Ok(Some(Credential::Signer(signer)))
    }

    /// The public identity kept at `address`, of a key the host does not hold.
    async fn identity(&self, address: &str) -> Result<Option<Credential>, CredentialError> {
        let Some(edition) = self
            .cells
            .resolve(IDENTITIES, &cell(address))
            .await
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let did = String::from_utf8(edition.content).map_err(failed)?;
        let verifier: Verifier = did
            .parse()
            .map_err(|_| CredentialError::Corrupted(format!("{did} is not a verifier")))?;
        Ok(Some(Credential::Verifier(verifier.into())))
    }

    async fn keep_identity(&self, address: &str, did: String) -> Result<(), CredentialError> {
        let current = self
            .cells
            .resolve(IDENTITIES, &cell(address))
            .await
            .map_err(failed)?;
        self.cells
            .publish(
                IDENTITIES,
                &cell(address),
                did.into_bytes(),
                current.map(|edition| edition.version),
            )
            .await
            .map(|_| ())
            .map_err(failed)
    }
}

#[async_trait(?Send)]
impl Provider<Generate> for Credentials {
    async fn execute(&self, input: Capability<Generate>) -> Result<Credential, CredentialError> {
        let credential = host::generate(&self.handle, input.address().to_owned())
            .await
            .map_err(failed)?;
        let signer = credentials::signer(credential).map_err(failed)?;
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
                self.keep_identity(address, verifier.did().to_string())
                    .await
            }
            // A key the host holds at this address is already kept; any other
            // was made outside the host, and a kernel keeps none.
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
        host::forget(&self.handle, address.to_owned())
            .await
            .map_err(failed)?;
        if let Some(edition) = self
            .cells
            .resolve(IDENTITIES, &cell(address))
            .await
            .map_err(failed)?
        {
            match self
                .cells
                .retract(IDENTITIES, &cell(address), edition.version)
                .await
            {
                Ok(()) | Err(LayerError::Conflict) => {}
                Err(error) => return Err(failed(error)),
            }
        }
        Ok(())
    }
}

#[async_trait(?Send)]
impl Provider<Load<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Load<Secret>>) -> Result<Secret, CredentialError> {
        let site = input.address().to_string();
        host::load_secret(&self.handle, site.clone())
            .await
            .map_err(failed)?
            .map(Secret::from)
            .ok_or(CredentialError::NotFound(site))
    }
}

#[async_trait(?Send)]
impl Provider<Save<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Save<Secret>>) -> Result<(), CredentialError> {
        host::save_secret(
            &self.handle,
            input.address().to_string(),
            input.secret().as_bytes().to_vec(),
        )
        .await
        .map_err(failed)
    }
}

#[async_trait(?Send)]
impl Provider<Retract<Secret>> for Credentials {
    async fn execute(&self, input: Capability<Retract<Secret>>) -> Result<(), CredentialError> {
        host::retract_secret(&self.handle, input.address().to_string())
            .await
            .map_err(failed)
    }
}
