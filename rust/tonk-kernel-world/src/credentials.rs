//! A credential, as a signer of Dialog DB. A credential is a resource of the
//! host; this side holds its public key. Dialog DB builds what it signs and
//! hands the bytes over; this reads them into a payload ([`crate::payload`])
//! and has the host sign that, so no key is in the memory of the guest kernel.

use std::rc::Rc;

use dialog_credentials::credential::SignerCredential;
use dialog_credentials::ed25519::external::{
    ExternalDeriving, ExternalRevealing, ExternalSecret, ExternalSign, ExternalSigning, SigningKey,
};
use dialog_credentials::secret::{SealedSecret, SecretError};
use dialog_credentials::{
    Ed25519Signer, Ed25519SigningKey, Ed25519Verifier, Ed25519VerifyingKey, Signer,
};

use crate::guest::layer_error;
use crate::layer::LayerError;
use crate::payload::{self, Payload};
use crate::world::sapling::kernel::credential::{self as world, Credential};

/// `payload`, as the world names it.
fn named(payload: Payload) -> world::Payload {
    match payload {
        Payload::Invocation(invocation) => world::Payload::Invocation(world::Invocation {
            audience: invocation.audience,
            subject: invocation.subject,
            command: invocation.command,
            arguments: invocation.arguments,
            proofs: invocation.proofs,
            cause: invocation.cause,
            issued_at: invocation.issued_at,
            expiration: invocation.expiration,
            meta: invocation.meta,
            nonce: invocation.nonce,
        }),
        Payload::Delegation(delegation) => world::Payload::Delegation(world::Delegation {
            audience: delegation.audience,
            subject: delegation.subject,
            command: delegation.command,
            policy: delegation.policy,
            expiration: delegation.expiration,
            not_before: delegation.not_before,
            meta: delegation.meta,
            nonce: delegation.nonce,
        }),
        Payload::Head(head) => world::Payload::Head(world::Head {
            branch: head.branch,
            tree: head.tree.to_vec(),
            edition: head.edition,
            context: head.context.map(|context| {
                context
                    .into_iter()
                    .map(|watermark| world::Watermark {
                        origin: watermark.origin.to_vec(),
                        edition: watermark.edition,
                        count: watermark.count,
                    })
                    .collect()
            }),
        }),
        Payload::RevisionRecord(record) => {
            world::Payload::RevisionRecord(world::RevisionRecord { body: record.body })
        }
    }
}

struct CredentialSign(Rc<Credential>);

impl ExternalSign for CredentialSign {
    fn sign(&self, bytes: &[u8]) -> ExternalSigning {
        let credential = Rc::clone(&self.0);
        let payload = payload::read(bytes, &credential.did());
        Box::pin(async move {
            let payload = payload.map_err(|reason| {
                signature::Error::from_source(LayerError::Denied(format!(
                    "a credential signs a UCAN, a head or a revision record: {reason}"
                )))
            })?;
            let signature = credential
                .sign(named(payload))
                .await
                .map_err(|error| signature::Error::from_source(layer_error(error)))?;
            <[u8; 64]>::try_from(signature).map_err(|_| signature::Error::new())
        })
    }
}

/// What reveals a secret concealed to a credential: the host, which holds its
/// key.
struct CredentialSecret(Rc<Credential>);

impl ExternalSecret for CredentialSecret {
    fn reveal(&self, context: &'static str, sealed: SealedSecret) -> ExternalRevealing {
        let credential = Rc::clone(&self.0);
        Box::pin(async move {
            credential
                .reveal(context.as_bytes().to_vec(), sealed.to_bytes())
                .await
                .map_err(|_| SecretError::Failed)
        })
    }

    fn derive(&self, context: &'static str, label: Vec<u8>) -> ExternalDeriving {
        let credential = Rc::clone(&self.0);
        Box::pin(async move {
            let derived = credential
                .derive(context.as_bytes().to_vec(), label)
                .await
                .map_err(|error| SecretError::Crypto(format!("{:?}", layer_error(error))))?;
            key(Rc::new(derived)).map_err(|error| SecretError::Crypto(error.to_string()))
        })
    }
}

/// `credential` as a key held outside this process, which signs, reveals and
/// derives through the host.
fn key(credential: Rc<Credential>) -> Result<SigningKey, LayerError> {
    let not_ed25519 = || LayerError::Invalid("the credential is not an Ed25519 did:key".into());
    let did: Ed25519Verifier = credential.did().parse().map_err(|_| not_ed25519())?;
    // A build for WASI has native keys alone.
    let Ed25519VerifyingKey::Native(verifying_key) = did.0;
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
pub fn signer(credential: Rc<Credential>) -> Result<SignerCredential, LayerError> {
    Ok(SignerCredential::from(Signer::from(Ed25519Signer::from(
        Ed25519SigningKey::from(key(credential)?),
    ))))
}
