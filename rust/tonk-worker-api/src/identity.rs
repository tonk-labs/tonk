//! Provider-neutral local-root wire types.

use serde::{Deserialize, Serialize};

/// Informational metadata recorded when Tonk creates a passkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyMetadata {
    /// Browser-reported Unix time immediately after credential creation.
    pub created_at: u64,
    /// Browser and operating-system label where creation ran.
    pub created_on: String,
}

/// Current local passkey-root state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RootStatus {
    /// This profile has no persisted local root grant.
    Missing {
        /// Current device profile DID.
        device_did: String,
    },
    /// A verified root → device grant is persisted.
    Ready {
        /// Root DID derived from the grant issuer.
        root_did: String,
        /// Current device profile DID.
        device_did: String,
        /// Opaque WebAuthn credential identifier.
        credential_id: String,
        /// CID of the stable root → device delegation.
        delegation_cid: String,
        /// Exact hex-encoded delegation bytes.
        delegation_hex: String,
        /// Creation details when this Tonk client created the passkey.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passkey: Option<PasskeyMetadata>,
        /// The account's X25519 recipient, when a ceremony on this device
        /// has recorded it. Absent means custody cannot be set up until
        /// a passkey assertion derives it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encryption_key: Option<String>,
    },
}

/// Persist a root ceremony result for the current profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRootRequest {
    /// Opaque WebAuthn credential identifier.
    pub credential_id: String,
    /// Exact hex-encoded root → device delegation bytes.
    pub delegation_hex: String,
    /// Creation details when this request follows passkey creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<PasskeyMetadata>,
    /// The account's X25519 recipient (`did:key:z6LS…`) when the
    /// ceremony held the secret, for the worker to publish as
    /// `AccountEncryptionKey`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<String>,
}

/// Service-worker message asking the originating document to run a
/// WebAuthn ceremony on the worker's behalf and answer through the
/// ordinary API. The worker has no `window`, so a passkey assertion can
/// only happen on the page that asked for the operation needing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebAuthnRequest {
    /// Fixed message discriminator: [`WEBAUTHN`].
    #[serde(rename = "type")]
    pub message_type: String,
    /// What the ceremony must produce.
    pub request: WebAuthnKind,
}

/// The ceremonies a page can be asked to run.
///
/// An enum rather than a bare string so the page's listener must
/// `match` it: adding a kind then fails to compile until something
/// handles it. It was a `String` compared with `!=` once, and
/// [`CREATE_ACCOUNT_REQUEST`] shipped with a sender and no receiver —
/// the worker asked, the listener returned early, and the dialog
/// reported success with no ceremony ever run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebAuthnKind {
    /// See [`ENCRYPTION_KEY_REQUEST`].
    #[serde(rename = "encryption-key")]
    EncryptionKey,
    /// See [`CREATE_ACCOUNT_REQUEST`].
    #[serde(rename = "create-account")]
    CreateAccount,
}

impl WebAuthnKind {
    /// The wire string this kind serializes as.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EncryptionKey => ENCRYPTION_KEY_REQUEST,
            Self::CreateAccount => CREATE_ACCOUNT_REQUEST,
        }
    }
}

/// Ask the page to link an account so a share can proceed.
///
/// Sent by the invite handler when a space cannot be shared because
/// nothing is registered. The page raises the registration UI; the
/// worker does not wait, and continues when the account facts land.
///
/// Distinct from [`WebAuthnRequest`]: that asks for one ceremony and
/// names what it must produce, whereas this asks for a whole
/// interaction and carries the space it is on behalf of, so the share
/// can be finished afterwards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkAccountRequest {
    /// Fixed message discriminator: [`LINK_ACCOUNT`].
    #[serde(rename = "type")]
    pub message_type: String,
    /// The space whose share is waiting on an account.
    pub space: String,
}

/// The `type` a [`LinkAccountRequest`] carries.
pub const LINK_ACCOUNT: &str = "link-account";

/// The `type` every [`WebAuthnRequest`] message carries.
pub const WEBAUTHN: &str = "webauthn";

/// Derive the account's encryption key from a passkey assertion and
/// save it with the root (`POST /api/identity/root` with `encryptionKey`).
/// The worker waits for that save before continuing.
pub const ENCRYPTION_KEY_REQUEST: &str = "encryption-key";

/// Create an account: the full signup ceremony, run in the top page
/// because WebAuthn needs a `window` and a user gesture and the service
/// worker has neither.
///
/// Raised by the registration form's `account/register` command. The
/// page runs the ceremony, saves the root, links the account and
/// enrolls it, and the outcome reaches every reader as facts — the
/// worker is not waiting on a response body.
pub const CREATE_ACCOUNT_REQUEST: &str = "create-account";
