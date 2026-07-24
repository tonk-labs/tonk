//! D1-backed [`RevocationRegistry`](super::RevocationRegistry).
//!
//! Reads the account registry's `devices` table through the
//! `ACCOUNTS_DB` binding. Read-only by convention: the registry is
//! owned and migrated by the account service; this module must never
//! issue anything but `SELECT`.

use serde::Deserialize;
use worker::d1::D1Database;
use worker::wasm_bindgen::JsValue;

use super::{RegistryError, RevocationRegistry, revoked_query};

/// A revoked-device row, only the two join columns.
#[derive(Deserialize)]
struct RevokedRow {
    delegation_cid: String,
    device_did: String,
}

/// D1-backed registry over the accounts database.
pub struct D1RevocationRegistry(D1Database);

impl D1RevocationRegistry {
    /// Wrap the `ACCOUNTS_DB` binding.
    pub fn new(db: D1Database) -> Self {
        Self(db)
    }
}

impl RevocationRegistry for D1RevocationRegistry {
    async fn revoked_of(&self, keys: &[String]) -> Result<Vec<String>, RegistryError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let binds: Vec<JsValue> = keys.iter().map(|key| JsValue::from_str(key)).collect();
        let rows = self
            .0
            .prepare(&revoked_query(keys.len()))
            .bind(&binds)
            .map_err(|err| RegistryError(err.to_string()))?
            .all()
            .await
            .map_err(|err| RegistryError(err.to_string()))?
            .results::<RevokedRow>()
            .map_err(|err| RegistryError(err.to_string()))?;

        Ok(keys
            .iter()
            .filter(|key| {
                rows.iter()
                    .any(|row| row.delegation_cid == **key || row.device_did == **key)
            })
            .cloned()
            .collect())
    }
}
