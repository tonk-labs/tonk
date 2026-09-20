//! The document cell: where a document's bytes live.
//!
//! One memory cell per document, `document/<id>` / `automerge` under the
//! space's subject, holding the raw automerge bytes of EVERY branch's
//! changes. It is the same compare-and-swap primitive a branch pointer
//! uses, so a write either replaces exactly the version the writer
//! merged or fails and the writer merges again.
//!
//! A [`Transport`] is one copy of that cell: the local one, or the
//! remote's. The sync pass moves bytes between two transports and never
//! looks inside them except through [`Document::merge`].

use dialog_artifacts::Entity;
use dialog_capability::{Capability, Fork, Provider, Subject};
use dialog_common::ConditionalSync;
use dialog_effects::memory::prelude::*;
use dialog_effects::memory::{Cell, MemoryError, Publish, Resolve, Version};
use dialog_repository::{RemoteSite, SiteAddress};
use thiserror::Error;

use crate::engine::{Document, DocumentError};

/// How often a lost compare-and-swap is retried before giving up. The
/// same bound branch sync uses.
pub const RETRY_LIMIT: usize = 4;

/// The name of the memory space holding a document's cells.
pub fn space(entity: &Entity) -> String {
    format!("document/{}", cell_id(entity))
}

/// The path-safe, fixed-length id of a document's cell: base58 of the
/// BLAKE3 of the entity URI. Entities contain `:` and `/`.
pub fn cell_id(entity: &Entity) -> String {
    let hash = blake3::hash(entity.to_string().as_bytes());
    bs58::encode(hash.as_bytes()).into_string()
}

/// The capability naming a document's cell under `subject`.
pub fn cell(subject: &Subject, entity: &Entity) -> Capability<Cell> {
    subject
        .clone()
        .memory()
        .space(space(entity))
        .cell("automerge")
}

/// Failures moving a document's bytes.
#[derive(Debug, Error)]
pub enum CellError {
    /// The cell moved since the version the write named.
    #[error("the document cell moved")]
    Conflict,
    /// The store refused or failed.
    #[error("document cell: {0}")]
    Memory(String),
    /// The bytes are not a document.
    #[error(transparent)]
    Document(#[from] DocumentError),
    /// The compare-and-swap kept losing.
    #[error("the document cell stayed contended after {RETRY_LIMIT} tries")]
    Contended,
}

impl From<MemoryError> for CellError {
    fn from(error: MemoryError) -> Self {
        match error {
            MemoryError::VersionMismatch { .. } => CellError::Conflict,
            other => CellError::Memory(other.to_string()),
        }
    }
}

/// One copy of a document's cell.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport {
    /// The current bytes and their version, if the cell exists.
    async fn resolve(&self) -> Result<Option<(Vec<u8>, Version)>, CellError>;

    /// Replace the bytes. `when` is the version the writer merged, or
    /// `None` to create the cell; anything else is [`CellError::Conflict`].
    async fn publish(&self, bytes: Vec<u8>, when: Option<Version>) -> Result<Version, CellError>;
}

/// The cell in this replica's own storage.
pub struct LocalCell<'a, Env> {
    cell: Capability<Cell>,
    env: &'a Env,
}

impl<'a, Env> LocalCell<'a, Env> {
    /// The local cell of `entity` under `subject`.
    pub fn new(subject: &Subject, entity: &Entity, env: &'a Env) -> Self {
        Self {
            cell: cell(subject, entity),
            env,
        }
    }

