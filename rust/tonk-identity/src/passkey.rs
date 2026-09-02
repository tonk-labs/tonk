//! WebAuthn passkey ceremonies.
//!
//! Window-context only: `navigator.credentials` does not exist in
//! workers, so these run from the page main thread, inside a user
//! gesture.

use crate::envelope::{CUSTODY_KEK_CONTEXT, CUSTODY_KEY_CONTEXT};
use anyhow::{Context, Result, anyhow};
use js_sys::{Array, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AuthenticationExtensionsClientInputs, AuthenticationExtensionsPrfInputs,
    AuthenticationExtensionsPrfValues, AuthenticatorSelectionCriteria, CredentialCreationOptions,
    CredentialRequestOptions, CredentialsContainer, PublicKeyCredential,
    PublicKeyCredentialCreationOptions, PublicKeyCredentialDescriptor,
    PublicKeyCredentialParameters, PublicKeyCredentialRequestOptions, PublicKeyCredentialRpEntity,
    PublicKeyCredentialType, PublicKeyCredentialUserEntity, UserVerificationRequirement,
};
use zeroize::Zeroizing;

/// COSE algorithms offered at registration: EdDSA, ES256, RS256. The
/// credential's own key is never used directly — only its PRF — so this
/// list is purely for authenticator compatibility.
const COSE_ALGORITHMS: [i32; 3] = [-8, -7, -257];

/// Both custody PRF outputs from one assertion.
pub struct CustodyEvaluation {
    /// The output at [`CUSTODY_KEY_CONTEXT`]; seeds the custody keypair.
    pub key: Zeroizing<[u8; 32]>,
    /// The output at [`CUSTODY_KEK_CONTEXT`]; derives the wrapping KEK.
    pub kek: Zeroizing<[u8; 32]>,
}

/// A custody passkey ceremony's result: the raw credential id, plus
/// both PRF outputs when the platform evaluated PRF during creation
/// (some only do so on a follow-up assertion).
pub struct CustodyCredential {
    /// Raw credential id, as registered with the authenticator.
    pub id: Vec<u8>,
    /// Both custody PRF outputs, when the platform returned them.
    pub evaluation: Option<CustodyEvaluation>,
}

/// Why a passkey ceremony did not produce a credential.
///
/// The browser answers with a `DOMException` whose `name` is the whole
/// story — `NotAllowedError` for a cancelled prompt, `InvalidStateError`
/// for a credential this authenticator already holds — and a caller
/// wanting to distinguish them should not be matching on prose. The
/// name becomes a variant; what the browser actually said is kept
/// alongside it, because the name alone rarely explains the failure.
#[derive(Debug, thiserror::Error)]
#[error("{context}: {detail}")]
pub struct CeremonyError {
    /// Which ceremony was refused.
    pub context: String,
    /// Why, as far as the browser named it.
    pub reason: CeremonyRefusal,
    /// What the browser said, verbatim.
    pub detail: String,
}

/// The `DOMException` name a ceremony was refused with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyRefusal {
    /// The user dismissed the prompt, or it timed out. The ordinary way
    /// a ceremony ends when someone changes their mind.
    NotAllowed,
    /// This authenticator already holds a credential the request
    /// excluded. Creation only.
    InvalidState,
    /// The request named something this browser or authenticator does
    /// not implement.
    NotSupported,
    /// The origin may not act for this relying party — usually a
    /// mismatched `rp.id` or an insecure context.
    Security,
    /// The ceremony ran and the authenticator evaluated no PRF, so this
    /// platform cannot hold custody at all.
    NoPrf,
    /// Anything else the browser reported.
    Other,
}

impl CeremonyRefusal {
    /// Classify a DOM exception name without depending on its human-readable
    /// message. Unknown names deliberately collapse to [`Self::Other`].
    pub fn from_name(name: &str) -> Self {
        match name {
            "NotAllowedError" => Self::NotAllowed,
            "InvalidStateError" => Self::InvalidState,
            "NotSupportedError" => Self::NotSupported,
            "SecurityError" => Self::Security,
            "NoPrfError" => Self::NoPrf,
            _ => Self::Other,
        }
    }

