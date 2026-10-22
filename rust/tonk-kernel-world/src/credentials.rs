//! The credentials of a layer, as signers of Dialog DB (PDD015). A credential
//! is a resource of the host; this side holds its public key, and each
//! signature is a call of the credential (`sign-head`, `sign-revision-record`,
//! `sign-invocation` or `sign-delegation`), so no key is in the memory of the
//! guest kernel. Dialog DB builds invocations and delegations as it always does.

use std::rc::Rc;

use dialog_artifacts::history::RECORD_SIGNING_DOMAIN;
use dialog_capability::identity::HEAD_SIGNING_DOMAIN;
use dialog_credentials::credential::SignerCredential;
use dialog_credentials::ed25519::external::{
    ExternalDeriving, ExternalRevealing, ExternalSecret, ExternalSign, ExternalSigning, SigningKey,
};
use dialog_credentials::secret::{SealedSecret, SecretError};
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
        let head = payload
            .strip_prefix(HEAD_SIGNING_DOMAIN)
            .map(<[u8]>::to_vec);
        let record = payload
            .strip_prefix(RECORD_SIGNING_DOMAIN)
            .map(<[u8]>::to_vec);
        let payload = payload.to_vec();
        Box::pin(async move {
            let signed = match (head, record, invocation(&payload)) {
                (Some(head), _, _) => credential.sign_head(head).await,
                (None, Some(record), _) => credential.sign_revision_record(record).await,
                (None, None, Some(true)) => credential.sign_invocation(payload, None).await,
                (None, None, Some(false)) => credential.sign_delegation(payload, None).await,
                (None, None, None) => {
                    return Err(signature::Error::from_source(LayerError::Denied(
                        "a credential signs a head, a revision record, or a UCAN".into(),
                    )));
                }
            };
            let signature =
                signed.map_err(|error| signature::Error::from_source(layer_error(error)))?;
            <[u8; 64]>::try_from(signature).map_err(|_| signature::Error::new())
        })
    }
}

/// What runs a credential's key agreement: the host, which holds its key.
struct CredentialSecret(Rc<Credential>);

impl ExternalSecret for CredentialSecret {
    fn reveal(&self, context: &'static str, sealed: SealedSecret) -> ExternalRevealing {
        let credential = Rc::clone(&self.0);
        Box::pin(async move {
            credential
                .reveal(context.to_owned(), sealed.to_bytes())
                .await
                .map_err(|_| SecretError::Failed)
        })
    }

    fn derive(&self, context: &'static str, label: Vec<u8>) -> ExternalDeriving {
        let credential = Rc::clone(&self.0);
        Box::pin(async move {
            let derived = credential
                .derive(context.to_owned(), label, None)
                .await
                .map_err(|error| SecretError::Crypto(format!("{:?}", layer_error(error))))?;
            key(derived).map_err(|error| SecretError::Crypto(error.to_string()))
        })
    }
}

/// `credential` as a key held outside this process, which signs, reveals and
/// derives through the host.
fn key(credential: Credential) -> Result<SigningKey, LayerError> {
    let not_ed25519 = || LayerError::Invalid("the credential is not an Ed25519 did:key".into());
    let did: Ed25519Verifier = credential.did().parse().map_err(|_| not_ed25519())?;
    // A build for WASI has native keys alone.
    let Ed25519VerifyingKey::Native(verifying_key) = did.0;
    let credential = Rc::new(credential);
    Ok(
        SigningKey::new(verifying_key, CredentialSign(Rc::clone(&credential)))
            .with_secret(CredentialSecret(credential)),
    )
}

/// `credential` as a signer of Dialog DB.
///
/// # Errors
///
/// If the DID of the credential is not an Ed25519 `did:key`.
pub fn signer(credential: Credential) -> Result<SignerCredential, LayerError> {
    Ok(SignerCredential::from(Signer::from(Ed25519Signer::from(
        Ed25519SigningKey::from(key(credential)?),
    ))))
}
