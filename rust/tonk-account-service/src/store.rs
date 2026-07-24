//! Storage abstraction for the account service.
//!
//! The schema in `migrations/0001_init.sql` is the single source of
//! truth: it is applied to Cloudflare D1 by wrangler and, natively, to
//! an in-memory `rusqlite` connection via [`sqlite::SqliteStore`]. Both
//! backends implement [`Store`], so ceremony logic elsewhere in this
//! crate is written once, generically over the trait.

/// A registered account: a verified email bound to a root DID.
#[derive(Debug, Clone)]
pub struct Account {
    /// Row id.
    pub id: i64,
    /// Verified email address.
    pub email: String,
    /// The account's root DID.
    pub root_did: String,
    /// Opaque identifier for the passkey credential used to create the
    /// account.
    pub credential_id: String,
    /// Creation time, as a unix timestamp in seconds.
    pub created_at: u64,
}

/// A device delegated under an account's root DID.
#[derive(Debug, Clone)]
pub struct Device {
    /// The owning account's row id.
    pub account_id: i64,
    /// The device's DID.
    pub device_did: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Human-readable device name.
    pub name: String,
    /// Whether the device is currently active or has been revoked.
    pub status: DeviceStatus,
    /// Creation time, as a unix timestamp in seconds.
    pub created_at: u64,
}

/// A device to register as part of atomically creating an account, before
/// the owning account's row id is known. Used by
/// [`Store::create_account_with_device`]; the device is always registered
/// [`DeviceStatus::Active`].
#[derive(Debug, Clone)]
pub struct NewDevice {
    /// The device's DID.
    pub device_did: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Human-readable device name.
    pub name: String,
}

/// Lifecycle state of a [`Device`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    /// The device's delegation is valid and usable.
    Active,
    /// The device's delegation has been revoked.
    Revoked,
}

impl DeviceStatus {
    /// The column value for this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceStatus::Active => "active",
            DeviceStatus::Revoked => "revoked",
        }
    }

    /// Parse a column value; unknown values are an internal error.
    pub fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "active" => Ok(DeviceStatus::Active),
            "revoked" => Ok(DeviceStatus::Revoked),
            other => Err(StoreError::Internal(format!(
                "unknown device status: {other}"
            ))),
        }
    }
}

/// A pending email verification code.
#[derive(Debug, Clone)]
pub struct CodeRow {
    /// The email address the code was issued to.
    pub email: String,
    /// Hash of the code (never the code itself).
    pub code_hash: String,
    /// Issue time, as a unix timestamp in seconds.
    pub created_at: u64,
    /// Expiry time, as a unix timestamp in seconds.
    pub expires_at: u64,
    /// Number of verification attempts made against this code.
    pub attempts: u32,
}

/// A short-lived browser handoff requested by a native CLI profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequest {
    /// BLAKE3 hash of the bearer secret; the raw secret is never stored.
    pub token_hash: String,
    /// CLI profile DID that will receive the root delegation.
    pub device_did: String,
    /// Human-readable CLI device name.
    pub device_name: String,
    /// Completed root-to-device delegation, until it is consumed once.
    pub delegation_hex: Option<String>,
    /// Creation time, as unix seconds.
    pub created_at: u64,
    /// Expiry time, as unix seconds.
    pub expires_at: u64,
    /// Consumption time, if the CLI has retrieved the delegation.
    pub consumed_at: Option<u64>,
}

/// Errors surfaced by a [`Store`] implementation.
#[derive(Debug)]
pub enum StoreError {
    /// The operation would violate a uniqueness constraint.
    Conflict(String),
    /// An unexpected storage failure.
    Internal(String),
}

/// Storage backend for accounts, devices, and email verification codes.
///
/// Implementations back this with either Cloudflare D1 (production) or
/// an in-memory `rusqlite` connection (tests), both built from the same
/// schema. Methods are plain `async fn`: callers are always generic
/// over `Store`, never `dyn Store`.
#[allow(async_fn_in_trait)]
pub trait Store {
    /// Look up the pending code for an email, if any.
    async fn code(&self, email: &str) -> Result<Option<CodeRow>, StoreError>;

