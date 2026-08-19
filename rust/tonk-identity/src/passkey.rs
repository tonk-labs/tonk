//! WebAuthn passkey ceremonies.
//!
//! Window-context only: `navigator.credentials` does not exist in
//! workers, so these run from the page main thread, inside a user
//! gesture.

use crate::derive::ROOT_KEY_CONTEXT;
use anyhow::{Context, Result, anyhow};
use js_sys::{Array, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AuthenticationExtensionsClientInputs, AuthenticationExtensionsPrfInputs,
    AuthenticationExtensionsPrfValues, AuthenticatorSelectionCriteria, CredentialCreationOptions,
    CredentialRequestOptions, CredentialsContainer, PublicKeyCredential,
    PublicKeyCredentialCreationOptions, PublicKeyCredentialParameters,
    PublicKeyCredentialRequestOptions, PublicKeyCredentialRpEntity, PublicKeyCredentialType,
    PublicKeyCredentialUserEntity, UserVerificationRequirement,
};
use zeroize::Zeroizing;

/// COSE algorithms offered at registration: EdDSA, ES256, RS256. The
/// credential's own key is never used directly — only its PRF — so this
/// list is purely for authenticator compatibility.
const COSE_ALGORITHMS: [i32; 3] = [-8, -7, -257];

/// A created passkey: the raw credential id, plus the PRF output when
/// the platform evaluated PRF during creation (some only do so on a
/// follow-up assertion).
pub struct PasskeyCredential {
    /// Raw credential id, as registered with the authenticator.
    pub id: Vec<u8>,
    /// PRF output, when the platform returned one at creation.
    pub prf_output: Option<Zeroizing<[u8; 32]>>,
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
    anyhow!("{context}: {detail}")
}

fn credentials() -> Result<CredentialsContainer> {
    Ok(web_sys::window()
        .context("no window: passkey ceremonies are window-only")?
        .navigator()
        .credentials())
}

