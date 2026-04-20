//! Per-profile index of repos the user has access to.
//!
//! Dialog-repository does not expose a way to enumerate opened repos
//! across both native and WASM targets, so the worker maintains its own
//! index. It is a cache of [`RepoEntry`] rows written on every successful
//! `create` or `claim` and read by `GET /api/repositories`.
//!
//! **Persistence:**
//!
//! - WASM: stored as a JSON blob in IndexedDB database `tonk-meta`,
//!   object store `repo-index`, key `v1`. Survives service worker
//!   restarts and page reloads.
//! - Native (tests only): in-memory. Tests do not exercise persistence.
//!
//! The index is owned by [`crate::worker::TonkState`] and is therefore
//! serialized by the outer [`tokio::sync::RwLock`] wrapping `TonkState`
//! in [`crate::router::AppState`] — no additional locking needed here.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single repo the profile has access to. Used as both the HTTP
/// response shape (see `router::repos`) and the persisted storage shape.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoEntry {
    /// Local repo name (storage key, URL path segment).
    pub local_repo: String,
    /// Subject DID the repo tracks. For self-owned repos this is the
    /// local repo's own DID; for invited repos, the inviter's repo DID.
    pub subject: String,
    /// Sync remote URL if one was configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// Whether the default branch has an upstream configured.
    pub has_upstream: bool,
}

/// Errors from the index. Failing to persist is surfaced loudly rather
/// than silently swallowed — otherwise a crashed write produces a stale
/// sidebar that divergences from what's actually in storage.
#[derive(Debug, Error)]
pub enum RepoIndexError {
    /// Serialization of the index to JSON failed.
    #[error("failed to serialize repo index: {0}")]
    Serialize(#[from] serde_json::Error),
    /// IndexedDB operation failed (WASM only).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[error("IndexedDB operation failed: {0}")]
    Storage(String),
}

/// In-memory list of repos, with the storage backend cfg-gated below.
/// Not internally locked — see the module doc comment.
#[derive(Default)]
pub struct RepoIndex {
    entries: Vec<RepoEntry>,
}

impl RepoIndex {
    /// Load the index from persistent storage, or start empty if none exists.
    pub async fn restore() -> Self {
        let entries = storage::load().await.unwrap_or_default();
        Self { entries }
    }

    /// Snapshot the current list. Callers receive an owned copy because
    /// the lock wrapping `TonkState` is held for the duration of the
    /// request and we don't want references escaping it.
    pub fn list(&self) -> Vec<RepoEntry> {
        self.entries.clone()
    }

    /// Append an entry and persist. On WASM, a storage write failure
    /// leaves the in-memory list updated but returns an error so the
    /// caller can surface it — re-opening the same entry later is
    /// idempotent, so the inconsistency is recoverable.
    pub async fn insert(&mut self, entry: RepoEntry) -> Result<(), RepoIndexError> {
        if !self
            .entries
            .iter()
            .any(|e| e.local_repo == entry.local_repo)
        {
            self.entries.push(entry);
        }
        storage::save(&self.entries).await
    }
}

/// Persistence backend. WASM uses IndexedDB; native is a no-op so
/// tests run without a real storage layer.
///
/// The stored payload is the JSON-encoded `Vec<RepoEntry>` as a
/// `JsString`, not a structured JS object — skips the
/// `serde-wasm-bindgen` dependency and keeps the schema migration story
/// simple (one version field baked into the key).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod storage {
    use super::{RepoEntry, RepoIndexError};
    use idb::event::DatabaseEvent;
    use idb::{Factory, TransactionMode};
    use wasm_bindgen::JsValue;

    const DB_NAME: &str = "tonk-meta";
    const DB_VERSION: u32 = 1;
    const STORE: &str = "repo-index";
    const KEY: &str = "v1";

    fn err<E: std::fmt::Display>(e: E) -> RepoIndexError {
        RepoIndexError::Storage(format!("{e}"))
    }

    async fn open_db() -> Result<idb::Database, RepoIndexError> {
        let factory = Factory::new().map_err(err)?;
        let mut req = factory.open(DB_NAME, Some(DB_VERSION)).map_err(err)?;
        req.on_upgrade_needed(|event| {
            let db = event.database().expect("upgrade event carries a database");
            if !db.store_names().iter().any(|n| n == STORE) {
                db.create_object_store(STORE, Default::default())
                    .expect("create_object_store");
            }
        });
        req.await.map_err(err)
    }

    pub(super) async fn load() -> Option<Vec<RepoEntry>> {
        let db = open_db().await.ok()?;
        let tx = db.transaction(&[STORE], TransactionMode::ReadOnly).ok()?;
        let store = tx.object_store(STORE).ok()?;
        let value: JsValue = store.get(JsValue::from_str(KEY)).ok()?.await.ok()??;
        let json = value.as_string()?;
        serde_json::from_str(&json).ok()
    }

    pub(super) async fn save(entries: &[RepoEntry]) -> Result<(), RepoIndexError> {
        let db = open_db().await?;
        let tx = db
            .transaction(&[STORE], TransactionMode::ReadWrite)
            .map_err(err)?;
        let store = tx.object_store(STORE).map_err(err)?;
        let json = serde_json::to_string(entries)?;
        store
            .put(&JsValue::from_str(&json), Some(&JsValue::from_str(KEY)))
            .map_err(err)?
            .await
            .map_err(err)?;
        tx.commit().map_err(err)?.await.map_err(err)?;
        Ok(())
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod storage {
    use super::{RepoEntry, RepoIndexError};

    pub(super) async fn load() -> Option<Vec<RepoEntry>> {
        None
    }

    pub(super) async fn save(_entries: &[RepoEntry]) -> Result<(), RepoIndexError> {
        Ok(())
    }
}
