//! Sync-route wire DTOs and sync-state classification.
//!
//! [`SyncState`]/[`classify`] are a pure, I/O-free comparison of a
//! branch's local head against its upstream head — they need only a
//! [`Revision`] from each side, no engine. They live here (not in
//! `tonk-schema`, which pulls the datalog engine) so the page can name
//! `SyncState` and even classify without linking the engine.
//! `tonk-schema` re-exports them for the native CLI and the worker.

use dialog_artifacts::Revision;
use serde::{Deserialize, Serialize};

/// How a branch's local head relates to its upstream head.
///
/// Serializes kebab-case (`no-upstream`, `synced`, `ahead`,
/// `behind`, `diverged`) so the wire shape reads the same as the
/// states the UI and `tonk status` render.
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
/// - the local context includes the remote's → [`Comparison::Ahead`]
///   (local has observed everything the remote has, and more);
/// - the remote context includes the local's → [`Comparison::Behind`].
///
/// Anything else is [`Comparison::Diverged`]. The comparison rests on each
/// head's causal [`Context`](dialog_artifacts::history::Context) — the
/// per-origin watermark of everything the revision has observed. Unlike the
/// old immediate-parent set, an included context proves the relationship
/// across arbitrary linear distance, so a branch many commits ahead of (or
/// behind) its upstream reads correctly, not conservatively as `Diverged`.
///
/// Two cases still fall back to `Diverged`:
///
/// - **No published context.** [`Revision::context`] is `None` on heads
///   minted before the history index existed. Without it the direction
///   cannot be proven from the head alone, so such a pairing reads as
///   `Diverged` (a pull-then-push still reconciles it; only the label is
///   conservative).
/// - **Genuinely concurrent.** Neither context includes the other — the two
///   replicas each hold revisions the other lacks.
///
/// Reporting `Diverged` in these cases keeps the invariant that we never
/// claim a direction we can't back up from the heads alone.
pub fn classify(local: Option<&Revision>, remote: Option<&Revision>) -> Comparison {
    match (local, remote) {
        (None, None) => Comparison::Synced,
        (Some(_), None) => Comparison::Ahead,
        (None, Some(_)) => Comparison::Behind,
        (Some(local), Some(remote)) => {
            if local.tree == remote.tree {
                return Comparison::Synced;
            }
            let (Some(local_ctx), Some(remote_ctx)) = (&local.context, &remote.context) else {
                // A head minted before the history index carries no context,
                // so the direction can't be proven from the heads alone.
                return Comparison::Diverged;
            };
            let local_leads = local_ctx.includes(remote_ctx);
            let remote_leads = remote_ctx.includes(local_ctx);
            match (local_leads, remote_leads) {
                // Trees differ yet each observed all the other has: not a
                // relationship the heads can order, so stay conservative.
                (true, true) => Comparison::Diverged,
                (true, false) => Comparison::Ahead,
                (false, true) => Comparison::Behind,
                (false, false) => Comparison::Diverged,
            }
        }
    }
}

/// Why a successful sync operation returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncDisposition {
    /// Reconciliation ran to completion.
    #[default]
    Completed,
    /// Deliberately skipped because the browser is offline.
    Offline,
    /// Deliberately skipped because synchronization is paused.
    Paused,
}