    /// Insert or replace the pending code for an email, resetting
    /// `attempts` to 0.
    async fn put_code(&self, row: &CodeRow) -> Result<(), StoreError>;

    /// Increment the attempt counter for an email's pending code.
    async fn bump_attempts(&self, email: &str) -> Result<(), StoreError>;

    /// Remove the pending code for an email.
    async fn delete_code(&self, email: &str) -> Result<(), StoreError>;

    /// Create a new account. Returns `StoreError::Conflict` if the
    /// email or root DID is already registered.
    async fn create_account(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        created_at: u64,
    ) -> Result<i64, StoreError>;

    /// Look up an account by root DID.
    async fn account_by_root(&self, root_did: &str) -> Result<Option<Account>, StoreError>;

    /// Atomically create a new account and register its first device.
    ///
    /// Either both rows are created or neither is: a conflict on the
    /// email, root DID, *or* the device DID rolls back the whole
    /// operation, so a device-registration failure can never strand an
    /// account with zero devices. Returns `StoreError::Conflict` in that
    /// case.
    async fn create_account_with_device(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        device: &NewDevice,
        created_at: u64,
    ) -> Result<i64, StoreError>;

    /// Register a device under an account. Returns `StoreError::Conflict`
    /// if the device DID is already registered.
    async fn insert_device(&self, device: &Device) -> Result<(), StoreError>;

    /// List all devices registered under an account.
    async fn devices(&self, account_id: i64) -> Result<Vec<Device>, StoreError>;

    /// Look up a device by its DID.
    async fn device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError>;

    /// Mark a device as revoked. Returns `false` if no matching device
    /// was found.
    async fn revoke_device(&self, account_id: i64, device_did: &str) -> Result<bool, StoreError>;

    /// Flip the account's root DID and passkey credential in one
    /// statement. Returns `StoreError::Conflict` if the new root DID is
    /// already registered to any account.
    async fn rotate_root(
        &self,
        account_id: i64,
        new_root_did: &str,
        new_credential_id: &str,
    ) -> Result<(), StoreError>;

    /// Repoint one device row at a fresh delegation CID. Returns `false`
    /// if no matching device was found.
    async fn update_device_delegation(
        &self,
        account_id: i64,
        device_did: &str,
        delegation_cid: &str,
    ) -> Result<bool, StoreError>;

    /// Create a pending CLI browser handoff.
    async fn put_link(&self, link: &LinkRequest) -> Result<(), StoreError>;

    /// Look up a handoff by its secret hash.
    async fn link(&self, token_hash: &str) -> Result<Option<LinkRequest>, StoreError>;

    /// Atomically register a device and complete its pending handoff.
    async fn complete_link(
        &self,
        token_hash: &str,
        device: &Device,
        delegation_hex: &str,
        now: u64,
    ) -> Result<bool, StoreError>;

    /// Atomically retrieve and consume a completed handoff once.
    async fn consume_link(&self, token_hash: &str, now: u64) -> Result<Option<String>, StoreError>;
}

/// SQL: look up the pending code row for an email.
pub const SELECT_CODE: &str =
    "SELECT email, code_hash, created_at, expires_at, attempts FROM email_codes WHERE email = ?1";

/// SQL: insert a fresh code for an email, or replace an existing one and
/// reset its attempt counter.
pub const UPSERT_CODE: &str = "INSERT INTO email_codes (email, code_hash, created_at, expires_at, attempts) \
     VALUES (?1, ?2, ?3, ?4, 0) \
     ON CONFLICT(email) DO UPDATE SET \
        code_hash = excluded.code_hash, \
        created_at = excluded.created_at, \
        expires_at = excluded.expires_at, \
        attempts = 0";

/// SQL: increment the attempt counter for an email's pending code.
pub const BUMP_ATTEMPTS: &str = "UPDATE email_codes SET attempts = attempts + 1 WHERE email = ?1";

