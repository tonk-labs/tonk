//! [`TrackingBranch`] — a local branch that tracks a remote branch.

// The `#[derive(Concept)]` macro generates helper types and
// associated functions without doc comments. Suppress the
// crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::{Branch, Upstream};

/// A local branch's tracking relationship with a remote branch.
///
/// A `TrackingBranch` is an *attribute on the local branch entity*
/// that names the remote branch it tracks. `this` is reused from
/// the local branch itself (no hash); `upstream` points at the
/// remote branch. Asserting a `TrackingBranch` fact says: "the
/// branch whose entity is `this` is tracking the branch whose
/// entity is `upstream`."
///
/// # Why a separate concept
///
/// Semantically this is just an optional `Upstream` attribute on
/// [`Branch`] — a branch either tracks something or it doesn't.
/// Dialog concepts require every field to be present, so
/// "optional" can't be expressed as `Option<Upstream>` on `Branch`
/// itself. Modeling it as a separate concept lets the optionality
/// fall out of presence/absence of the fact: if a `TrackingBranch`
/// fact is asserted for a given branch entity, that branch tracks
/// something; if not, it doesn't. Queries pick it up via an
/// optional-match / left-join pattern rather than a null check.
///
/// # Why `Upstream` instead of reusing `Origin`
///
/// [`Branch`] already has an `origin: Origin` attribute pointing at
/// its replica or remote. Using `Origin` again here for the
/// tracked-branch relation would conflate "I belong to X" with "I
/// track X" across the schema, making queries ambiguous. A
/// dedicated [`Upstream`] attribute keeps the two relations
/// distinct.
///
/// # Constructing
///
/// [`TrackingBranch::new`] takes the local branch and the upstream
/// (remote) branch it tracks:
///
/// ```no_run
/// # use tonk_schema::{Branch, TrackingBranch};
/// # fn example(local: Branch, upstream: Branch) -> TrackingBranch {
/// TrackingBranch::new(&local, &upstream)
/// # }
/// ```
///
/// Or via the [`Branch::set_upstream`] shortcut:
///
/// ```no_run
/// # use tonk_schema::{Branch, TrackingBranch};
/// # fn example(local: Branch, upstream: Branch) -> TrackingBranch {
/// local.set_upstream(&upstream)
/// # }
/// ```
///
/// [`Branch::set_upstream`]: crate::Branch::set_upstream
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct TrackingBranch {
    /// The local branch's entity.
    pub this: Entity,
    /// The upstream (remote) branch this one is tracking.
    pub upstream: Upstream,
}

impl TrackingBranch {
    /// Build a tracking-branch link.
    ///
    /// `this` is set to `local`'s entity (no hash — this concept
    /// attaches a relationship attribute to an existing branch) and
    /// `upstream` points at the `upstream` branch being tracked.
    pub fn new(local: &Branch, upstream: &Branch) -> Self {
        Self {
            this: local.this.clone(),
            upstream: Upstream::from(upstream.this.clone()),
        }
    }
}

impl Branch {
    /// Record that this branch tracks `upstream`.
    ///
    /// Shortcut for [`TrackingBranch::new(self, upstream)`][TrackingBranch::new].
    pub fn set_upstream(&self, upstream: &Branch) -> TrackingBranch {
        TrackingBranch::new(self, upstream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, Name, Remote, Replica};
    use dialog_varsig::did;

    fn setup() -> (Branch, Branch) {
        let replica = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let remote = Remote::new(
            &replica,
            did!("test:repo"),
            Address(b"addr".to_vec()),
            Name("origin".into()),
        );
        let local = replica.branch("main");
        let tracked = remote.branch("main");
        (local, tracked)
    }

    #[test]
    fn this_is_local_branch_entity() {
        let (local, upstream) = setup();
        let link = TrackingBranch::new(&local, &upstream);
        assert_eq!(link.this, local.this);
    }

    #[test]
    fn upstream_is_remote_branch_entity() {
        let (local, upstream) = setup();
        let link = TrackingBranch::new(&local, &upstream);
        assert_eq!(link.upstream.0, upstream.this);
    }

    #[test]
    fn set_upstream_matches_new() {
        let (local, upstream) = setup();
        let via_method = local.set_upstream(&upstream);
        let via_new = TrackingBranch::new(&local, &upstream);
        assert_eq!(via_method, via_new);
    }
}