/// Extension inputs requesting a PRF evaluation over the versioned
/// derivation context.
fn prf_extensions() -> AuthenticationExtensionsClientInputs {
    let values =
        AuthenticationExtensionsPrfValues::new_with_u8_array(&Uint8Array::from(ROOT_KEY_CONTEXT));
    let prf = AuthenticationExtensionsPrfInputs::new();
    prf.set_eval(&values);
    let extensions = AuthenticationExtensionsClientInputs::new();
    extensions.set_prf(&prf);
    extensions
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

/// Registration options: a discoverable, user-verified credential on
/// this origin, with PRF requested up front.
///
/// `label` names the credential in the user's passkey manager. It carries
/// the account address when an account ceremony is what creates this
/// passkey, and is `None` for a root created before any account exists —
/// which must not imply one. Both `name` and `displayName` take it:
/// Chrome's passkey list and macOS Keychain surface `name`, so labelling
/// only `displayName` would leave the list unreadable.
///
/// The user handle stays random regardless. It is not a display field —
/// it rides every assertion — and deriving it from an address would make
/// two accounts on one authenticator collide.
fn creation_options(label: Option<&str>) -> Result<PublicKeyCredentialCreationOptions> {
    let mut challenge = rand::random::<[u8; 32]>();
    let rp = PublicKeyCredentialRpEntity::new("tonk");
    if let Some(id) = current_rp_id() {
        rp.set_id(id);
    }
    let mut user_id = rand::random::<[u8; 32]>();
    let opaque_name = hex::encode(rand::random::<[u8; 16]>());
    let user = PublicKeyCredentialUserEntity::new_with_u8_slice(
        label.unwrap_or(&opaque_name),
        label.unwrap_or("Tonk identity"),
        &mut user_id,
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
    options.set_extensions(&prf_extensions());
    Ok(options)
}

/// Read the PRF output out of a ceremony's extension results.
fn extract_prf(credential: &PublicKeyCredential) -> Option<Zeroizing<[u8; 32]>> {
    let results = credential
        .get_client_extension_results()
        .get_prf()?
        .get_results()?;
    let first = Uint8Array::new(&results.get_first().into());
    if first.length() != 32 {
        return None;
    }
    let mut output = Zeroizing::new([0u8; 32]);
    first.copy_to(output.as_mut());
    Some(output)
}

/// Create the account passkey on this origin. One biometric prompt;
/// must be called during a user gesture.
///
/// `label` is the name the passkey manager shows — see [`creation_options`].
pub async fn create_passkey(label: Option<&str>) -> Result<PasskeyCredential> {
    let creation = CredentialCreationOptions::new();
    creation.set_public_key(&creation_options(label)?);
    let promise = credentials()?
        .create_with_options(&creation)
        .map_err(|e| ceremony_error("credentials.create was rejected", e))?;
    let credential: PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|e| ceremony_error("passkey creation failed", e))?
        .dyn_into()
        .map_err(|_| anyhow!("credentials.create returned a non-public-key credential"))?;
    let id = Uint8Array::new(&credential.raw_id()).to_vec();
    let prf_output = extract_prf(&credential);
    Ok(PasskeyCredential { id, prf_output })
}

/// Evaluate the passkey's PRF via a discoverable-credential assertion.
/// One biometric prompt; must be called during a user gesture.
pub async fn evaluate_passkey() -> Result<PasskeyCredential> {
    let mut challenge = rand::random::<[u8; 32]>();
    let options = PublicKeyCredentialRequestOptions::new_with_u8_slice(&mut challenge);
    options.set_user_verification(UserVerificationRequirement::Required);
    options.set_extensions(&prf_extensions());
    if let Some(id) = current_rp_id() {
        options.set_rp_id(id);
    }
    let request = CredentialRequestOptions::new();
    request.set_public_key(&options);
    let promise = credentials()?
        .get_with_options(&request)
        .map_err(|e| ceremony_error("credentials.get was rejected", e))?;
    let credential: PublicKeyCredential = JsFuture::from(promise)
        .await
        .map_err(|e| ceremony_error("passkey assertion failed", e))?
        .dyn_into()
        .map_err(|_| anyhow!("credentials.get returned a non-public-key credential"))?;
    let id = Uint8Array::new(&credential.raw_id()).to_vec();
    let prf_output = extract_prf(&credential).ok_or_else(|| {
        anyhow!("the authenticator returned no PRF output; this platform cannot derive a root key")
    })?;
    Ok(PasskeyCredential {
        id,
        prf_output: Some(prf_output),
    })
}

/// Evaluate and return only the PRF output.
pub async fn prf_output() -> Result<Zeroizing<[u8; 32]>> {
    evaluate_passkey()
        .await?
        .prf_output
        .ok_or_else(|| anyhow!("the authenticator returned no PRF output"))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::derive::ROOT_KEY_CONTEXT;
    use js_sys::{Reflect, Uint8Array};
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_requests_prf_evaluation_with_the_versioned_context() {
        let extensions = prf_extensions();
        let prf = Reflect::get(&extensions, &"prf".into()).unwrap();
        let eval = Reflect::get(&prf, &"eval".into()).unwrap();
        let first = Reflect::get(&eval, &"first".into()).unwrap();
        assert_eq!(Uint8Array::new(&first).to_vec(), ROOT_KEY_CONTEXT);
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
        let options = creation_options(None).unwrap();
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

    /// With no label the entity stays opaque: a passkey created before any
    /// account exists must not imply one.
    #[dialog_common::test]
    fn it_uses_an_opaque_user_entity_when_unlabelled() {
        let first = creation_options(None).unwrap();
        let second = creation_options(None).unwrap();

        assert_ne!(user_field(&first, "name"), user_field(&second, "name"));
        assert!(!user_field(&first, "name").contains('@'));
        assert_eq!(user_field(&first, "displayName"), "Tonk identity");
    }

    /// Labelled, the entity carries the address, so a passkey manager lists
    /// something a person can tell apart from their other keys. Both fields:
    /// Chrome's list and macOS Keychain surface `name`, not `displayName`.
    #[dialog_common::test]
    fn it_labels_the_user_entity_with_the_account_address() {
        let options = creation_options(Some("someone@example.com")).unwrap();

        assert_eq!(user_field(&options, "name"), "someone@example.com");
        assert_eq!(user_field(&options, "displayName"), "someone@example.com");
    }

    /// The handle stays random either way. It is the credential's user id,
    /// and deriving it from an address would make two accounts on one
    /// authenticator collide — and leak the address into a field that is sent
    /// on every assertion, not just shown in a manager.
    /// Each handle is read the moment its options are built, not after all
    /// three exist: `new_with_u8_slice` gives JS a view into wasm linear
    /// memory rather than a copy, so a later call reusing that slot changes
    /// what an earlier entity's `id` reads back. Production never holds two
    /// option objects at once — each goes straight to `credentials.create` —
    /// but a test that compares them has to read as it goes.
    #[dialog_common::test]
    fn it_keeps_the_user_handle_random_whether_labelled_or_not() {
        let labelled = user_handle(&creation_options(Some("someone@example.com")).unwrap());
        let again = user_handle(&creation_options(Some("someone@example.com")).unwrap());
        let unlabelled = user_handle(&creation_options(None).unwrap());

        assert_eq!(labelled.len(), 32, "a full 32-byte handle");
        assert_ne!(labelled, again, "same address, different handle");
        assert_ne!(labelled, unlabelled);
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
        let options = creation_options(None).unwrap();
        let rp = Reflect::get(&options, &"rp".into()).unwrap();
        assert!(
            Reflect::get(&rp, &"id".into()).unwrap().is_undefined(),
            "rp.id must stay unset off the tonk.network apex"
        );
    }
}