/// SQL: remove the pending code for an email.
pub const DELETE_CODE: &str = "DELETE FROM email_codes WHERE email = ?1";

/// SQL: insert a new account.
pub const INSERT_ACCOUNT: &str =
    "INSERT INTO accounts (email, root_did, credential_id, created_at) VALUES (?1, ?2, ?3, ?4)";

/// SQL: look up an account by root DID.
pub const SELECT_ACCOUNT_BY_ROOT: &str =
    "SELECT id, email, root_did, credential_id, created_at FROM accounts WHERE root_did = ?1";

/// SQL: register a device under an account.
pub const INSERT_DEVICE: &str = "INSERT INTO devices (account_id, device_did, delegation_cid, name, status, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// SQL: register a device under the account just created by a preceding
/// `INSERT_ACCOUNT` statement in the same batch/transaction on this
/// connection, via SQLite's `last_insert_rowid()`. Used by
/// [`Store::create_account_with_device`]'s D1 batch, where the new
/// account's id is not otherwise known until the batch commits. Always
/// registers the device as `active`.
pub const INSERT_DEVICE_FOR_NEW_ACCOUNT: &str = "INSERT INTO devices (account_id, device_did, delegation_cid, name, status, created_at) \
     VALUES (last_insert_rowid(), ?1, ?2, ?3, 'active', ?4)";

/// SQL: list the devices registered under an account.
pub const SELECT_DEVICES_BY_ACCOUNT: &str = "SELECT account_id, device_did, delegation_cid, name, status, created_at \
     FROM devices WHERE account_id = ?1";

/// SQL: look up a device by its DID.
pub const SELECT_DEVICE_BY_DID: &str = "SELECT account_id, device_did, delegation_cid, name, status, created_at \
     FROM devices WHERE device_did = ?1";

/// SQL: mark a device as revoked.
pub const UPDATE_DEVICE_REVOKE: &str =
    "UPDATE devices SET status = 'revoked' WHERE account_id = ?1 AND device_did = ?2";

/// SQL: flip an account's root DID and passkey credential.
pub const UPDATE_ACCOUNT_ROOT: &str =
    "UPDATE accounts SET root_did = ?2, credential_id = ?3 WHERE id = ?1";

/// SQL: repoint one device row at a fresh delegation.
pub const UPDATE_DEVICE_DELEGATION: &str =
    "UPDATE devices SET delegation_cid = ?3 WHERE account_id = ?1 AND device_did = ?2";

/// SQL: create a pending browser handoff.
pub const INSERT_LINK: &str = "INSERT INTO link_requests \
    (token_hash, device_did, device_name, delegation_hex, created_at, expires_at, consumed_at) \
    VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL)";

/// SQL: load a browser handoff by token hash.
pub const SELECT_LINK: &str = "SELECT token_hash, device_did, device_name, delegation_hex, \
    created_at, expires_at, consumed_at FROM link_requests WHERE token_hash = ?1";

/// SQL: insert a handoff device only while its request is pending and live.
pub const INSERT_DEVICE_FOR_LINK: &str = "INSERT INTO devices \
    (account_id, device_did, delegation_cid, name, status, created_at) \
    SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS (SELECT 1 FROM link_requests \
    WHERE token_hash = ?7 AND delegation_hex IS NULL AND consumed_at IS NULL AND expires_at >= ?8)";

/// SQL: attach the delegation to a live, still-pending handoff.
pub const COMPLETE_LINK: &str = "UPDATE link_requests SET delegation_hex = ?1 \
    WHERE token_hash = ?2 AND delegation_hex IS NULL AND consumed_at IS NULL AND expires_at >= ?3";

/// SQL: retrieve a completed handoff and mark it consumed in one statement.
pub const CONSUME_LINK: &str = "UPDATE link_requests SET consumed_at = ?1 \
    WHERE token_hash = ?2 AND delegation_hex IS NOT NULL AND consumed_at IS NULL AND expires_at >= ?3 \
    RETURNING delegation_hex";

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod sqlite;

#[cfg(target_arch = "wasm32")]
pub mod d1;
