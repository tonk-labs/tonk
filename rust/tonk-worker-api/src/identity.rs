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
    /// `AccountSealedInbox`.
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
    /// What the worker will do once the page has mediated, echoed back
    /// with the handles so the handoff carries its own reason. Only
    /// [`WebAuthnKind::Custody`] sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<CustodyIntent>,
    /// The account's own passkey, hex credential id, when the worker
    /// knows it: the prompt is pinned to it so a browser holding several
    /// passkeys for this origin cannot answer with another account's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

/// What a custody handoff should do once it holds the handles.
///
/// The page runs the assertion and nothing else, so the work it was
/// asked for travels with the handles rather than being inferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CustodyIntent {
    /// Purge the account this passkey holds: sign `/void/customer/purge`
    /// with the recovered root, present it, and clear this device.
    PurgeAccount(AccountPurge),
    /// Delegate the account this passkey holds to a waiting process,
    /// and send the page to its callback with the grant.
    AuthorizeDevice(DeviceAuthorization),
    /// Mint a delegation from the account this passkey holds: a single
    /// `account -> audience` hop over `subject` at `command`, answered to
    /// the page that asked.
    Delegate(RootDelegation),
    /// Register an existing account as a customer of the access
    /// service.
    Enroll(Enrollment),
    /// Create an account this passkey holds, and enroll it.
    CreateAccount(AccountCreation),
    /// Open the account this passkey holds and link this browser to it.
    Login(DeviceLink),
    /// Seal the account a first passkey holds under a second one, so
    /// either can open it. Needs two ceremonies, so the handoff carries
    /// two sets of handles.
    AddPasskey(PasskeyAddition),
}

/// The delegation [`CustodyIntent::Delegate`] asks the account to make.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootDelegation {
    /// The DID the delegation is over.
    pub subject: String,
    /// The command it grants, as a path (`/` for everything).
    pub command: String,
    /// The DID it is addressed to.
    pub audience: String,
}

/// The purge [`CustodyIntent::PurgeAccount`] carries. Empty: the worker
/// already checked the plan-bound address when the command arrived, and
/// the root the passkey recovers is checked against the linked account
/// before anything is signed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPurge {}

/// The device authorization [`CustodyIntent::AuthorizeDevice`] carries:
/// what the waiting process asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthorization {
    /// The device DID the grant is addressed to.
    pub audience: String,
    /// Where the grant goes: the loopback URL a waiting CLI listens on, or
    /// the https page of a deployment signing a browser in through this one.
    pub callback: String,
    /// The name the waiting process gave itself.
    pub name: String,
    /// Account constraint from a scoped handoff, not proof of authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_account: Option<String>,
}

impl Default for CustodyIntent {
    fn default() -> Self {
        Self::Enroll(Enrollment::default())
    }
}

/// The enrollment a custody handoff should perform once it holds the
/// handles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enrollment {
    /// The address to enroll, or `None` for the account's recorded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Adding a second passkey to an account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAddition {
    /// The account the existing passkey must open, so a mismatched
    /// assertion is refused rather than sealing the wrong secret.
    pub account_did: String,
    /// The access service's `/ucan/` endpoint the custody cell
    /// resolves through.
    pub endpoint: String,
}

/// The browser a custody handoff should link to the account it opens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLink {
    /// What this browser is called in the account's device list.
    pub device_name: String,
    /// The access service's `/ucan/` endpoint the custody cell
    /// resolves through.
    pub endpoint: String,
    /// The account service's base URL.
    pub provider: String,
}

/// The account a custody handoff should bring into being.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreation {
    /// Chosen during signup, committed before enrollment sends the email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The address the account is created for.
    pub email: String,
    /// What this browser is called in the account's device list.
    pub device_name: String,
    /// The account repository's remote, so the descriptor names it.
    pub remote: String,
    /// The account service's base URL. Travels with the request
    /// because no account is linked yet, so the worker cannot look it
    /// up the way every later call does.
    pub provider: String,
    /// Browser/OS label recorded with the created passkey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_on: Option<String>,
}

/// The ceremonies a page can be asked to run.
///
/// An enum rather than a bare string so the page's listener must
/// `match` it: adding a kind then fails to compile until something
/// handles it. It was a `String` compared with `!=` once, and a kind
/// shipped with a sender and no receiver: the worker asked, the listener
/// returned early, and the page reported success with no ceremony run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebAuthnKind {
    /// See [`CUSTODY_REQUEST`].
    #[serde(rename = "custody")]
    Custody,
}

impl WebAuthnKind {
    /// The wire string this kind serializes as.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Custody => CUSTODY_REQUEST,
        }
    }
}

/// The `type` every [`WebAuthnRequest`] message carries.
pub const WEBAUTHN: &str = "webauthn";

/// Mediate a passkey so the worker can mint custody material.
///
/// The page runs one assertion and posts the two derivation handles it
/// yields; the worker does the minting and drops them. The page holds
/// no key material and builds nothing: it only supplies the gesture
/// WebAuthn requires.
pub const CUSTODY_REQUEST: &str = "custody";

#[cfg(test)]
mod tests {
    use super::*;

    /// The page and the worker are built apart and meet on this shape.
    #[dialog_common::test]
    fn it_names_a_delegation_request_on_the_wire() {
        let intent = CustodyIntent::Delegate(RootDelegation {
            subject: "did:key:zSpace".into(),
            command: "/".into(),
            audience: "did:key:zMember".into(),
        });

        let wire = serde_json::to_value(&intent).unwrap();

        assert_eq!(
            wire,
            serde_json::json!({
                "kind": "delegate",
                "subject": "did:key:zSpace",
                "command": "/",
                "audience": "did:key:zMember",
            })
        );
        assert_eq!(
            serde_json::from_value::<CustodyIntent>(wire).unwrap(),
            intent
        );
    }
}