    /// The sync marker of `entity` for the remote named `remote`:
    /// `remote/<remote>/document/<id>` / `synced`, following the
    /// `remote/<name>/branch/<branch>` convention.
    pub fn marker(subject: &Subject, remote: &str, entity: &Entity, env: &'a Env) -> Self {
        Self {
            cell: subject
                .clone()
                .memory()
                .space(format!("remote/{remote}/{}", space(entity)))
                .cell("synced"),
            env,
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<Env> Transport for LocalCell<'_, Env>
where
    Env: Provider<Resolve> + Provider<Publish> + ConditionalSync,
{
    async fn resolve(&self) -> Result<Option<(Vec<u8>, Version)>, CellError> {
        let edition = self.cell.clone().resolve().perform(self.env).await?;
        Ok(edition.map(|edition| (edition.content, edition.version)))
    }

    async fn publish(&self, bytes: Vec<u8>, when: Option<Version>) -> Result<Version, CellError> {
        Ok(self
            .cell
            .clone()
            .publish(bytes, when)
            .perform(self.env)
            .await?)
    }
}

/// The cell at the repository's remote.
pub struct RemoteCell<'a, Env> {
    cell: Capability<Cell>,
    address: SiteAddress,
    env: &'a Env,
}

impl<'a, Env> RemoteCell<'a, Env> {
    /// The remote cell of `entity` under `subject`, at `address`.
    pub fn new(subject: &Subject, entity: &Entity, address: SiteAddress, env: &'a Env) -> Self {
        Self {
            cell: cell(subject, entity),
            address,
            env,
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<Env> Transport for RemoteCell<'_, Env>
where
    Env:
        Provider<Fork<RemoteSite, Resolve>> + Provider<Fork<RemoteSite, Publish>> + ConditionalSync,
{
    async fn resolve(&self) -> Result<Option<(Vec<u8>, Version)>, CellError> {
        let edition = self
            .cell
            .clone()
            .resolve()
            .fork(&self.address)
            .perform(self.env)
            .await?;
        Ok(edition.map(|edition| (edition.content, edition.version)))
    }

    async fn publish(&self, bytes: Vec<u8>, when: Option<Version>) -> Result<Version, CellError> {
        Ok(self
            .cell
            .clone()
            .publish(bytes, when)
            .fork(&self.address)
            .perform(self.env)
            .await?)
    }
}

/// Load the document a transport holds, with the version to write
/// against.
pub async fn load<T: Transport + ?Sized>(
    transport: &T,
) -> Result<Option<(Document, Version)>, CellError> {
    match transport.resolve().await? {
        Some((bytes, version)) => Ok(Some((Document::load(&bytes)?, version))),
        None => Ok(None),
    }
}

/// Store `document`, merging in whatever a concurrent writer stored
/// first. Bytes that are already there stay: a save only ever grows the
/// cell. Returns the version stored.
pub async fn save<T: Transport + ?Sized>(
    transport: &T,
    document: &mut Document,
    mut when: Option<Version>,
) -> Result<Version, CellError> {
    for _ in 0..RETRY_LIMIT {
        match transport.publish(document.save(), when.clone()).await {
            Ok(version) => return Ok(version),
            Err(CellError::Conflict) => match transport.resolve().await? {
                Some((bytes, version)) => {
                    document.merge(&bytes)?;
                    when = Some(version);
                }
                None => when = None,
            },
            Err(other) => return Err(other),
        }
    }
    Err(CellError::Contended)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;

    /// An in-memory cell with real compare-and-swap, for the sync tests.
    #[derive(Default)]
    pub(crate) struct MemoryCell {
        state: Mutex<Option<(Vec<u8>, u64)>>,
    }

    impl MemoryCell {
        /// Overwrite the cell with no precondition — a faulty writer.
        pub(crate) fn clobber(&self, bytes: Vec<u8>) {
            let mut state = self.state.lock().unwrap();
            let next = state.as_ref().map_or(1, |(_, version)| version + 1);
            *state = Some((bytes, next));
        }
    }

    fn version(n: u64) -> Version {
        Version::from(format!("v{n}"))
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl Transport for MemoryCell {
        async fn resolve(&self) -> Result<Option<(Vec<u8>, Version)>, CellError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .as_ref()
                .map(|(bytes, n)| (bytes.clone(), version(*n))))
        }

        async fn publish(
            &self,
            bytes: Vec<u8>,
            when: Option<Version>,
        ) -> Result<Version, CellError> {
            let mut state = self.state.lock().unwrap();
            let current = state.as_ref().map(|(_, n)| version(*n));
            if current != when {
                return Err(CellError::Conflict);
            }
            let next = state.as_ref().map_or(1, |(_, n)| n + 1);
            *state = Some((bytes, next));
            Ok(version(next))
        }
    }
}
