//! Time-window enforcement for presented UCAN containers.
//!
//! The chain walk refuses an expired chain on its own: it intersects
//! every hop's bounds with the invocation's and compares the result to
//! the clock, so an expired chain never reaches a permit. This screen
//! is not what makes expiry enforced.
//!
//! What it does is name the refusal. The walk reports a time bound as a
//! generic verification failure, which reaches a client as
//! `CHAIN_INVALID`; running the same question here first turns it into
//! `401 INVOCATION_EXPIRED`, which clients distinguish from a chain
//! that never held up. Deleting this module would keep the enforcement
//! and lose the code, so it stays until clients stop reading it.
//!
//! Unbounded chains are unaffected: a `root -> device` grant carries no
//! expiration, so its window is open and every check passes.

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::AnySignature;

/// The window every hop of a presented chain agrees on.
///
/// Each bound is the tightest any hop declares: the latest start and the
/// earliest expiration, so the window is the intersection rather than
/// any one hop's claim. `None` means unbounded on that side.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresentedWindow {
    /// Latest start bound in unix seconds.
    pub not_before: Option<u64>,
    /// Earliest expiration bound in unix seconds.
    pub expires_at: Option<u64>,
}

/// Read the validity window off a presented container.
///
/// Revocation is not read here: the authorizer carries a checker and
/// answers that per link while verifying. This is only the clock
/// question, which no part of the chain walk asks.
pub fn collect_window(container_bytes: &[u8]) -> Result<PresentedWindow, ContainerError> {
    let tokens = Container::from_bytes(container_bytes)?.into_tokens();
    let Some(invocation_bytes) = tokens.first() else {
        return Err(ContainerError::Invocation(
            "container must contain at least an invocation".to_string(),
        ));
    };
    let invocation: Invocation<AnySignature> = serde_ipld_dagcbor::from_slice(invocation_bytes)
        .map_err(|error| {
            ContainerError::Invocation(format!("failed to decode invocation: {error}"))
        })?;
    let mut not_before: Option<u64> = None;
    let mut expires_at = invocation.expiration().map(|stamp| stamp.to_unix());
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<AnySignature> =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|error| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {error}"))
            })?;
        if let Some(stamp) = delegation.not_before() {
            not_before = Some(not_before.map_or(stamp.to_unix(), |seen| seen.max(stamp.to_unix())));
        }
        if let Some(stamp) = delegation.expiration() {
            expires_at = Some(expires_at.map_or(stamp.to_unix(), |seen| seen.min(stamp.to_unix())));
        }
    }
    Ok(PresentedWindow {
        not_before,
        expires_at,
    })
}

/// Whether the presented chain is valid at a given moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowVerdict {
    /// Now falls inside the window every hop agrees on, or no hop
    /// bounds it.
    Valid,
    /// Every hop agreed the chain would stop being valid before now.
    Expired,
    /// A hop declares the chain does not start until later.
    NotYetValid,
}

/// Check the presented chain's effective window against `now_s`, in
/// unix seconds.
///
/// The bounds are inclusive: a chain expiring exactly now is still
/// valid, matching how the intersection is computed upstream and
/// avoiding a one-second cliff for clients that stamp `now + ttl`.
pub fn check_window(presented: &PresentedWindow, now_s: u64) -> WindowVerdict {
    if let Some(not_before) = presented.not_before
        && now_s < not_before
    {
        return WindowVerdict::NotYetValid;
    }
    if let Some(expires_at) = presented.expires_at
        && now_s > expires_at
    {
        return WindowVerdict::Expired;
    }
    WindowVerdict::Valid
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn bounded(not_before: Option<u64>, expires_at: Option<u64>) -> PresentedWindow {
        PresentedWindow {
            not_before,
            expires_at,
        }
    }

    #[dialog_common::test]
    async fn it_accepts_an_unbounded_chain() {
        let verdict = check_window(&bounded(None, None), 1_000);

        assert_eq!(
            verdict,
            WindowVerdict::Valid,
            "an unexpiring root to device grant must keep working"
        );
    }

    #[dialog_common::test]
    async fn it_accepts_a_chain_inside_its_window() {
        let verdict = check_window(&bounded(Some(500), Some(1_500)), 1_000);

        assert_eq!(verdict, WindowVerdict::Valid);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_past_its_expiration() {
        let verdict = check_window(&bounded(None, Some(999)), 1_000);

        assert_eq!(verdict, WindowVerdict::Expired);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_before_it_starts() {
        let verdict = check_window(&bounded(Some(1_001), None), 1_000);

        assert_eq!(verdict, WindowVerdict::NotYetValid);
    }

    /// The window is the intersection, not any one hop's claim.
    ///
    /// Built from real delegations: a root expiring late and a leaf
    /// expiring early, plus a `not_before` on the leaf. What must come
    /// back is the tightest of each, since a chain is only usable where
    /// every hop agrees it is.
    #[dialog_common::test]
    async fn it_reads_the_tightest_bound_each_hop_declares() {
        use dialog_credentials::{Ed25519Signer, Signer};
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_ucan_core::time::Timestamp;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder};

        let at = |seconds: u64| {
            Timestamp::new(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .expect("a representable timestamp")
        };
        use dialog_varsig::Principal as _;

        let space = Ed25519Signer::import(&[110u8; 32]).await.expect("a signer");
        let profile = Ed25519Signer::import(&[111u8; 32]).await.expect("a signer");
        let device = Ed25519Signer::import(&[112u8; 32]).await.expect("a signer");

        let root = DelegationBuilder::new()
            .issuer(Signer::from(space.clone()))
            .audience(&profile.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .expiration(at(9_000))
            .try_build()
            .await
            .expect("a delegation");
        let leaf = DelegationBuilder::new()
            .issuer(Signer::from(profile.clone()))
            .audience(&device.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .not_before(at(1_000))
            .expiration(at(5_000))
            .try_build()
            .await
            .expect("a delegation");
        let chain = DelegationChain::new(root)
            .push(leaf)
            .expect("the hops connect");

        let invocation = InvocationBuilder::new()
            .issuer(Signer::from(device.clone()))
            .audience(&space.did())
            .subject(&space.did())
            .command(vec!["test".to_string()])
            .arguments(std::collections::BTreeMap::new())
            .proofs(chain.proof_cids().to_vec())
            .try_build()
            .await
            .expect("an invocation");
        let mut tokens =
            vec![serde_ipld_dagcbor::to_vec(&invocation).expect("the invocation encodes")];
        for (_, delegation) in chain.export() {
            tokens.push(delegation.encoded().to_vec());
        }
        let bytes = dialog_ucan_core::Container::new(tokens)
            .into_bytes()
            .expect("a container");

        let window = collect_window(&bytes).expect("the container parses");
        assert_eq!(
            window,
            PresentedWindow {
                not_before: Some(1_000),
                expires_at: Some(5_000),
            },
            "the earliest expiration and the latest start bound the chain"
        );
        assert_eq!(check_window(&window, 6_000), WindowVerdict::Expired);
        assert_eq!(check_window(&window, 3_000), WindowVerdict::Valid);
    }

    #[dialog_common::test]
    async fn it_refuses_a_container_it_cannot_read() {
        assert!(
            collect_window(b"not a container").is_err(),
            "an unreadable container yields no window to screen against"
        );
    }

    #[dialog_common::test]
    async fn it_treats_the_bounds_as_inclusive() {
        assert_eq!(
            check_window(&bounded(Some(1_000), Some(1_000)), 1_000),
            WindowVerdict::Valid,
            "a chain expiring exactly now is still valid"
        );
    }
}
