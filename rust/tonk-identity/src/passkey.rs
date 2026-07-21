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
    anyhow!("{context}: {value:?}")
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

/// The apex that owns tonk passkeys. The RP ID is the root-key custody
/// boundary — every current and future origin under it can silently
/// derive a visiting user's root key with one discoverable-credential
/// assertion — so two invariants hold: nothing untrusted is ever served
/// from `*.tonk.spot`, and staging deploys live off-apex so they mint
/// disjoint credentials. Widening later is possible via Related Origin
/// Requests; narrowing never is.
const RP_APEX: &str = "tonk.spot";

/// The pinned RP ID for hosts under the apex; `None` (WebAuthn's
/// per-host default) everywhere else, which keeps localhost tests and
/// off-apex staging working with their own credentials.
fn apex_rp_id(host: &str) -> Option<&'static str> {
    (host == RP_APEX || host.ends_with(".tonk.spot")).then_some(RP_APEX)
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
fn creation_options(user_name: &str) -> Result<PublicKeyCredentialCreationOptions> {
    let mut challenge = rand::random::<[u8; 32]>();
    let rp = PublicKeyCredentialRpEntity::new("tonk");
    if let Some(id) = current_rp_id() {
        rp.set_id(id);
    }
    let mut user_id = rand::random::<[u8; 16]>();
    let user = PublicKeyCredentialUserEntity::new_with_u8_slice(user_name, user_name, &mut user_id);
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
pub async fn create_passkey(user_name: &str) -> Result<PasskeyCredential> {
    let creation = CredentialCreationOptions::new();
    creation.set_public_key(&creation_options(user_name)?);
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
pub async fn prf_output() -> Result<Zeroizing<[u8; 32]>> {
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
    extract_prf(&credential).ok_or_else(|| {
        anyhow!("the authenticator returned no PRF output; this platform cannot derive a root key")
    })
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
    fn it_requires_a_discoverable_user_verified_credential() {
        let options = creation_options("tester").unwrap();
        let selection = Reflect::get(&options, &"authenticatorSelection".into()).unwrap();
        let resident = Reflect::get(&selection, &"residentKey".into()).unwrap();
        assert_eq!(resident.as_string().as_deref(), Some("required"));
        let verification = Reflect::get(&selection, &"userVerification".into()).unwrap();
        assert_eq!(verification.as_string().as_deref(), Some("required"));
    }

    #[dialog_common::test]
    fn it_pins_the_rp_id_to_the_apex_only_for_spot_hosts() {
        assert_eq!(apex_rp_id("tonk.spot"), Some("tonk.spot"));
        assert_eq!(apex_rp_id("hub.tonk.spot"), Some("tonk.spot"));
        assert_eq!(apex_rp_id("a.b.tonk.spot"), Some("tonk.spot"));
        assert_eq!(apex_rp_id("staging.tonk.xyz"), None);
        assert_eq!(apex_rp_id("localhost"), None);
        // A suffix match must not treat a sibling registrable domain as ours.
        assert_eq!(apex_rp_id("evil-tonk.spot"), None);
    }

    #[dialog_common::test]
    fn it_leaves_the_rp_id_unset_off_apex() {
        // wasm tests run on a localhost origin, which is off-apex, so the
        // creation options must carry no id and requests no rpId.
        let options = creation_options("tester").unwrap();
        let rp = Reflect::get(&options, &"rp".into()).unwrap();
        assert!(
            Reflect::get(&rp, &"id".into()).unwrap().is_undefined(),
            "rp.id must stay unset off the tonk.spot apex"
        );
    }
}
