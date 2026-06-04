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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncState {
    /// No upstream is configured for this branch. Supplied by the caller
    /// (which knows the branch's configuration); never inferred here.
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

/// Classify `local` against `remote` from the two heads alone.
///
/// This is the cheap variant: one [`Revision`] from each side, no history
/// walk. It only reports a direction it can *prove*:
///
/// - identical trees (including both heads absent) → [`SyncState::Synced`];
/// - one side absent and the other present → the present side leads;
/// - the remote head is the local head's recorded parent → [`SyncState::Ahead`];
/// - the local head is the remote head's recorded parent → [`SyncState::Behind`].
///
/// Anything else is [`SyncState::Diverged`]. In particular the logical
/// clocks (`period`, `moment`) are deliberately *not* used as a tiebreak:
/// two replicas that fork while the same issuer keeps committing produce
/// the same clock signature as a genuine linear lead, so the clocks can't
/// distinguish "ahead" from "diverged". Reporting `Diverged` there keeps
/// the invariant that we never claim a direction we can't back up.
///
/// [`SyncState::NoUpstream`] is never returned — the caller supplies it
/// when no upstream is configured.
pub fn classify(local: Option<&Revision>, remote: Option<&Revision>) -> SyncState {
    match (local, remote) {
        (None, None) => SyncState::Synced,
        (Some(_), None) => SyncState::Ahead,
        (None, Some(_)) => SyncState::Behind,
        (Some(local), Some(remote)) => {
            if local.tree == remote.tree {
                SyncState::Synced
            } else if local.cause.contains(&remote.tree) {
                SyncState::Ahead
            } else if remote.cause.contains(&local.tree) {
                SyncState::Behind
            } else {
                SyncState::Diverged
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
        assert_eq!(classify(Some(&local), Some(&remote)), SyncState::Synced);
    }

    #[dialog_common::test]
    fn it_is_synced_when_both_heads_are_absent() {
        assert_eq!(classify(None, None), SyncState::Synced);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_remote_is_absent() {
        let local = rev(tree(1), &[]);
        assert_eq!(classify(Some(&local), None), SyncState::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_local_is_absent() {
        let remote = rev(tree(1), &[]);
        assert_eq!(classify(None, Some(&remote)), SyncState::Behind);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_remote_is_the_local_parent() {
        let remote = rev(tree(1), &[]);
        let local = rev(tree(2), &[tree(1)]);
        assert_eq!(classify(Some(&local), Some(&remote)), SyncState::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_local_is_the_remote_parent() {
        let local = rev(tree(1), &[]);
        let remote = rev(tree(2), &[tree(1)]);
        assert_eq!(classify(Some(&local), Some(&remote)), SyncState::Behind);
    }

    #[dialog_common::test]
    fn it_is_diverged_when_heads_are_unrelated() {
        let local = rev(tree(1), &[tree(3)]);
        let remote = rev(tree(2), &[tree(4)]);
        assert_eq!(classify(Some(&local), Some(&remote)), SyncState::Diverged);
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
