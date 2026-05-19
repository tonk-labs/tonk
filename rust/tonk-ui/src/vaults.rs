//! Per-browser-profile registry of local-disk vaults.
//!
//! Each entry pairs a vault id (the opaque string used as
//! `FsAddress::id` on the worker side) with the
//! `FileSystemDirectoryHandle` the user picked, a display name, and
//! cached status. Persisted in IndexedDB so the list survives reloads;
//! the handle itself rides along through IDB's structured-cloneable
//! storage — only the permission grant doesn't survive (browsers reset
//! that on every cold load).
//!
//! Status semantics ([`VaultStatus`]):
//! - `Green`: permission queried as `granted`; sync will succeed.
//! - `Yellow`: handle known but permission is `prompt` — needs a user
//!   gesture to reconnect.
//! - `Red`: handle no longer resolves (directory moved/renamed) and
//!   the user must re-pick.
//!
//! Construction (`VaultRegistry::open`) is async because IndexedDB
//! open is async; the same registry instance is intended to be held
//! for the lifetime of the page.

use idb::{Database, DatabaseEvent, Factory, KeyPath, ObjectStoreParams, TransactionMode};
use js_sys::{Object, Reflect};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::FileSystemDirectoryHandle;

const DB_NAME: &str = "tonk-vaults";
const STORE_NAME: &str = "vaults";
const DB_VERSION: u32 = 1;

const FIELD_ID: &str = "id";
const FIELD_DISPLAY_NAME: &str = "displayName";
const FIELD_HANDLE: &str = "handle";
const FIELD_SUBJECT_DID: &str = "subjectDid";
const FIELD_LAST_SYNCED_AT: &str = "lastSyncedAt";
const FIELD_LAST_KNOWN_STATUS: &str = "lastKnownStatus";

/// Cached permission/probe state for a vault.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultStatus {
    /// Permission `granted` at last probe — sync ready.
    Green,
    /// Permission `prompt` — needs a user gesture to reconnect.
    Yellow,
    /// Handle no longer resolves (directory missing) — re-pick required.
    Red,
}

impl VaultStatus {
    fn to_storage(self) -> &'static str {
        match self {
            VaultStatus::Green => "green",
            VaultStatus::Yellow => "yellow",
            VaultStatus::Red => "red",
        }
    }

    fn from_storage(s: &str) -> Option<Self> {
        match s {
            "green" => Some(VaultStatus::Green),
            "yellow" => Some(VaultStatus::Yellow),
            "red" => Some(VaultStatus::Red),
            _ => None,
        }
    }
}

/// One entry in the registry.
#[derive(Clone)]
pub struct VaultEntry {
    /// Opaque id — used as the `FsAddress::id` on the worker side.
    /// Currently a randomly-minted UUID; the join flow associates an
    /// entry with a subject DID via `subject_did` rather than reusing
    /// it as the id, so multiple entries can coexist for the same
    /// subject (e.g. the user picked the same vault twice).
    pub id: String,
    /// User-chosen friendly name (shown in the vaults list and the
    /// reconnect prompt).
    pub display_name: String,
    /// The directory the user picked via `showDirectoryPicker()`.
    pub handle: FileSystemDirectoryHandle,
    /// Subject DID this vault holds, when known. Set when the entry
    /// is created via the join flow (where the invite supplies the
    /// DID) or when a standalone "open existing vault" verifies the
    /// directory's `credential/key/self` against an expected DID.
    /// `None` for standalone opens that haven't been associated yet.
    pub subject_did: Option<String>,
    /// Wall-clock ms since epoch of the most recent sync, or `None`
    /// if this vault has never been synced from this device.
    pub last_synced_at: Option<f64>,
    /// Last-probed status. Refreshed on boot scan and after each
    /// permission/connectivity change.
    pub last_known_status: VaultStatus,
}

impl std::fmt::Debug for VaultEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultEntry")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("handle", &"<FileSystemDirectoryHandle>")
            .field("subject_did", &self.subject_did)
            .field("last_synced_at", &self.last_synced_at)
            .field("last_known_status", &self.last_known_status)
            .finish()
    }
}

/// Errors raised by [`VaultRegistry`] operations.
#[derive(Debug, Error)]
pub enum VaultRegistryError {
    /// IndexedDB returned an error.
    #[error("IndexedDB error: {0}")]
    Idb(String),
    /// A record was missing an expected field or had the wrong shape.
    #[error("Stored vault record is malformed: {0}")]
    Malformed(String),
}

impl From<idb::Error> for VaultRegistryError {
    fn from(error: idb::Error) -> Self {
        VaultRegistryError::Idb(error.to_string())
    }
}

/// Per-browser-profile registry of FS-Access vaults.
pub struct VaultRegistry {
    db: Database,
}

impl VaultRegistry {
    /// Open (or create) the IndexedDB-backed registry.
    pub async fn open() -> Result<Self, VaultRegistryError> {
        let factory = Factory::new()?;
        let mut request = factory.open(DB_NAME, Some(DB_VERSION))?;
        request.on_upgrade_needed(|event| {
            let database = event
                .database()
                .expect("idb upgrade event missing database");
            // Ignore "already exists" — covers the case where a
            // previous version created the store at the same name.
            let mut params = ObjectStoreParams::new();
            params.key_path(Some(KeyPath::new_single(FIELD_ID)));
            let _ = database.create_object_store(STORE_NAME, params);
        });
        let db = request.await?;
        Ok(Self { db })
    }

    /// List every registered vault.
    pub async fn list(&self) -> Result<Vec<VaultEntry>, VaultRegistryError> {
        let tx = self
            .db
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)?;
        let store = tx.object_store(STORE_NAME)?;
        let raw = store.get_all(None, None)?.await?;
        tx.await?;