    /// The `DOMException` name this refusal came back as, for handing
    /// across the JS boundary.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAllowed => "NotAllowedError",
            Self::InvalidState => "InvalidStateError",
            Self::NotSupported => "NotSupportedError",
            Self::Security => "SecurityError",
            Self::NoPrf => "NoPrfError",
            Self::Other => "Error",
        }
    }
}

fn ceremony_error(context: &str, value: JsValue) -> anyhow::Error {
    let property = |name: &str| {
        Reflect::get(&value, &name.into())
            .ok()
            .and_then(|value| value.as_string())
            .filter(|value| !value.trim().is_empty())
    };
    let detail = if let Some(message) = value.as_string() {
        message
    } else {
        match (property("name"), property("message")) {
            (Some(name), Some(message)) => format!("{name}: {message}"),
            (Some(name), None) => name,
            (None, Some(message)) => message,
            (None, None) => "unknown browser error".to_string(),
        }
    };
    let detail: String = detail.chars().take(512).collect();
    let reason = property("name")
        .as_deref()
        .map(CeremonyRefusal::from_name)
        .unwrap_or(CeremonyRefusal::Other);
    CeremonyError {
        context: context.to_string(),
        reason,
        detail,
    }
    .into()
}

fn credentials() -> Result<CredentialsContainer> {
    Ok(web_sys::window()
        .context("no window: passkey ceremonies are window-only")?
        .navigator()
        .credentials())
}

/// Extension inputs requesting PRF evaluation at both custody salts:
/// `first` seeds the custody keypair, `second` derives the KEK.
fn custody_extensions() -> AuthenticationExtensionsClientInputs {
    let values = AuthenticationExtensionsPrfValues::new_with_u8_array(&Uint8Array::from(
        CUSTODY_KEY_CONTEXT,
    ));
    values.set_second_u8_array(&Uint8Array::from(CUSTODY_KEK_CONTEXT));
    let prf = AuthenticationExtensionsPrfInputs::new();
    prf.set_eval(&values);
    let extensions = AuthenticationExtensionsClientInputs::new();
    extensions.set_prf(&prf);
    extensions
}

/// A fresh user handle, one per credential.
///
/// Not derived from the account, which is what it used to be. An
/// authenticator holding a discoverable credential for the same
/// `(rp.id, user.id)` **replaces** it — that is the spec, and there is
/// no delete API to undo it. So an account-derived handle meant that
/// adding a passkey on a device that already had one destroyed the
/// first, and if its custody cell was the only way in, the account with
/// it.
///
/// Random means credentials never collide and nothing is ever silently
/// destroyed. The cost is that a passkey manager lists them separately
/// rather than grouped, which `name` and `display_name` are for.
fn fresh_user_id() -> [u8; 16] {
    rand::random()
}

/// The origin that owns tonk passkeys. The RP ID is the root-key custody
/// boundary — any origin allowed to use it can silently derive a visiting
/// user's root key with one discoverable-credential assertion — so it is
/// pinned to this exact origin and nothing else. Every other host under
/// `tonk.network`, including any wildcard hostname, is its own relying party
/// with its own disjoint credentials. Widening later is
/// possible via Related Origin Requests; narrowing never is.
const RP_APEX: &str = "tonk.network";

/// The pinned RP ID on the apex origin itself; `None` (WebAuthn's
/// per-host default) everywhere else, which gives localhost tests,
/// off-apex staging, and every other host their own credentials.
fn apex_rp_id(host: &str) -> Option<&'static str> {
    (host == RP_APEX).then_some(RP_APEX)
}

/// The RP ID for the current browsing context, if the host is on-apex.
fn current_rp_id() -> Option<&'static str> {
    let window = web_sys::window()?;
    let location = Reflect::get(&window.into(), &"location".into()).ok()?;
    let hostname = Reflect::get(&location, &"hostname".into()).ok()?;
    let host = hostname.as_string()?;
    apex_rp_id(&host)
}

/// Registration options for a custody passkey: the same discoverable,
/// user-verified credential, with both custody salts requested and the
/// stable account-derived user handle — see [`custody_user_id`].
fn custody_creation_options(
    name: Option<&str>,
    display_name: Option<&str>,
) -> Result<PublicKeyCredentialCreationOptions> {
    let options = creation_options_shell(name, display_name, &fresh_user_id())?;
    options.set_extensions(&custody_extensions());
    Ok(options)
}

