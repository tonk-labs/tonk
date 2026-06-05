//! Sync-state classification.
//!
//! A pure, I/O-free comparison of a branch's local head against its
//! upstream head. Both the native CLI and the wasm worker call this to
//! drive a synced/ahead/behind/diverged indicator and to decide whether
//! a sync would do anything.

use dialog_repository::Revision;
use serde::{Deserialize, Serialize};

/// How a branch's local head relates to its upstream head.
///
/// Serializes kebab-case (`no-upstream`, `synced`, `ahead`,
/// `behind`, `diverged`) so the wire shape reads the same as the
/// states the UI and `slide status` render.
///
/// [`classify`] derives the four head-comparison states as a
/// [`Comparison`]; the configuration state `NoUpstream` is supplied
/// by the caller (which knows the branch's configuration) by
/// widening a `Comparison` through [`From`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncState {
    /// No upstream is configured for this branch. Supplied by the caller;
    /// never produced by [`classify`].
    NoUpstream,
    /// Local and upstream heads are identical — nothing to do.
    Synced,
    /// Local is strictly ahead: a push would fast-forward the upstream.
    Ahead,
    /// Local is strictly behind: a pull would fast-forward the local head.
    Behind,
    /// The two heads have diverged, or their relationship can't be proven
    /// from the heads alone. The safe fallback — a sync needs a merge.
    Diverged,
}

/// The relationship [`classify`] can prove between a branch's local and
/// upstream heads.
///
/// Exactly [`SyncState`] minus the configuration state `NoUpstream`, so
/// the classifier's return type matches what it can actually produce.
/// Callers widen into [`SyncState`] (adding `NoUpstream` when no upstream
/// is configured) via the [`From`] impl below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// Local and upstream heads are identical.
    Synced,
    /// Local is strictly ahead by one proven commit.
    Ahead,
    /// Local is strictly behind by one proven commit.
    Behind,
    /// The relationship can't be proven from the two heads alone.
    Diverged,
}

impl From<Comparison> for SyncState {
    fn from(comparison: Comparison) -> Self {
        match comparison {
            Comparison::Synced => SyncState::Synced,
            Comparison::Ahead => SyncState::Ahead,
            Comparison::Behind => SyncState::Behind,
            Comparison::Diverged => SyncState::Diverged,
        }
    }
}

/// Classify `local` against `remote` from the two heads alone.
///
/// This is the cheap variant: one [`Revision`] from each side, no history
/// walk. It only reports a direction it can *prove*:
///
/// - identical trees (including both heads absent) → [`Comparison::Synced`];
/// - one side absent and the other present → the present side leads;
/// - the remote head is the local head's recorded parent → [`Comparison::Ahead`];
/// - the local head is the remote head's recorded parent → [`Comparison::Behind`].
///
/// Anything else is [`Comparison::Diverged`]. Two deliberate limits feed
/// that fallback:
///
/// - **One-commit horizon.** [`Revision::cause`] records only the immediate
///   parent, so the ahead/behind checks span a single commit. A strictly
///   linear branch two or more commits ahead of (or behind) its upstream
///   reads as `Diverged`, not `Ahead`/`Behind` — proving the longer
///   relationship would need the history walk this function avoids. A
///   pull-then-push still reconciles such a branch correctly; only the
///   reported label is conservative.
/// - **No clock tiebreak.** The logical clocks (`period`, `moment`) are not
///   consulted: two replicas that fork while the same issuer keeps committing
///   produce the same clock signature as a genuine linear lead, so the clocks
///   can't distinguish "ahead" from "diverged".
///
/// Reporting `Diverged` in both cases keeps the invariant that we never claim
/// a direction we can't back up from the heads alone.
pub fn classify(local: Option<&Revision>, remote: Option<&Revision>) -> Comparison {
    match (local, remote) {
        (None, None) => Comparison::Synced,
        (Some(_), None) => Comparison::Ahead,
        (None, Some(_)) => Comparison::Behind,
        (Some(local), Some(remote)) => {
            if local.tree == remote.tree {
                Comparison::Synced
            } else if local.cause.contains(&remote.tree) {
                Comparison::Ahead
            } else if remote.cause.contains(&local.tree) {
                Comparison::Behind
            } else {
                Comparison::Diverged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_capability::Did;
    use dialog_repository::TreeReference;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn did() -> Did {
        "did:key:test".parse().unwrap()
    }

    fn tree(byte: u8) -> TreeReference {
        TreeReference::from([byte; 32])
    }

    /// A revision with the given head tree and recorded parent trees.
    fn rev(head: TreeReference, cause: &[TreeReference]) -> Revision {
        Revision {
            subject: did(),
            issuer: did(),
            authority: did(),
            tree: head,
            cause: cause.iter().cloned().collect(),
            period: 0,
            moment: 0,
        }
    }

    #[dialog_common::test]
    fn it_is_synced_when_heads_match() {
        let local = rev(tree(1), &[]);
        let remote = rev(tree(1), &[]);
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Synced);
    }

    #[dialog_common::test]
    fn it_is_synced_when_both_heads_are_absent() {
        assert_eq!(classify(None, None), Comparison::Synced);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_remote_is_absent() {
        let local = rev(tree(1), &[]);
        assert_eq!(classify(Some(&local), None), Comparison::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_local_is_absent() {
        let remote = rev(tree(1), &[]);
        assert_eq!(classify(None, Some(&remote)), Comparison::Behind);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_remote_is_the_local_parent() {
        let remote = rev(tree(1), &[]);
        let local = rev(tree(2), &[tree(1)]);
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_local_is_the_remote_parent() {
        let local = rev(tree(1), &[]);
        let remote = rev(tree(2), &[tree(1)]);
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Behind);
    }

    #[dialog_common::test]
    fn it_is_diverged_when_heads_are_unrelated() {
        let local = rev(tree(1), &[tree(3)]);
        let remote = rev(tree(2), &[tree(4)]);
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Diverged);
    }

    #[dialog_common::test]
    fn it_is_diverged_when_local_leads_by_more_than_one_commit() {
        // `cause` records only the immediate parent, so a strictly
        // linear branch two commits ahead reads as Diverged: the shared
        // base tree(1) is the grandparent, absent from tree(3)'s cause.
        // This pins the documented one-commit horizon (a pull-then-push
        // still reconciles it; only the reported label is conservative).
        let remote = rev(tree(1), &[]);
        let local = rev(tree(3), &[tree(2)]);
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Diverged);
    }

    #[dialog_common::test]
    fn it_widens_each_comparison_into_the_matching_sync_state() {
        assert_eq!(SyncState::from(Comparison::Synced), SyncState::Synced);
        assert_eq!(SyncState::from(Comparison::Ahead), SyncState::Ahead);
        assert_eq!(SyncState::from(Comparison::Behind), SyncState::Behind);
        assert_eq!(SyncState::from(Comparison::Diverged), SyncState::Diverged);
    }

    #[dialog_common::test]
    fn it_serializes_states_kebab_case() {
        // The UI badges and `slide status` read these exact strings.
        let cases = [
            (SyncState::NoUpstream, "\"no-upstream\""),
            (SyncState::Synced, "\"synced\""),
            (SyncState::Ahead, "\"ahead\""),
            (SyncState::Behind, "\"behind\""),
            (SyncState::Diverged, "\"diverged\""),
        ];
        for (state, expected) in cases {
            assert_eq!(serde_json::to_string(&state).unwrap(), expected);
        }
    }
}
