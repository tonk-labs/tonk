//! Writing a custody cell during enrollment.
//!
//! Enrollment verifies a recovery invocation the ceremony pre-signed —
//! self-issued by the passkey's custody key, naming one fixed cell, and
//! checksumming the sealed envelope that travels beside it. Redeeming it
//! there rather than queueing it is what keeps a signup from finishing
//! with an account nobody can open on a second device.
//!
//! Nothing is served while this lands: the customer is `Registered`, and
//! the gate refuses every read and write behind a provider in that
//! state. So the cell is written into a space that answers nothing until
//! the emailed link is clicked.
//!
//! The key is not derived here. The authorizer already turns a verified
//! invocation into a signed request naming exactly the object that
//! invocation authorizes, so this executes that request rather than
//! rebuilding the layout it encodes.

use async_trait::async_trait;

/// Why a custody cell could not be written.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The service could not reach storage, or storage refused. Always
    /// the service's own fault as far as the caller is concerned: the
    /// invocation was verified before it got here.
    #[error("the custody cell could not be stored: {0}")]
    Unavailable(String),
}

/// Somewhere a custody cell can be put.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Vault {
    /// Redeem `recovery` — a verified `/use/put/memory/cell` invocation
    /// chain, re-encoded as a redeemable container (proofs included,
    /// when the ceremony issued through a delegate) — and store `sealed`
    /// where it names.
    ///
    /// Takes the invocation rather than a key so the object written is
    /// the one the ceremony authorized, not one this service chose.
    ///
    /// The write is CREATE-ONLY: the invocation carries no `when`, which
    /// production storage signs into the permit as `if-none-match: *`,
    /// so a cell that already exists answers 412. Enrollment publishes
    /// once per custody space for exactly this reason, and every
    /// implementation — the in-memory one included — must refuse a
    /// second write the same way, or tests pass against a laxer store
    /// than the one deployed.
    async fn publish(&self, recovery: &[u8], sealed: &[u8]) -> Result<(), VaultError>;
}

/// A [`Vault`] that keeps cells in memory, for tests and local
/// development. Holds `(recovery, sealed)` pairs in the order written,
/// keyed create-only by the custody subject the invocation names — the
/// same refusal production storage answers with a 412.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct CapturedVault(pub std::sync::Mutex<Vec<(Vec<u8>, Vec<u8>)>>);

/// The custody subject a redeemable recovery container acts on.
#[cfg(any(test, feature = "helpers"))]
fn recovery_subject(recovery: &[u8]) -> Result<String, VaultError> {
    dialog_ucan_core::InvocationChain::<dialog_varsig::AnySignature>::try_from(recovery)
        .map(|chain| chain.subject().to_string())
        .map_err(|error| {
            VaultError::Unavailable(format!("the recovery container did not decode: {error}"))
        })
}

#[cfg(any(test, feature = "helpers"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Vault for CapturedVault {
    async fn publish(&self, recovery: &[u8], sealed: &[u8]) -> Result<(), VaultError> {
        let subject = recovery_subject(recovery)?;
        let mut cells = self.0.lock().expect("vault mutex poisoned");
        for (held, _) in cells.iter() {
            if recovery_subject(held)? == subject {
                // What production answers: the permit carries
                // `if-none-match: *`, so the second write of a cell is
                // a 412, not a replacement.
                return Err(VaultError::Unavailable(format!(
                    "storage answered 412 Precondition Failed: {subject} already holds a cell"
                )));
            }
        }
        cells.push((recovery.to_vec(), sealed.to_vec()));
        Ok(())
    }
}

#[cfg(any(test, feature = "helpers"))]
impl CapturedVault {
    /// The sealed bytes written for a cell, if any were.
    pub fn sealed(&self) -> Vec<Vec<u8>> {
        self.0
            .lock()
            .expect("vault mutex poisoned")
            .iter()
            .map(|(_, sealed)| sealed.clone())
            .collect()
    }
}

/// A [`Vault`] backed by an authorizer: the invocation is redeemed for a
/// presigned request, and that request is executed.
///
/// The service holds the authorizer already, and it is what turns a
/// verified invocation into the exact object that invocation names. So
/// nothing here knows the key layout — asking would mean encoding it
/// twice, and the copies would drift.
pub struct AuthorizedVault<A>(pub A);

/// What an authorizer must do to back a vault: redeem a container for a
/// permit. Narrower than the authorizer's own surface, so tests need not
/// stand up S3 to exercise the caller.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Redeemer {
    /// Turn a verified container into a presigned request.
    async fn redeem(&self, container: &[u8]) -> Result<dialog_remote_s3::Permit, VaultError>;
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<A: Redeemer + dialog_common::ConditionalSync> Vault for AuthorizedVault<A> {
    async fn publish(&self, recovery: &[u8], sealed: &[u8]) -> Result<(), VaultError> {
        // `recovery` is already a redeemable container — the verified
        // chain re-encoded with its proofs. Re-wrapping the bare token
        // here used to drop those proofs, so a delegate-issued recovery
        // verified at enrollment and then failed at the write.
        let permit = self.0.redeem(recovery).await?;
        let response = permit
            .upload(sealed.to_vec())
            .await
            .map_err(|error| VaultError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(VaultError::Unavailable(format!(
                "storage answered {}",
                response.status()
            )));
        }
        Ok(())
    }
}
