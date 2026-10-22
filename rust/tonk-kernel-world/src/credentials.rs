//! The credentials of a layer, as signers of Dialog DB (PDD015). A credential
//! is a resource of the host; this side holds its public key, and each
//! signature is a call of `credential.sign-invocation` or
//! `credential.sign-delegation`, so no key is in the memory of the guest
//! kernel. Dialog DB builds invocations and delegations as it always does.

use std::rc::Rc;

use dialog_credentials::credential::SignerCredential;
use dialog_credentials::ed25519::external::{ExternalSign, ExternalSigning, SigningKey};
use dialog_credentials::{
    Ed25519Signer, Ed25519SigningKey, Ed25519Verifier, Ed25519VerifyingKey, Signer,
};
use ipld_core::ipld::Ipld;

use crate::guest::layer_error;
use crate::layer::LayerError;
use crate::world::sapling::kernel::credentials::Credential;

/// The kind of UCAN a payload is: the key of its payload map.
fn invocation(payload: &[u8]) -> Option<bool> {
    let Ok(Ipld::Map(map)) = serde_ipld_dagcbor::from_slice::<Ipld>(payload) else {
        return None;
    };
    map.keys().find_map(|key| match key.split_once('@') {
        Some(("ucan/inv", _)) => Some(true),
        Some(("ucan/dlg", _)) => Some(false),
        _ => None,
    })
}

struct CredentialSign(Rc<Credential>);

impl ExternalSign for CredentialSign {
    fn sign(&self, payload: &[u8]) -> ExternalSigning {
        let credential = Rc::clone(&self.0);
        let payload = payload.to_vec();
        Box::pin(async move {
            let signed = match invocation(&payload) {
                Some(true) => credential.sign_invocation(payload, None).await,
                Some(false) => credential.sign_delegation(payload, None).await,
                None => {
                    return Err(signature::Error::from_source(LayerError::Denied(
                        "a credential signs a UCAN invocation or delegation".into(),
                    )));
                }
            };
            let signature =
                signed.map_err(|error| signature::Error::from_source(layer_error(error)))?;
            <[u8; 64]>::try_from(signature).map_err(|_| signature::Error::new())
        })
    }
}

/// `credential` as a signer of Dialog DB.
///
/// # Errors
///
/// If the DID of the credential is not an Ed25519 `did:key`.
pub fn signer(credential: Credential) -> Result<SignerCredential, LayerError> {
    let not_ed25519 = || LayerError::Invalid("the credential is not an Ed25519 did:key".into());
    let did: Ed25519Verifier = credential.did().parse().map_err(|_| not_ed25519())?;
    // A build for WASI has native keys alone.
    let Ed25519VerifyingKey::Native(verifying_key) = did.0;
    let key = SigningKey::new(verifying_key, CredentialSign(Rc::new(credential)));
    Ok(SignerCredential::from(Signer::from(Ed25519Signer::from(
        Ed25519SigningKey::from(key),
    ))))
}
