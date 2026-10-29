//! Signing a person in, in a browser: what the embedder does with a passkey
//! for a persona that asked to merge into the persona the person signs in
//! as (`provisional-persona.merge`).
//!
//! The page runs the passkey ceremony and hands the embedder what the
//! passkey derived. The embedder creates the account the passkey holds, or
//! opens the one it holds, and lends the guest kernel both personas' keys
//! for one call of `lifecycle.merge`. In it the guest kernel asks what the
//! ceremony made of the account, and to enroll an account it created at the
//! access service, which the embedder answers from what it keeps here until
//! the merge returns. The account's key and the passkey's are gone then.

use std::cell::RefCell;
use std::rc::Rc;

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Principal as _;
use js_sys::{Reflect, Uint8Array};
use serde_json::{Value, json};
use tonk_identity::account::{Account, Crypto};
use tonk_identity::custodian::Custodian;
use tonk_identity::request::{OwnedCustodyMaterial, build_enroll_invocation};
use wasm_bindgen::{JsCast, JsValue};
use zeroize::Zeroizing;

use crate::credentials::Key;
use crate::kernel::Served;

/// What a passkey ceremony is for: an account it creates, or one it opens.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Create,
    LogIn,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::LogIn => "log-in",
        }
    }
}

/// What a sign-in was asked for: the ceremony, the address the person signs
/// in as, what this device is called, and where the account syncs.
pub struct Asked {
    pub kind: Kind,
    pub email: String,
    pub device_name: String,
    pub remote: String,
}

/// A sign-in under way: what the ceremony made of the account, kept until
/// the merge it lends the account's key to returns.
struct Signing {
    kind: Kind,
    email: String,
    device_name: String,
    remote: String,
    custodian: Custodian,
    account: Rc<Account>,
    /// The custody set an account the ceremony created is enrolled with.
    material: Option<Rc<OwnedCustodyMaterial>>,
    /// The permanent persona's delegation to the representative, once the
    /// guest kernel made it.
    link: Option<DelegationChain>,
}

/// The sign-in under way, if one is.
#[derive(Clone, Default)]
pub struct SignIn(Rc<RefCell<Option<Signing>>>);

/// What the page derived with a passkey, as it hands it over:
/// `{ credentialId, key, kek }`, the credential in hex and the two PRF
/// outputs, which are cleared as they are read.
async fn custodian_of(handoff: &JsValue) -> Result<Custodian, String> {
    let field = |name: &str| {
        Reflect::get(handoff, &JsValue::from_str(name))
            .map_err(|_| format!("the hand-off could not be read for {name}"))
    };
    let take = |name: &str| -> Result<Zeroizing<[u8; 32]>, String> {
        let array = field(name)?
            .dyn_into::<Uint8Array>()
            .map_err(|_| format!("the hand-off's {name} is no Uint8Array"))?;
        let length = array.length();
        let mut bytes = Zeroizing::new([0u8; 32]);
        if length == 32 {
            array.copy_to(bytes.as_mut());
        }
        array.fill(0, 0, length);
        if length != 32 {
            return Err(format!("the hand-off's {name} is {length} bytes, not 32"));
        }
        Ok(bytes)
    };
    let key = take("key");
    let kek = take("kek");
    let credential_id = field("credentialId")?
        .as_string()
        .and_then(|hex| hex::decode(hex).ok())
        .filter(|id| !id.is_empty())
        .ok_or("the hand-off names no credential")?;
    let custodian = tonk_identity::webcrypto_kek::Custodian::adopt(credential_id, &*key?, &*kek?)
        .await
        .map_err(|_| "the passkey's derivations could not be imported".to_owned())?;
    Ok(Custodian::Passkey(custodian))
}

/// An error for the page, with the custody service's `code` for a refusal
/// it knows, such as that the account awaits activation.
fn failure(message: &str, error: Option<&anyhow::Error>) -> JsValue {
    let failed = js_sys::Error::new(message);
    if let Some(denial) = error.and_then(tonk_identity::custody::denial_of) {
        let _ = Reflect::set(&failed, &"code".into(), &denial.code().into());
    }
    failed.into()
}

