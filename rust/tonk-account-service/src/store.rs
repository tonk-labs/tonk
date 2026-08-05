//! Storage abstraction for the account service.
//!
//! The ordered SQL files under `migrations/` are the schema's source of
//! truth: Wrangler applies them to Cloudflare D1 and
//! [`sqlite::SqliteStore`] applies the same sequence natively. Both
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
    /// Exact root-signed account repository descriptor, when established.
    pub repository_descriptor: Option<Vec<u8>>,
    /// Passkey ceremony time, when Tonk recorded it.
    pub passkey_created_at: Option<u64>,
    /// Browser and operating system where the passkey ceremony ran.
    pub passkey_created_on: Option<String>,
    /// Creation time, as a unix timestamp in seconds.
    pub created_at: u64,
}

/// Facts Tonk recorded during a passkey creation ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyMetadata {
    /// Ceremony time, as a unix timestamp in seconds.
    pub created_at: u64,
    /// Browser and operating system where the ceremony ran.
    pub created_on: String,
}

/// A device delegated under an account's root DID.
#[derive(Debug, Clone)]
pub struct Device {
    /// Attachment row id; zero for a row not inserted yet.
    pub id: i64,
    /// The owning account's row id.
    pub account_id: i64,
    /// The device's DID.
    pub device_did: String,
    /// Random identifier for this exact attachment generation.
    pub attachment_id: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Exact public delegation path bytes, hex-encoded. Empty only for a
    /// legacy row created before migration 0003 retained these bytes.
    pub delegation_hex: String,
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
    /// Random identifier for this exact attachment generation.
    pub attachment_id: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Exact public delegation path bytes, hex-encoded.
    pub delegation_hex: String,
    /// Human-readable device name.
    pub name: String,
}

/// Lifecycle state of a [`Device`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    /// The device's delegation is valid and usable.
    Active,
    /// The attachment was hidden by a signed device detach intent.
    Detached,
    /// The device's delegation has been revoked.
    Revoked,
}

impl DeviceStatus {
    /// The column value for this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceStatus::Active => "active",
            DeviceStatus::Detached => "detached",
            DeviceStatus::Revoked => "revoked",
        }
    }

    /// Parse a column value; unknown values are an internal error.
    pub fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "active" => Ok(DeviceStatus::Active),
            "detached" => Ok(DeviceStatus::Detached),
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
    /// Account selected by browser completion.
    pub account_id: Option<i64>,
    /// Service-generated attachment generation.
    pub attachment_id: Option<String>,
    /// CID of the completed root-to-device grant.
    pub delegation_cid: Option<String>,
    /// Completed root-to-device delegation.
    pub delegation_hex: Option<String>,
    /// Account repository descriptor copied alongside the delegation.
    pub descriptor_hex: Option<String>,
    /// Creation time, as unix seconds.
    pub created_at: u64,
    /// Expiry time, as unix seconds.
    pub expires_at: u64,
    /// Browser completion time.
    pub completed_at: Option<u64>,
    /// First consumption time. Consumption remains replayable.
    pub consumed_at: Option<u64>,
    /// Successful activation time.
    pub activated_at: Option<u64>,
    /// Signed cancellation time.
    pub cancelled_at: Option<u64>,
}

/// Material persisted when a pending handoff is completed.
#[derive(Debug, Clone, Copy)]
pub struct LinkCompletion<'a> {
    /// Hash identifying the pending handoff.
    pub token_hash: &'a str,
    /// Account selected by browser completion.
    pub account_id: i64,
    /// Service-generated attachment generation.
    pub attachment_id: &'a str,
    /// CID of the completed root-to-device grant.
    pub delegation_cid: &'a str,
    /// Completed root-to-device delegation.
    pub delegation_hex: &'a str,
    /// Account repository descriptor copied alongside the delegation.
    pub descriptor_hex: &'a str,
    /// Browser completion time, as unix seconds.
    pub now: u64,
}

/// Result of idempotently activating a completed handoff.
#[derive(Debug, Clone)]
pub enum ActivateOutcome {
    /// The attachment is active (newly inserted or replayed).
    Active(Device),
    /// Another active attachment owns this device DID.
    ActiveDeviceConflict,
    /// This delegation CID was previously revoked.
    RevokedDelegation,
    /// The completed attachment was cancelled by detach.
    Cancelled,
    /// No matching completed handoff exists.
    Unknown,
}