/// Response for successful sync operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Compatibility field; successful responses always serialize `true`.
    pub success: bool,
    /// Whether reconciliation completed or was deliberately skipped.
    #[serde(default)]
    pub disposition: SyncDisposition,
    /// Local branch revision *before* the sync ran. `None` when
    /// the branch had no commits at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Revision>,
    /// Local branch revision *after* the sync. `None` when the
    /// branch still has no commits, or when the operation failed
    /// before producing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Revision>,
    /// Legacy field accepted during rollout; new successful responses omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Sync-state of a branch relative to its upstream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    /// How the local head relates to the upstream head.
    pub state: SyncState,
    /// Local branch revision, or `null` if it has no commits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<Revision>,
    /// Upstream branch revision as last fetched, or `null` if the
    /// upstream has no commits (or none is configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Revision>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::history::{Context, Edition, Origin, Version};
    use dialog_artifacts::{Entity, TreeReference};

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn tree(byte: u8) -> TreeReference {
        TreeReference::from([byte; 32])
    }

    fn origin(byte: u8) -> Origin {
        Origin([byte; 32])
    }

    /// A context that has observed each origin's chain up to the given
    /// edition (one `record` per edition, so the revision count matches
    /// the depth). Passing `&[(origin, depth)]` builds the per-origin
    /// watermark map `classify` compares.
    fn context(watermarks: &[(Origin, u64)]) -> Context {
        let mut ctx = Context::new();
        for (origin, depth) in watermarks {
            for edition in 1..=*depth {
                ctx.record(Version::new(*origin, Edition::new(edition)));
            }
        }
        ctx
    }

    fn branch() -> Entity {
        "did:key:test".parse().unwrap()
    }

    /// A revision with the given head tree and causal context.
    fn rev(head: TreeReference, context: Context) -> Revision {
        let mut revision = Revision::new(head, branch(), "did:key:test".parse().unwrap());
        revision.context = Some(context);
        revision
    }

    /// A revision minted before the history index existed: no context.
    fn contextless(head: TreeReference) -> Revision {
        Revision::new(head, branch(), "did:key:test".parse().unwrap())
    }

    #[dialog_common::test]
    fn it_is_synced_when_heads_match() {
        let local = rev(tree(1), context(&[(origin(0), 1)]));
        let remote = rev(tree(1), context(&[(origin(0), 1)]));
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Synced);
    }

    #[dialog_common::test]
    fn it_is_synced_when_both_heads_are_absent() {
        assert_eq!(classify(None, None), Comparison::Synced);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_remote_is_absent() {
        let local = rev(tree(1), context(&[(origin(0), 1)]));
        assert_eq!(classify(Some(&local), None), Comparison::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_local_is_absent() {
        let remote = rev(tree(1), context(&[(origin(0), 1)]));
        assert_eq!(classify(None, Some(&remote)), Comparison::Behind);
    }

    #[dialog_common::test]
    fn it_is_ahead_when_local_context_includes_remote() {
        // Same origin, local one edition deeper: local has observed
        // everything remote has, and more.
        let remote = rev(tree(1), context(&[(origin(0), 1)]));
        let local = rev(tree(2), context(&[(origin(0), 2)]));
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Ahead);
    }

    #[dialog_common::test]
    fn it_is_behind_when_remote_context_includes_local() {
        let local = rev(tree(1), context(&[(origin(0), 1)]));
        let remote = rev(tree(2), context(&[(origin(0), 2)]));
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Behind);
    }

    #[dialog_common::test]
    fn it_is_diverged_when_contexts_are_concurrent() {
        // Each side holds a revision from an origin the other lacks:
        // neither context includes the other.
        let local = rev(tree(1), context(&[(origin(1), 1)]));
        let remote = rev(tree(2), context(&[(origin(2), 1)]));
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Diverged);
    }

    #[dialog_common::test]
    fn it_is_ahead_across_a_multi_commit_lead() {
        // The context proves ahead/behind across arbitrary linear
        // distance — a branch several commits ahead reads as Ahead, not
        // conservatively as Diverged the way the old parent-set check did.
        let remote = rev(tree(1), context(&[(origin(0), 1)]));
        let local = rev(tree(3), context(&[(origin(0), 5)]));
        assert_eq!(classify(Some(&local), Some(&remote)), Comparison::Ahead);
    }

    #[dialog_common::test]
    fn it_is_diverged_when_a_head_carries_no_context() {
        // A pre-history-index head can't be ordered from the head alone.
        let remote = contextless(tree(1));
        let local = rev(tree(2), context(&[(origin(0), 2)]));
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
    fn it_serializes_success_dispositions_in_kebab_case() {
        let response = SyncResponse {
            success: true,
            disposition: SyncDisposition::Offline,
            before: None,
            after: None,
            error: None,
        };
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["disposition"], "offline");
        assert!(json.get("error").is_none());
    }

    #[dialog_common::test]
    fn it_serializes_states_kebab_case() {
        // The UI badges and `tonk status` read these exact strings.
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
