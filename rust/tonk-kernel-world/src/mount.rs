//! The mounts of layers, by location: how Dialog DB opens a space on a layer.
//!
//! Dialog DB opens each provider of a space from a `Location` alone
//! (`Resource<Location>::open`), with no other context. So a layer is mounted
//! here first, under a location of its own, and the space opens from that
//! location. The name of the location is the DID of the subject and a number,
//! so that two layers of one subject, such as those of two workers in the
//! daemon, never meet. A [`Mount`] takes the layer out again when it drops;
//! the space that Dialog DB opened keeps its own handle on it. A location
//! that names no mounted layer opens the host's layer for it (see
//! [`crate::space`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use dialog_effects::storage::{Directory, Location, StorageError};
use dialog_storage::resource::Resource;

use crate::store::Store;

/// The directory of each location that names a mounted layer.
pub const DIRECTORY: &str = "sapling:layer";

/// One mounted store.
struct Entry(Store);

// SAFETY: a build for `wasm32` runs on one thread, so the store, whose layer
// need not be `Send` there, is never reached from another thread.
#[cfg(target_arch = "wasm32")]
unsafe impl Send for Entry {}

static MOUNTS: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(0);

fn mounts() -> std::sync::MutexGuard<'static, BTreeMap<String, Entry>> {
    MOUNTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A store, mounted at [`Mount::location`] until this drops.
pub struct Mount {
    location: Location,
}

impl Mount {
    /// Mounts `store` at a location of its own.
    pub fn new(store: Store) -> Self {
        let name = format!(
            "{}#{}",
            store.layer().subject(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        mounts().insert(name.clone(), Entry(store));
        Self {
            location: Location::new(Directory::At(DIRECTORY.to_owned()), name),
        }
    }

    /// The location that the store is mounted at.
    pub fn location(&self) -> &Location {
        &self.location
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        mounts().remove(&self.location.name);
    }
}

/// The store mounted at `location`, if one is.
pub(crate) fn mounted(location: &Location) -> Option<Store> {
    match &location.directory {
        Directory::At(directory) if directory == DIRECTORY => {
            mounts().get(&location.name).map(|entry| entry.0.clone())
        }
        _ => None,
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Resource<Location> for Store {
    type Error = StorageError;

    fn is_not_found(error: &StorageError) -> bool {
        matches!(error, StorageError::NotFound(_))
    }

    async fn open(location: &Location) -> Result<Self, StorageError> {
        crate::space::store(location).await
    }
}