fn creation_options_shell(
    name: Option<&str>,
    display_name: Option<&str>,
    user_id: &[u8; 16],
) -> Result<PublicKeyCredentialCreationOptions> {
    let mut challenge = rand::random::<[u8; 32]>();
    let rp = PublicKeyCredentialRpEntity::new("tonk");
    if let Some(id) = current_rp_id() {
        rp.set_id(id);
    }
    // What a passkey manager shows. With per-credential handles these
    // are the only thing telling two entries apart, so a caller that
    // has something distinguishing should pass it.
    let opaque_name = hex::encode(rand::random::<[u8; 16]>());
    // A copy per entity: `new_with_u8_slice` keeps a view on the buffer
    // it is handed rather than copying it, so two entities built from
    // one slice end up sharing a handle — which is exactly the
    // collision the random id exists to avoid.
    let mut handle = *user_id;
    let user = PublicKeyCredentialUserEntity::new_with_u8_slice(
        name.unwrap_or(&opaque_name),
        display_name.or(name).unwrap_or("Tonk identity"),
        &mut handle,
    );
    let params = Array::new();
    for algorithm in COSE_ALGORITHMS {
        params.push(
            &PublicKeyCredentialParameters::new(algorithm, PublicKeyCredentialType::PublicKey)
                .into(),
        );
    }
    let options = PublicKeyCredentialCreationOptions::new_with_u8_slice(
        &mut challenge,
        &params.into(),
        &rp,
        &user,
    );
    let selection = AuthenticatorSelectionCriteria::new();
    selection.set_resident_key("required");
    selection.set_user_verification(UserVerificationRequirement::Required);
    options.set_authenticator_selection(&selection);
    Ok(options)
}

/// Read both custody PRF outputs out of a ceremony's extension results.
fn extract_custody(credential: &PublicKeyCredential) -> Option<CustodyEvaluation> {
    let results = credential
        .get_client_extension_results()
        .get_prf()?
        .get_results()?;
    let first = Uint8Array::new(&results.get_first().into());
    let second = Uint8Array::new(&results.get_second()?.into());
    if first.length() != 32 || second.length() != 32 {
        return None;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    first.copy_to(key.as_mut());
    let mut kek = Zeroizing::new([0u8; 32]);
    second.copy_to(kek.as_mut());
    Some(CustodyEvaluation { key, kek })
}

/// Create a custody passkey for an existing account. One biometric
/// prompt; must be called during a user gesture. Some platforms only
/// evaluate PRF on a follow-up assertion, so `evaluation` may be
/// absent — chase it with [`evaluate_custody_passkey`].
pub async fn create_custody_passkey(
    name: Option<&str>,
    display_name: Option<&str>,
) -> Result<CustodyCredential> {
    let creation = CredentialCreationOptions::new();
    creation.set_public_key(&custody_creation_options(name, display_name)?);
    let promise = credentials()?
        .create_with_options(&creation)
        .map_err(|e| ceremony_error("credentials.create was rejected", e))?;
    let credential: PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|e| ceremony_error("custody passkey creation failed", e))?
        .dyn_into()
        .map_err(|_| anyhow!("credentials.create returned a non-public-key credential"))?;
    let id = Uint8Array::new(&credential.raw_id()).to_vec();
    let evaluation = extract_custody(&credential);
    Ok(CustodyCredential { id, evaluation })
}