/// Storage-level result of detaching one exact generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetachStoreOutcome {
    /// An active row changed to detached.
    Detached,
    /// The exact row was already detached.
    AlreadyDetached,
    /// A completed link was cancelled before activation.
    CancelledPendingActivation,
    /// Another active generation supersedes this one.
    Superseded,
    /// The exact row is permanently revoked.
    Revoked,
    /// Neither a device row nor completed link has this ID.
    UnknownAttachment,
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

    /// Look up an account by email address. Callers are responsible for
    /// lowercasing, as the column stores lowercased addresses.
    ///
    /// Reserved for explaining a uniqueness conflict to a caller who has
    /// already proven control of the address, never for answering
    /// "is this email registered?" before that proof — see
    /// [`crate::core::accounts::create_account`].
    async fn account_by_email(&self, email: &str) -> Result<Option<Account>, StoreError>;

    /// Atomically create a new account and register its first device.
    ///
    /// Either both rows are created or neither is: a conflict on the
    /// email, root DID, or account-scoped device registration rolls back
    /// the whole operation, so a device-registration failure can never
    /// strand an account with zero devices. Returns `StoreError::Conflict`
    /// in that case.
    async fn create_account_with_device(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        repository_descriptor: &[u8],
        passkey: Option<&PasskeyMetadata>,
        device: &NewDevice,
        created_at: u64,
    ) -> Result<i64, StoreError>;

    /// Establish one immutable repository descriptor and return `(winner, created)`.
    async fn establish_repository_descriptor(
        &self,
        account_id: i64,
        candidate: &[u8],
    ) -> Result<(Vec<u8>, bool), StoreError>;

    /// Register a device under an account. Returns `StoreError::Conflict`
    /// if the device DID is already registered under that account.
    async fn insert_device(&self, device: &Device) -> Result<(), StoreError>;

    /// List all devices registered under an account.
    async fn devices(&self, account_id: i64) -> Result<Vec<Device>, StoreError>;

    /// Look up the active generation for a device DID, globally.
    async fn active_device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError>;

    /// Look up the actionable generation for an account and DID (active first,
    /// otherwise newest history).
    async fn device_for_account(
        &self,
        account_id: i64,
        device_did: &str,
    ) -> Result<Option<Device>, StoreError>;

    /// Look up one exact attachment generation.
    async fn attachment(&self, attachment_id: &str) -> Result<Option<Device>, StoreError>;

    /// Mark the active generation for an account/device DID revoked.
    async fn revoke_device(&self, account_id: i64, device_did: &str) -> Result<bool, StoreError>;

    /// Project a verified revocation onto the row matching its delegation CID.
    async fn revoke_device_by_cid(
        &self,
        account_id: i64,
        delegation_cid: &str,
    ) -> Result<bool, StoreError>;

    /// Create a pending CLI browser handoff.
    async fn put_link(&self, link: &LinkRequest) -> Result<(), StoreError>;

    /// Look up a handoff by its secret hash.
    async fn link(&self, token_hash: &str) -> Result<Option<LinkRequest>, StoreError>;

    /// Durably complete a handoff without activating its attachment.
    async fn complete_link(&self, completion: &LinkCompletion<'_>) -> Result<bool, StoreError>;

    /// Retrieve a completed handoff, recording first consumption while
    /// preserving the result for crash-safe replay.
    async fn consume_link(
        &self,
        token_hash: &str,
        now: u64,
    ) -> Result<Option<LinkRequest>, StoreError>;

    /// Look up a completed handoff by attachment generation.
    async fn completed_link_by_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<Option<LinkRequest>, StoreError>;

    /// Idempotently insert the active device row for a completed handoff.
    async fn activate_completed_link(
        &self,
        token_hash: &str,
        attachment_id: &str,
        now: u64,
    ) -> Result<ActivateOutcome, StoreError>;

    /// Detach or cancel exactly one attachment generation.
    async fn detach_attachment(
        &self,
        attachment_id: &str,
        now: u64,
    ) -> Result<DetachStoreOutcome, StoreError>;
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

/// SQL: insert a new account with its repository descriptor.
pub const INSERT_ACCOUNT_WITH_DESCRIPTOR: &str = "INSERT INTO accounts \
    (email, root_did, credential_id, repository_descriptor, passkey_created_at, passkey_created_on, created_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

/// SQL: look up an account by root DID.
pub const SELECT_ACCOUNT_BY_ROOT: &str = "SELECT id, email, root_did, credential_id, \
    repository_descriptor, passkey_created_at, passkey_created_on, created_at FROM accounts WHERE root_did = ?1";

/// SQL: install a descriptor only while the account has none.
pub const ESTABLISH_REPOSITORY_DESCRIPTOR: &str = "UPDATE accounts \
    SET repository_descriptor = ?2 WHERE id = ?1 AND repository_descriptor IS NULL \
    RETURNING repository_descriptor";

/// SQL: read the established descriptor winner.
pub const SELECT_REPOSITORY_DESCRIPTOR: &str =
    "SELECT repository_descriptor FROM accounts WHERE id = ?1";

/// SQL: look up an account by email address.
pub const SELECT_ACCOUNT_BY_EMAIL: &str = "SELECT id, email, root_did, credential_id, \
    repository_descriptor, passkey_created_at, passkey_created_on, created_at FROM accounts WHERE email = ?1";

/// SQL: register a device under an account.
pub const INSERT_DEVICE: &str = "INSERT INTO devices (account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

/// SQL: register a device under the account just created by a preceding
/// `INSERT_ACCOUNT` statement in the same batch/transaction on this
/// connection, via SQLite's `last_insert_rowid()`. Used by
/// [`Store::create_account_with_device`]'s D1 batch, where the new
/// account's id is not otherwise known until the batch commits. Always
/// registers the device as `active`.
pub const INSERT_DEVICE_FOR_NEW_ACCOUNT: &str = "INSERT INTO devices (account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at) \
     VALUES (last_insert_rowid(), ?1, ?2, ?3, ?4, ?5, 'active', ?6)";

/// SQL: list the devices registered under an account.
pub const SELECT_DEVICES_BY_ACCOUNT: &str = "SELECT id, account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at \
     FROM devices WHERE account_id = ?1 ORDER BY created_at DESC, id DESC";

/// SQL: look up the globally active generation for a device DID.
pub const SELECT_ACTIVE_DEVICE_BY_DID: &str = "SELECT id, account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at \
     FROM devices WHERE device_did = ?1 AND status = 'active'";

/// SQL: look up the actionable generation by account and DID.
pub const SELECT_DEVICE_FOR_ACCOUNT: &str = "SELECT id, account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at \
     FROM devices WHERE account_id = ?1 AND device_did = ?2 ORDER BY status = 'active' DESC, created_at DESC, id DESC LIMIT 1";

/// SQL: look up one exact attachment generation.
pub const SELECT_ATTACHMENT: &str = "SELECT id, account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at \
     FROM devices WHERE attachment_id = ?1";

/// SQL: mark a device as revoked.
pub const UPDATE_DEVICE_REVOKE: &str = "UPDATE devices SET status = 'revoked' WHERE account_id = ?1 AND device_did = ?2 AND status = 'active'";

/// SQL: project revocation by the exact registered delegation CID.
pub const UPDATE_DEVICE_REVOKE_BY_CID: &str =
    "UPDATE devices SET status = 'revoked' WHERE account_id = ?1 AND delegation_cid = ?2";

/// SQL: create a pending browser handoff.
pub const INSERT_LINK: &str = "INSERT INTO link_requests \
    (token_hash, device_did, device_name, delegation_hex, descriptor_hex, created_at, expires_at, consumed_at, account_id, attachment_id, delegation_cid, completed_at, activated_at, cancelled_at) \
    VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL)";

/// SQL: common link projection.
pub const LINK_COLUMNS: &str = "token_hash, device_did, device_name, account_id, attachment_id, delegation_cid, delegation_hex, descriptor_hex, created_at, expires_at, completed_at, consumed_at, activated_at, cancelled_at";

/// SQL: load a browser handoff by token hash.
pub const SELECT_LINK: &str = "SELECT token_hash, device_did, device_name, account_id, attachment_id, delegation_cid, delegation_hex, descriptor_hex, created_at, expires_at, completed_at, consumed_at, activated_at, cancelled_at FROM link_requests WHERE token_hash = ?1";

/// SQL: load a completed handoff by attachment generation.
pub const SELECT_LINK_BY_ATTACHMENT: &str = "SELECT token_hash, device_did, device_name, account_id, attachment_id, delegation_cid, delegation_hex, descriptor_hex, created_at, expires_at, completed_at, consumed_at, activated_at, cancelled_at FROM link_requests WHERE attachment_id = ?1";

/// SQL: attach recoverable completion material without activating a device.
pub const COMPLETE_LINK: &str = "UPDATE link_requests SET account_id = ?1, attachment_id = ?2, delegation_cid = ?3, delegation_hex = ?4, descriptor_hex = ?5, completed_at = ?6, expires_at = ?7 WHERE token_hash = ?8 AND delegation_hex IS NULL AND descriptor_hex IS NULL AND cancelled_at IS NULL AND expires_at >= ?9";

/// SQL: record first consumption while returning the replayable row.
pub const CONSUME_LINK: &str = "UPDATE link_requests SET consumed_at = COALESCE(consumed_at, ?1) WHERE token_hash = ?2 AND delegation_hex IS NOT NULL AND descriptor_hex IS NOT NULL AND activated_at IS NULL AND cancelled_at IS NULL AND expires_at >= ?3 RETURNING token_hash, device_did, device_name, account_id, attachment_id, delegation_cid, delegation_hex, descriptor_hex, created_at, expires_at, completed_at, consumed_at, activated_at, cancelled_at";

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod sqlite;

#[cfg(target_arch = "wasm32")]
pub mod d1;