        let mut entries = Vec::with_capacity(raw.len());
        for value in raw {
            entries.push(decode_entry(&value)?);
        }
        Ok(entries)
    }

    /// Find an entry whose `subject_did` matches `did`. Returns the
    /// first match. Linear scan over the full registry — fine at the
    /// current scale (a handful of entries per device).
    pub async fn find_by_subject(
        &self,
        did: &str,
    ) -> Result<Option<VaultEntry>, VaultRegistryError> {
        for entry in self.list().await? {
            if entry.subject_did.as_deref() == Some(did) {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    /// Fetch a single entry by id.
    pub async fn get(&self, id: &str) -> Result<Option<VaultEntry>, VaultRegistryError> {
        let tx = self
            .db
            .transaction(&[STORE_NAME], TransactionMode::ReadOnly)?;
        let store = tx.object_store(STORE_NAME)?;
        let value = store.get(JsValue::from_str(id))?.await?;
        tx.await?;
        match value {
            Some(value) if !value.is_undefined() && !value.is_null() => {
                Ok(Some(decode_entry(&value)?))
            }
            _ => Ok(None),
        }
    }

    /// Insert or replace an entry.
    pub async fn put(&self, entry: &VaultEntry) -> Result<(), VaultRegistryError> {
        let tx = self
            .db
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)?;
        let store = tx.object_store(STORE_NAME)?;
        let value = encode_entry(entry)?;
        store.put(&value, None)?.await?;
        tx.commit()?.await?;
        Ok(())
    }

    /// Remove the entry with the given id. Returns `true` if it
    /// existed.
    pub async fn remove(&self, id: &str) -> Result<bool, VaultRegistryError> {
        let existed = self.get(id).await?.is_some();
        if !existed {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction(&[STORE_NAME], TransactionMode::ReadWrite)?;
        let store = tx.object_store(STORE_NAME)?;
        store.delete(JsValue::from_str(id))?.await?;
        tx.commit()?.await?;
        Ok(true)
    }

    /// Update an entry's cached status (boot scan, reconnect, etc.).
    /// No-op if the entry is missing.
    pub async fn update_status(
        &self,
        id: &str,
        status: VaultStatus,
    ) -> Result<(), VaultRegistryError> {
        let Some(mut entry) = self.get(id).await? else {
            return Ok(());
        };
        entry.last_known_status = status;
        self.put(&entry).await
    }

    /// Update an entry's last-synced timestamp (ms since epoch).
    /// No-op if the entry is missing.
    pub async fn touch_last_synced(
        &self,
        id: &str,
        when_ms: f64,
    ) -> Result<(), VaultRegistryError> {
        let Some(mut entry) = self.get(id).await? else {
            return Ok(());
        };
        entry.last_synced_at = Some(when_ms);
        self.put(&entry).await
    }
}

fn encode_entry(entry: &VaultEntry) -> Result<JsValue, VaultRegistryError> {
    let obj = Object::new();
    set(&obj, FIELD_ID, &JsValue::from_str(&entry.id))?;
    set(
        &obj,
        FIELD_DISPLAY_NAME,
        &JsValue::from_str(&entry.display_name),
    )?;
    set(&obj, FIELD_HANDLE, entry.handle.as_ref())?;
    set(
        &obj,
        FIELD_SUBJECT_DID,
        &match &entry.subject_did {
            Some(did) => JsValue::from_str(did),
            None => JsValue::NULL,
        },
    )?;
    set(
        &obj,
        FIELD_LAST_SYNCED_AT,
        &match entry.last_synced_at {
            Some(t) => JsValue::from_f64(t),
            None => JsValue::NULL,
        },
    )?;
    set(
        &obj,
        FIELD_LAST_KNOWN_STATUS,
        &JsValue::from_str(entry.last_known_status.to_storage()),
    )?;
    Ok(obj.into())
}

fn decode_entry(value: &JsValue) -> Result<VaultEntry, VaultRegistryError> {
    let id = get_string(value, FIELD_ID)?;
    let display_name = get_string(value, FIELD_DISPLAY_NAME)?;
    let handle_value = Reflect::get(value, &JsValue::from_str(FIELD_HANDLE))
        .map_err(|_| VaultRegistryError::Malformed(format!("missing {FIELD_HANDLE}")))?;
    let handle: FileSystemDirectoryHandle = handle_value.dyn_into().map_err(|_| {
        VaultRegistryError::Malformed(format!("{FIELD_HANDLE} is not a FileSystemDirectoryHandle"))
    })?;
    let subject_did = Reflect::get(value, &JsValue::from_str(FIELD_SUBJECT_DID))
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_string()
            }
        });
    let last_synced_at = Reflect::get(value, &JsValue::from_str(FIELD_LAST_SYNCED_AT))
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_f64()
            }
        });
    let status_str = get_string(value, FIELD_LAST_KNOWN_STATUS)?;
    let last_known_status = VaultStatus::from_storage(&status_str).ok_or_else(|| {
        VaultRegistryError::Malformed(format!("unknown status value '{status_str}'"))
    })?;

    Ok(VaultEntry {
        id,
        display_name,
        handle,
        subject_did,
        last_synced_at,
        last_known_status,
    })
}

fn set(obj: &Object, key: &str, value: &JsValue) -> Result<(), VaultRegistryError> {
    Reflect::set(obj.as_ref(), &JsValue::from_str(key), value)
        .map_err(|_| VaultRegistryError::Malformed(format!("could not set {key}")))?;
    Ok(())
}

fn get_string(value: &JsValue, key: &str) -> Result<String, VaultRegistryError> {
    Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
        .ok_or_else(|| VaultRegistryError::Malformed(format!("missing or non-string {key}")))
}