impl SignIn {
    /// Sign the person in with what the page derived with a passkey, for what
    /// was `asked`: create or open the account, and merge `session`'s
    /// provisional persona into it. Answers the permanent persona's DID and
    /// its delegation to the representative.
    pub async fn sign_in(
        &self,
        kernel: &Served,
        session: &Ed25519Signer,
        handoff: JsValue,
        asked: Asked,
    ) -> Result<(String, Vec<u8>), JsValue> {
        let Asked {
            kind,
            email,
            device_name,
            remote,
        } = asked;
        let custodian = custodian_of(&handoff)
            .await
            .map_err(|error| failure(&error, None))?;
        let account = match kind {
            Kind::Create => custodian
                .account()
                .create()
                .perform(&Crypto)
                .await
                .map_err(|error| failure("the account did not seal under this passkey", Some(&error)))?,
            Kind::LogIn => custodian
                .account()
                .load(remote.clone())
                .perform(&Crypto)
                .await
                .map_err(|error| failure("the account did not open", Some(&error)))?
                .ok_or_else(|| {
                    failure(
                        "this passkey has published no account yet; create one on the browser that holds it",
                        None,
                    )
                })?,
        };
        let Signer::Ed25519(permanent) = account
            .signer()
            .await
            .map_err(|error| failure("the account's key did not derive", Some(&error)))?;
        *self.0.borrow_mut() = Some(Signing {
            kind,
            email,
            device_name,
            remote,
            custodian,
            account: Rc::new(account),
            material: None,
            link: None,
        });
        let merged = kernel
            .merge(
                Key::Sealed(Box::new(session.clone())),
                Key::Sealed(Box::new(permanent)),
            )
            .await;
        self.0.borrow_mut().take();
        merged.map_err(|error| failure(&error, None))
    }

    /// What the ceremony made of the account, for the guest kernel, which
    /// hands over `link`, the permanent persona's delegation to the
    /// representative: as JSON.
    pub async fn ceremony(&self, link: &[u8]) -> Result<String, String> {
        let link = DelegationChain::try_from(link)
            .map_err(|error| format!("the link is no delegation: {error}"))?;
        let (custodian, account, kind) = {
            let held = self.0.borrow();
            let signing = held.as_ref().ok_or("no one is signing in")?;
            (
                signing.custodian.clone(),
                Rc::clone(&signing.account),
                signing.kind,
            )
        };
        let material = match kind {
            Kind::Create => Some(Rc::new(material(&custodian, &account).await?)),
            Kind::LogIn => None,
        };
        let mut held = self.0.borrow_mut();
        let signing = held.as_mut().ok_or("no one is signing in")?;
        let created = (kind == Kind::Create).then(|| {
            json!({
                "createdAt": js_sys::Date::now() as u64 / 1000,
                "createdOn": signing.device_name,
            })
        });
        let answer = json!({
            "kind": kind.name(),
            "credentialId": custodian.credential_id().map(hex::encode).unwrap_or_default(),
            "remote": signing.remote,
            "encryptionKey": account.secret().did().to_string(),
            "passkey": created.unwrap_or(Value::Null),
            "custody": material.as_ref().map(|material| material.custody.to_string()),
            "sealedHex": material.as_ref().map(|material| hex::encode(&material.sealed)),
        });
        signing.material = material;
        signing.link = Some(link);
        Ok(answer.to_string())
    }

    /// The `/customer/enroll` invocation that enrolls the account the
    /// ceremony created, signed by the representative's key, `device`,
    /// through the permanent persona's delegation to it.
    pub async fn enrollment(&self, device: &Ed25519Signer) -> Result<Vec<u8>, String> {
        let (email, link, material) = {
            let held = self.0.borrow();
            let signing = held.as_ref().ok_or("no one is signing in")?;
            (
                signing.email.clone(),
                signing
                    .link
                    .clone()
                    .ok_or("the account linked no device yet")?,
                signing
                    .material
                    .clone()
                    .ok_or("the ceremony created no account to enroll")?,
            )
        };
        build_enroll_invocation(device.clone(), &link, &email, &material.borrow())
            .await
            .map_err(|error| format!("the enrollment did not sign: {error:#}"))
    }
}

/// The custody set an account the ceremony created is enrolled with: the
/// passkey's consent to the account, and its signed write of the sealed
/// account secret.
async fn material(
    custodian: &Custodian,
    account: &Account,
) -> Result<OwnedCustodyMaterial, String> {
    let custody = custodian
        .signer()
        .await
        .map_err(|error| format!("the custody key did not derive: {error:#}"))?;
    let root = account
        .signer()
        .await
        .map_err(|error| format!("the account's key did not derive: {error:#}"))?;
    let sealed = account.envelope().encode();
    let consent = tonk_identity::custody::sign_custody_consent(custody.clone(), &root.did())
        .await
        .map_err(|error| format!("the custody consent did not sign: {error:#}"))?;
    let recovery =
        tonk_identity::custody::sign_deferred_publish_invocation(custody.clone(), &sealed)
            .await
            .map_err(|error| format!("the cell write did not sign: {error:#}"))?;
    Ok(OwnedCustodyMaterial {
        custody: custody.did(),
        consent,
        recovery,
        sealed,
    })
}