/// Evaluate both custody salts via an assertion. One biometric prompt;
/// must be called during a user gesture.
///
/// Pass the stored credential id whenever the caller knows which passkey
/// owns the custody being opened: `allowCredentials` then pins the
/// assertion to it, so a browser holding several passkeys for this RP
/// cannot offer one that derives a different custody space. Discoverable
/// (`None`) is only for flows where choosing the passkey IS choosing the
/// account — login, and a creation chase where the id isn't recorded yet.
pub async fn evaluate_custody_passkey(credential_id: Option<&[u8]>) -> Result<CustodyCredential> {
    let mut challenge = rand::random::<[u8; 32]>();
    let options = PublicKeyCredentialRequestOptions::new_with_u8_slice(&mut challenge);
    options.set_user_verification(UserVerificationRequirement::Required);
    options.set_extensions(&custody_extensions());
    if let Some(id) = current_rp_id() {
        options.set_rp_id(id);
    }
    if let Some(credential_id) = credential_id {
        let mut credential_id = credential_id.to_vec();
        let descriptor = PublicKeyCredentialDescriptor::new_with_u8_slice(
            &mut credential_id,
            PublicKeyCredentialType::PublicKey,
        );
        let allowed = js_sys::Array::new();
        allowed.push(&descriptor);
        options.set_allow_credentials(&allowed);
    }
    let request = CredentialRequestOptions::new();
    request.set_public_key(&options);
    let promise = credentials()?
        .get_with_options(&request)
        .map_err(|e| ceremony_error("credentials.get was rejected", e))?;
    let credential: PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|e| ceremony_error("custody assertion failed", e))?
        .dyn_into()
        .map_err(|_| anyhow!("credentials.get returned a non-public-key credential"))?;
    let id = Uint8Array::new(&credential.raw_id()).to_vec();
    let evaluation = extract_custody(&credential).ok_or_else(|| {
        anyhow::Error::from(CeremonyError {
            context: "custody assertion failed".to_string(),
            reason: CeremonyRefusal::NoPrf,
            detail: "the authenticator returned no PRF outputs; this platform cannot unlock \
                         custody"
                .to_string(),
        })
    })?;
    Ok(CustodyCredential {
        id,
        evaluation: Some(evaluation),
    })
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Reflect, Uint8Array};
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    /// A `DOMException` name becomes a variant, so a caller can tell a
    /// dismissed prompt from a real failure without reading prose.
    #[dialog_common::test]
    fn it_names_the_reason_a_ceremony_was_refused() {
        for (name, expected) in [
            ("NotAllowedError", CeremonyRefusal::NotAllowed),
            ("InvalidStateError", CeremonyRefusal::InvalidState),
            ("NotSupportedError", CeremonyRefusal::NotSupported),
            ("SecurityError", CeremonyRefusal::Security),
            ("NoPrfError", CeremonyRefusal::NoPrf),
            ("FutureError", CeremonyRefusal::Other),
        ] {
            assert_eq!(CeremonyRefusal::from_name(name), expected);
        }
        let refusal = |name: &str| {
            let error = js_sys::Error::new("the operation was refused");
            error.set_name(name);
            ceremony_error("test ceremony", error.into())
        };

        let reason = |name: &str| {
            refusal(name)
                .downcast_ref::<CeremonyError>()
                .expect("a ceremony refusal")
                .reason
        };
        assert_eq!(reason("NotAllowedError"), CeremonyRefusal::NotAllowed);
        assert_eq!(reason("InvalidStateError"), CeremonyRefusal::InvalidState);
        assert_eq!(reason("NotSupportedError"), CeremonyRefusal::NotSupported);
        assert_eq!(reason("SecurityError"), CeremonyRefusal::Security);
    }

    /// Anything unrecognised keeps what the browser said rather than
    /// being flattened into one opaque failure.
    #[dialog_common::test]
    fn it_keeps_the_detail_of_an_unrecognised_refusal() {
        let error = js_sys::Error::new("something novel went wrong");
        error.set_name("SomeFutureError");
        let refusal = ceremony_error("test ceremony", error.into());
        let refusal = refusal
            .downcast_ref::<CeremonyError>()
            .expect("a ceremony refusal");
        assert_eq!(refusal.reason, CeremonyRefusal::Other);
        assert!(
            refusal.detail.contains("something novel went wrong"),
            "{}",
            refusal.detail
        );
    }

    #[dialog_common::test]
    fn it_requests_both_custody_salts() {
        let extensions = custody_extensions();
        let prf = Reflect::get(&extensions, &"prf".into()).unwrap();
        let eval = Reflect::get(&prf, &"eval".into()).unwrap();
        let first = Reflect::get(&eval, &"first".into()).unwrap();
        let second = Reflect::get(&eval, &"second".into()).unwrap();
        assert_eq!(Uint8Array::new(&first).to_vec(), CUSTODY_KEY_CONTEXT);
        assert_eq!(Uint8Array::new(&second).to_vec(), CUSTODY_KEK_CONTEXT);
    }

    /// Every credential gets its own handle.
    ///
    /// It used to be `blake3(account_did)`, which meant two passkeys
    /// for one account on one authenticator shared
    /// `(rp.id, user.id)` — and the spec says the second **replaces**
    /// the first. There is no delete API to undo that, so adding a
    /// passkey could destroy the one already there, and with it the
    /// account if that passkey's cell was the only way in.
    #[dialog_common::test]
    fn it_gives_every_credential_its_own_handle() {
        let one = custody_creation_options(Some("someone@example.com"), None).unwrap();
        let two = custody_creation_options(Some("someone@example.com"), None).unwrap();
        assert_ne!(
            user_handle(&one),
            user_handle(&two),
            "two credentials for the same person never collide"
        );
    }

    #[dialog_common::test]
    fn it_preserves_browser_error_names_and_messages() {
        let error = js_sys::Error::new("phone authenticator returned no PRF");
        error.set_name("NotSupportedError");

        assert_eq!(
            ceremony_error("passkey assertion failed", error.into()).to_string(),
            "passkey assertion failed: NotSupportedError: phone authenticator returned no PRF"
        );
    }

    #[dialog_common::test]
    fn it_requires_a_discoverable_user_verified_credential() {
        let options = custody_creation_options(None, None).unwrap();
        let selection = Reflect::get(&options, &"authenticatorSelection".into()).unwrap();
        let resident = Reflect::get(&selection, &"residentKey".into()).unwrap();
        assert_eq!(resident.as_string().as_deref(), Some("required"));
        let verification = Reflect::get(&selection, &"userVerification".into()).unwrap();
        assert_eq!(verification.as_string().as_deref(), Some("required"));
    }

    fn user_entity(options: &PublicKeyCredentialCreationOptions) -> JsValue {
        Reflect::get(options, &"user".into()).unwrap()
    }

    fn user_field(options: &PublicKeyCredentialCreationOptions, field: &str) -> String {
        Reflect::get(&user_entity(options), &field.into())
            .unwrap()
            .as_string()
            .unwrap()
    }

    fn user_handle(options: &PublicKeyCredentialCreationOptions) -> Vec<u8> {
        Uint8Array::new(&Reflect::get(&user_entity(options), &"id".into()).unwrap()).to_vec()
    }

    /// With no label the entity stays opaque; labelled, it carries the
    /// address so a passkey manager lists something a person can tell
    /// apart. Both fields: Chrome's list and macOS Keychain surface
    /// `name`, not `displayName`.
    #[dialog_common::test]
    fn it_labels_the_user_entity_with_the_account_address() {
        let unlabelled = custody_creation_options(None, None).unwrap();
        assert!(!user_field(&unlabelled, "name").contains('@'));
        assert_eq!(user_field(&unlabelled, "displayName"), "Tonk identity");

        let options = custody_creation_options(Some("someone@example.com"), None).unwrap();
        assert_eq!(user_field(&options, "name"), "someone@example.com");
        assert_eq!(user_field(&options, "displayName"), "someone@example.com");
    }

    #[dialog_common::test]
    fn it_pins_the_rp_id_to_the_apex_origin_only() {
        assert_eq!(apex_rp_id("tonk.network"), Some("tonk.network"));
        // Every other host under the apex is its own relying party, so it
        // cannot derive an apex root key from a visiting user's passkey.
        assert_eq!(apex_rp_id("www.tonk.network"), None);
        assert_eq!(apex_rp_id("hub.tonk.network"), None);
        assert_eq!(apex_rp_id("a.b.tonk.network"), None);
        assert_eq!(apex_rp_id("staging.tonk.xyz"), None);
        assert_eq!(apex_rp_id("localhost"), None);
        // A suffix match must not treat a sibling registrable domain as ours.
        assert_eq!(apex_rp_id("evil-tonk.network"), None);
    }

    #[dialog_common::test]
    fn it_leaves_the_rp_id_unset_off_apex() {
        // wasm tests run on a localhost origin, which is off-apex, so the
        // creation options must carry no id and requests no rpId.
        let options = custody_creation_options(None, None).unwrap();
        let rp = Reflect::get(&options, &"rp".into()).unwrap();
        assert!(
            Reflect::get(&rp, &"id".into()).unwrap().is_undefined(),
            "rp.id must stay unset off the tonk.network apex"
        );
    }
}
