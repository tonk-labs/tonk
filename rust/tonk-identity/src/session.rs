//! The `device → session` delegation.
//!
//! `root → device` is subject-open, command-open and unexpiring: the
//! most powerful thing the root can sign, held indefinitely by every
//! linked device. Nothing about it degrades on its own, which is why
//! withdrawing it has to be a registry lookup — there is no renewal to
//! withhold.
//!
//! A session splits that. The device keeps its grant and uses it to
//! mint a short-lived delegation to an ephemeral key, and the session
//! key is what signs presign invocations. The grant stays the thing
//! that survives offline and that the registry records; the session is
//! the thing that lapses on its own.

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;

/// How long a session delegation is good for.
///
/// Hours, not minutes: a session must survive a stretch offline and a
/// laptop lid, or clients will churn through renewals and the
/// unreachable-renewal path becomes the common one rather than the
/// exceptional one. Short enough that a device losing its grant stops
/// mattering within a working day.
pub const SESSION_TTL_SECONDS: u64 = 12 * 60 * 60;

/// Mint the `device → session` delegation: subject-open like the grant
/// it descends from, but bounded in time.
///
/// The device signs this itself from the `root → device` grant it
/// already holds — no network, no renewal endpoint, nothing to be
/// unreachable. Withdrawal works by the grant's registry row, which the
/// chain walk already checks; the expiry is what bounds the damage
/// between a revoke and the next presign.
pub async fn mint_session_delegation(
    device: Ed25519Signer,
    session: &Did,
    ttl_seconds: u64,
) -> Result<DelegationChain> {
    // dialog re-exports the platform clock: std natively, web_time
    // under wasm, where std::time::SystemTime is a different type.
    let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(ttl_seconds))
        .map_err(|err| anyhow::anyhow!("session expiration out of range: {err}"))?;

    let delegation = DelegationBuilder::new()
        .issuer(Signer::from(device))
        .audience(session)
        .subject(UcanSubject::Any)
        .command(vec![])
        .expiration(expiration)
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the session delegation: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

/// Extend a `root → device` grant with a freshly minted session hop,
/// producing the chain a session key presents.
///
/// The composed chain still carries the grant's subject and every hop's
/// CID, so the chain walk sees the device's identity as well as the
/// session's — revoking the device severs the session on its next
/// presign without the session itself being known to the registry.
pub async fn extend_with_session(
    grant: &DelegationChain,
    device: Ed25519Signer,
    session: &Did,
    ttl_seconds: u64,
) -> Result<DelegationChain> {
    let session_chain = mint_session_delegation(device, session, ttl_seconds).await?;
    let session_delegation = session_chain
        .proofs()
        .last()
        .ok_or_else(|| anyhow::anyhow!("session chain has no proof"))?
        .clone();
    grant
        .push(session_delegation)
        .map_err(|err| anyhow::anyhow!("failed to extend the grant with a session: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_varsig::principal::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const ROOT_PRF: [u8; 32] = [1u8; 32];
    const DEVICE_SEED: [u8; 32] = [2u8; 32];
    const SESSION_SEED: [u8; 32] = [4u8; 32];

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    #[dialog_common::test]
    async fn it_mints_a_bounded_delegation_to_the_session_key() {
        let device = signer(&DEVICE_SEED).await;
        let session = signer(&SESSION_SEED).await;

        let chain = mint_session_delegation(device, &session.did(), SESSION_TTL_SECONDS)
            .await
            .unwrap();

        assert_eq!(*chain.audience(), session.did());
        assert!(
            chain.expiration().is_some(),
            "a session delegation must expire; that is the whole point"
        );
        assert!(
            chain.subject().is_none(),
            "the session inherits the grant's subject-open shape"
        );
    }

    #[dialog_common::test]
    async fn it_expires_within_the_requested_ttl() {
        let device = signer(&DEVICE_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let before = Timestamp::now().to_unix();

        let chain = mint_session_delegation(device, &session.did(), 900)
            .await
            .unwrap();

        let expires_at = chain.expiration().unwrap().to_unix();
        assert!(expires_at >= before + 900);
        assert!(expires_at <= Timestamp::now().to_unix() + 900);
    }

    #[dialog_common::test]
    async fn it_extends_a_device_grant_with_a_session_hop() {
        let root = Ed25519Signer::import(&ROOT_PRF).await.unwrap();
        let device = signer(&DEVICE_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let grant = crate::delegation::mint_device_delegation(root, &device.did())
            .await
            .unwrap();

        let composed = extend_with_session(&grant, device, &session.did(), SESSION_TTL_SECONDS)
            .await
            .unwrap();

        assert_eq!(*composed.audience(), session.did());
        assert_eq!(
            composed.proof_cids().len(),
            2,
            "the grant hop must stay in the chain so the screen still sees the device"
        );
        assert!(composed.expiration().is_some());
    }

    #[dialog_common::test]
    async fn it_narrows_an_unexpiring_grant() {
        let root = Ed25519Signer::import(&ROOT_PRF).await.unwrap();
        let device = signer(&DEVICE_SEED).await;
        let session = signer(&SESSION_SEED).await;
        let grant = crate::delegation::mint_device_delegation(root, &device.did())
            .await
            .unwrap();
        assert!(grant.expiration().is_none(), "the grant is unexpiring");

        let composed = extend_with_session(&grant, device, &session.did(), 900)
            .await
            .unwrap();

        assert!(
            composed.expiration().is_some(),
            "extending an unbounded grant must bound the result"
        );
    }
}
