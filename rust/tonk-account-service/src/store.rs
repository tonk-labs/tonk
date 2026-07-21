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

/// SQL: list the devices registered under an account.
pub const SELECT_DEVICES_BY_ACCOUNT: &str = "SELECT account_id, device_did, delegation_cid, name, status, created_at \
     FROM devices WHERE account_id = ?1";

/// SQL: look up a device by its DID.
pub const SELECT_DEVICE_BY_DID: &str = "SELECT account_id, device_did, delegation_cid, name, status, created_at \
     FROM devices WHERE device_did = ?1";

/// SQL: mark a device as revoked.
pub const UPDATE_DEVICE_REVOKE: &str =
    "UPDATE devices SET status = 'revoked' WHERE account_id = ?1 AND device_did = ?2";

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod sqlite;

#[cfg(target_arch = "wasm32")]
pub mod d1;
