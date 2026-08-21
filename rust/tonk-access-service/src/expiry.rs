//! Time-window enforcement for presented UCAN containers.
//!
//! The chain verifier computes the window a chain is valid in and hands
//! it back as a `TimeRange`; `InvocationChain::verify` discards it, so
//! nothing on the presign path ever compared it to the clock. A chain
//! that expired last year verifies exactly like a fresh one — only a
//! chain that can *never* be valid is rejected upstream.
//!
//! This screen closes that. It reads the window off the same parse the
//! revocation screen already does
//! ([`PresentedCredentials`](super::revocation::PresentedCredentials))
//! and refuses a presign outside it.
//!
//! Unbounded chains are unaffected: a `root → device` grant carries no
//! expiration, so its window is open and every check passes. That is
//! what makes this safe to turn on ahead of the clients that will start
//! bounding themselves, and it is the enforcement short-lived session
//! delegations depend on — an expiry nothing checks buys nothing.

use crate::revocation::PresentedCredentials;

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
pub fn check_window(presented: &PresentedCredentials, now_s: u64) -> WindowVerdict {
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

    fn bounded(not_before: Option<u64>, expires_at: Option<u64>) -> PresentedCredentials {
        PresentedCredentials {
            delegators: Default::default(),
            subject: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                .parse()
                .expect("test DID parses"),
            delegation_cids: vec!["bafycid".to_string()],
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

    #[dialog_common::test]
    async fn it_treats_the_bounds_as_inclusive() {
        assert_eq!(
            check_window(&bounded(Some(1_000), Some(1_000)), 1_000),
            WindowVerdict::Valid,
            "a chain expiring exactly now is still valid"
        );
    }
}
