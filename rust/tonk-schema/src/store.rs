//! Typed read/write surface over a meta branch.
//!
//! `MetaStore` is the shared facade that carry (CLI) and tonk-worker
//! (browser service worker) both use to transact and query the
//! `tonk-schema` concepts on a meta branch. It wraps `(operator,
//! branch)` and exposes typed methods for the operations both crates
//! used to hand-roll inline.
//!
//! # Caller composes the transaction
//!
//! Write-side methods on `MetaStore` are *concept-returning*, not
//! commit-driving. They build the typed concept (or look up the
//! concepts that need retracting) and hand them back; the caller
//! composes them into a `branch.transaction()` and commits when
//! ready. This matches dialog's own builder pattern and lets the
//! caller batch a `MetaStore` operation and a future
//! [`tonk-notation`]-emitted statement into one atomic transaction.
//!
//! Reads are direct — they take the round trip and return typed
//! values.
//!
//! # Per-repo meta vs profile meta
//!
//! The same `MetaStore` works for both flavours of meta branch:
//!
//! - **Per-repository meta**: facts about a repository's replicas,
//!   branches, remotes, and tracking links. Methods take a
//!   `Replica::new(profile_did, repo_did, name)`.
//! - **Profile meta**: facts about every repository the profile has
//!   a replica of. Methods take the profile's self-replica
//!   `Replica::new(profile_did, profile_did, name)`. The
//!   `find_replica_for_subject` and `list_replicas_for_profile`
//!   queries are the load-bearing ops here.

#![allow(clippy::needless_lifetimes)]

use anyhow::{Context, Result};
use dialog_artifacts::Entity;
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch as DialogBranch, RemoteSite, SiteAddress};
use dialog_varsig::Did;

use crate::prelude::DidExt;
use crate::{Branch, Name, Remote, Replica, TrackingBranch};

/// Operator-shaped environment for meta-branch reads and writes.
///
/// Mirrors the `Provider<...>` chain that dialog itself requires
/// for `branch.query()...perform(env)` and
/// `branch.transaction()...commit().perform(env)`. Any operator
/// (native, wasm, or test/volatile) that satisfies these provider
/// bounds implements `MetaEnv` automatically — call sites just
/// pass `&operator`.
pub trait MetaEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> MetaEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Concepts that must be retracted alongside a `Remote` to avoid
/// dangling references on the meta branch. Returned by
/// [`MetaStore::dependents_of_remote`].
#[derive(Debug, Clone)]
pub struct RemoteDependents {
    /// The remote-side `Branch` concept owned by the remote
    /// (e.g. `<remote>/main`). The schema has no convention for
    /// "the" tracked branch beyond this — callers retract every
    /// branch that names the remote as origin.
    pub branches: Vec<Branch>,
    /// Tracking links pointing at any branch on the remote.
    pub tracking_links: Vec<TrackingBranch>,
}

/// Typed read/write facade over a meta branch.
///
/// Hold one of these at the call site — all operations are
/// methods. Both crates can construct it the same way:
///
/// ```ignore
/// let store = MetaStore::new(&meta_branch, &operator);
/// let remotes = store.list_remotes(&replica).await?;
/// ```
pub struct MetaStore<'a, E> {
    /// The meta branch this store reads from and writes to.
    pub branch: &'a DialogBranch,
    /// The operator/environment used to perform I/O.
    pub env: &'a E,
}

impl<'a, E> MetaStore<'a, E> {
    /// Build a new store over `(branch, env)`.
    pub fn new(branch: &'a DialogBranch, env: &'a E) -> Self {
        Self { branch, env }
    }

    // ---------------- pure builders ----------------
    //
    // Zero-I/O wrappers around the existing concept constructors.
    // Their job is discoverability — call sites that already hold
    // a `MetaStore` shouldn't also need to glob-import every
    // concept module to compose an assertion.

    /// Construct a [`Replica`] concept (no I/O).
    pub fn replica(&self, profile: Did, subject: Did, name: impl Into<Name>) -> Replica {
        Replica::new(profile, subject, name)
    }

    /// Construct a [`Remote`] concept owned by `replica` (no I/O).
    pub fn remote(
        &self,
        replica: &Replica,
        name: impl Into<Name>,
        subject: Did,
        address: &SiteAddress,
    ) -> Remote {
        replica.remote(name, subject, address)
    }

    /// Construct a [`Branch`] concept on `owner` (no I/O).
    ///
    /// `owner` is anything that views as an entity — a `Replica`
    /// for a local branch, a `Remote` for the remote-side
    /// counterpart.
    pub fn branch_of(&self, owner: impl AsRef<Entity>, name: impl Into<Name>) -> Branch {
        Branch::new(owner, name)
    }

    /// Construct a [`TrackingBranch`] linking `local` → `upstream`
    /// (no I/O).
    pub fn tracking(&self, local: &Branch, upstream: &Branch) -> TrackingBranch {
        TrackingBranch::new(local, upstream)
    }

    /// Replica-anchor bundle for a fresh meta branch.
    ///
    /// Returns `(Replica, Branch)` — the replica concept plus a
    /// `Branch` concept naming the meta branch itself (using the
    /// dialog branch's own name, so the returned `Branch.name`
    /// always matches `self.branch.name()`).
    ///
    /// Caller asserts both in one transaction:
    ///
    /// ```ignore
    /// let (replica, meta_branch) = store.claim_replica(profile, subject, "home");
    /// store.branch.transaction()
    ///     .assert(replica)
    ///     .assert(meta_branch)
    ///     .commit().perform(env).await?;
    /// ```
    pub fn claim_replica(
        &self,
        profile: Did,
        subject: Did,
        name: impl Into<Name>,
    ) -> (Replica, Branch) {
        let replica = Replica::new(profile, subject, name);
        let meta = replica.branch(self.branch.name());
        (replica, meta)
    }
}

impl<'a, E: MetaEnv> MetaStore<'a, E> {
    // ---------------- reads ----------------

    /// Every [`Remote`] on `replica`, sorted by name for stable
    /// output.
    pub async fn list_remotes(&self, replica: &Replica) -> Result<Vec<Remote>> {
        let mut rows: Vec<Remote> = self
            .branch
            .query()
            .select(Query::<Remote> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(replica.this().clone()),
                subject: Term::var("subject"),
                address: Term::var("address"),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::list_remotes: query failed")?;
        rows.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        Ok(rows)
    }

    /// Find a single [`Remote`] on `replica` by its local name.
    pub async fn find_remote(&self, replica: &Replica, name: &str) -> Result<Option<Remote>> {
        Ok(self
            .list_remotes(replica)
            .await?
            .into_iter()
            .find(|r| r.name.0 == name))
    }

    /// Every [`Branch`] whose `origin` is `replica`.
    ///
    /// # Polymorphism gotcha
    ///
    /// `Query::<Branch>` matches any entity with a `(name, origin)`
    /// attribute pair. [`Remote`] has the same pair (plus `subject`
    /// and `address`), so a `Remote` on `replica` shows up here
    /// alongside true local `Branch` entities. Callers that need to
    /// distinguish them should also call [`MetaStore::list_remotes`]
    /// and filter by entity. The schema-level query engine has no
    /// concept-tag discriminator today.
    pub async fn list_branches(&self, replica: &Replica) -> Result<Vec<Branch>> {
        let mut rows: Vec<Branch> = self
            .branch
            .query()
            .select(Query::<Branch> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(replica.this().clone()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::list_branches: query failed")?;
        rows.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        Ok(rows)
    }

    /// Every [`Branch`] on the meta branch — local *and*
    /// remote-side, across all replicas. Used to build the
    /// `entity → Branch` lookup table for resolving upstream
    /// targets (a `TrackingBranch.upstream` points at a remote-side
    /// branch entity, and you need to find the corresponding
    /// `Branch` to recover its name and which remote owns it).
    ///
    /// Same polymorphism caveat as [`MetaStore::list_branches`] —
    /// `Remote` entities surface here too. Callers typically build
    /// a `HashMap<Entity, Branch>` and skip any entity that's also
    /// in the remotes set.
    pub async fn list_all_branches(&self) -> Result<Vec<Branch>> {
        self.branch
            .query()
            .select(Query::<Branch> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::var("origin"),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::list_all_branches: query failed")
    }

    /// Every [`TrackingBranch`] owned by `replica`. The
    /// per-replica counterpart to [`MetaStore::find_upstream`]
    /// (single branch) and [`MetaStore::stale_tracking_for`]
    /// (single branch) — used when assembling a full repository
    /// view in one pass.
    pub async fn list_tracking_for_replica(
        &self,
        replica: &Replica,
    ) -> Result<Vec<TrackingBranch>> {
        self.branch
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::var("this"),
                upstream: Term::var("upstream"),
                origin: Term::from(replica.this().clone()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::list_tracking_for_replica: query failed")
    }

    /// The tracking link recorded for `local`, or `None` if the
    /// branch has no upstream.
    pub async fn find_upstream(&self, local: &Branch) -> Result<Option<TrackingBranch>> {
        let rows: Vec<TrackingBranch> = self
            .branch
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::from(local.this.clone()),
                upstream: Term::var("upstream"),
                origin: Term::from(local.origin.0.clone()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::find_upstream: query failed")?;
        Ok(rows.into_iter().next())
    }

    /// Resolve `local`'s upstream branch all the way to the
    /// `Remote` entity that owns it.
    ///
    /// Two-hop walk: tracking → upstream branch → branch.origin
    /// (a remote's entity for a remote-side branch). Returns `None`
    /// if `local` has no upstream or if the upstream branch is not
    /// recorded on this meta branch.
    pub async fn resolve_upstream_remote(&self, local: &Branch) -> Result<Option<Entity>> {
        let Some(tracking) = self.find_upstream(local).await? else {
            return Ok(None);
        };
        let upstream_branches: Vec<Branch> = self
            .branch
            .query()
            .select(Query::<Branch> {
                this: Term::from(tracking.upstream.0.clone()),
                name: Term::var("name"),
                origin: Term::var("origin"),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::resolve_upstream_remote: upstream-branch query failed")?;
        Ok(upstream_branches.into_iter().next().map(|b| b.origin.0))
    }

    /// Every [`Replica`] owned by `profile`. Used by profile-meta
    /// callers to enumerate the spaces a profile knows about.
    pub async fn list_replicas_for_profile(&self, profile: &Did) -> Result<Vec<Replica>> {
        let mut rows: Vec<Replica> = self
            .branch
            .query()
            .select(Query::<Replica> {
                this: Term::var("this"),
                name: Term::var("name"),
                subject: Term::var("subject"),
                profile: Term::from(profile.this()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::list_replicas_for_profile: query failed")?;
        rows.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        Ok(rows)
    }

    /// The replica `profile` already has for `subject`, or `None`.
    /// Load-bearing for `join` flows: "do I already have this
    /// space?".
    pub async fn find_replica_for_subject(
        &self,
        profile: &Did,
        subject: &Did,
    ) -> Result<Option<Replica>> {
        let rows: Vec<Replica> = self
            .branch
            .query()
            .select(Query::<Replica> {
                this: Term::var("this"),
                name: Term::var("name"),
                subject: Term::from(subject.this()),
                profile: Term::from(profile.this()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::find_replica_for_subject: query failed")?;
        Ok(rows.into_iter().next())
    }

    // ---------------- stale-cleanup helpers ----------------
    //
    // These return concepts to retract. Caller composes the
    // retract chain into a transaction; nothing is committed here.

    /// Tracking links currently recorded on `local`, suitable for
    /// retraction before re-asserting a fresh upstream. Returns
    /// `Vec` rather than `Option` because `set-upstream` semantics
    /// permit (defensively) retracting any number of stale links
    /// in one transaction.
    pub async fn stale_tracking_for(&self, local: &Branch) -> Result<Vec<TrackingBranch>> {
        self.branch
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::from(local.this.clone()),
                upstream: Term::var("upstream"),
                origin: Term::from(local.origin.0.clone()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::stale_tracking_for: query failed")
    }

    /// Concepts that must be retracted alongside a `Remote` to keep
    /// the meta branch internally consistent: the remote-side
    /// `Branch` concepts owned by it, plus any local
    /// `TrackingBranch` whose upstream lives on the remote.
    pub async fn dependents_of_remote(&self, remote: &Remote) -> Result<RemoteDependents> {
        let branches: Vec<Branch> = self
            .branch
            .query()
            .select(Query::<Branch> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(remote.this.clone()),
            })
            .perform(self.env)
            .try_vec()
            .await
            .context("MetaStore::dependents_of_remote: branch query failed")?;

        let mut tracking_links = Vec::new();
        for branch in &branches {
            let mut links: Vec<TrackingBranch> = self
                .branch
                .query()
                .select(Query::<TrackingBranch> {
                    this: Term::var("this"),
                    upstream: Term::from(branch.this.clone()),
                    origin: Term::var("origin"),
                })
                .perform(self.env)
                .try_vec()
                .await
                .context("MetaStore::dependents_of_remote: tracking query failed")?;
            tracking_links.append(&mut links);
        }

        Ok(RemoteDependents {
            branches,
            tracking_links,
        })
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end tests against an in-memory dialog repo.
    //!
    //! Every test goes through the full claim → assert → query
    //! cycle so the dialog wiring is exercised end-to-end. The
    //! pure builders (`replica`, `remote`, etc.) don't get their
    //! own tests here — they're covered by the corresponding
    //! `tests` modules in `replica.rs`, `remote.rs`, etc.

    use super::*;
    use dialog_credentials::Credential;
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};
    use dialog_repository::{Repository, SiteAddress};
    use dialog_varsig::did;

    /// Open a fresh in-memory repo and its meta branch, returning
    /// the operator + repository handle (kept alive for the test)
    /// and the opened meta branch.
    async fn fixture() -> (
        dialog_operator::Operator<dialog_storage::provider::storage::VolatileSpace>,
        Repository<Credential>,
        DialogBranch,
    ) {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let meta = repo
            .branch("meta")
            .open()
            .perform(&operator)
            .await
            .expect("open meta branch");
        (operator, repo, meta)
    }

    /// Convenience: a `SiteAddress` for tests. Any non-trivial
    /// bytes — the fact-roundtripping is what matters here, not
    /// the address contents.
    fn fake_address(tag: &str) -> SiteAddress {
        // We use a UCAN address with a placeholder URL. If
        // SiteAddress::Ucan is removed in a future dialog version,
        // swap to whatever the test-friendly variant is.
        use dialog_remote_ucan_s3::UcanAddress;
        UcanAddress::new(format!("https://example.invalid/{tag}/")).into()
    }

    #[tokio::test]
    async fn claim_replica_yields_consistent_meta_branch() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let (replica, meta_branch_concept) =
            store.claim_replica(profile.clone(), subject.clone(), "home");

        assert_eq!(meta_branch_concept.origin.0, *replica.this());
        assert_eq!(meta_branch_concept.name.0, meta.name());
    }

    #[tokio::test]
    async fn replica_round_trip() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let (replica, meta_branch_concept) =
            store.claim_replica(profile.clone(), subject.clone(), "home");

        meta.transaction()
            .assert(replica.clone())
            .assert(meta_branch_concept)
            .commit()
            .perform(&operator)
            .await
            .expect("commit replica anchor");

        // The replica should be discoverable by (profile, subject).
        let found = store
            .find_replica_for_subject(&profile, &subject)
            .await
            .expect("find_replica_for_subject");
        assert_eq!(found.as_ref().map(|r| r.this()), Some(replica.this()));

        // It should also surface in list_replicas_for_profile.
        let all = store
            .list_replicas_for_profile(&profile)
            .await
            .expect("list_replicas_for_profile");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].this(), replica.this());
    }

    #[tokio::test]
    async fn add_remote_round_trip_and_sorted_listing() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let replica = store.replica(profile, subject.clone(), "home");

        // Add two remotes whose insertion order != sort order, so
        // we exercise the sort-by-name guarantee.
        let backup_addr = fake_address("backup");
        let origin_addr = fake_address("origin");
        let backup = store.remote(&replica, "backup", subject.clone(), &backup_addr);
        let origin = store.remote(&replica, "origin", subject, &origin_addr);

        meta.transaction()
            .assert(replica.clone())
            .assert(backup)
            .assert(origin)
            .commit()
            .perform(&operator)
            .await
            .expect("commit remotes");

        let listed = store.list_remotes(&replica).await.expect("list_remotes");
        let names: Vec<&str> = listed.iter().map(|r| r.name.0.as_str()).collect();
        assert_eq!(names, vec!["backup", "origin"]);

        let found = store
            .find_remote(&replica, "origin")
            .await
            .expect("find_remote present");
        assert_eq!(found.map(|r| r.name.0), Some("origin".into()));

        let missing = store
            .find_remote(&replica, "nope")
            .await
            .expect("find_remote absent");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn upstream_resolution() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let replica = store.replica(profile, subject.clone(), "home");
        let addr = fake_address("origin");
        let remote = store.remote(&replica, "origin", subject, &addr);
        let local = store.branch_of(&replica, "main");
        let tracked = store.branch_of(&remote, "main");
        let tracking = store.tracking(&local, &tracked);

        // No upstream yet → both queries return None.
        let pre = store
            .find_upstream(&local)
            .await
            .expect("find_upstream pre");
        assert!(pre.is_none());

        meta.transaction()
            .assert(replica.clone())
            .assert(remote.clone())
            .assert(local.clone())
            .assert(tracked.clone())
            .assert(tracking.clone())
            .commit()
            .perform(&operator)
            .await
            .expect("commit upstream");

        let post = store
            .find_upstream(&local)
            .await
            .expect("find_upstream post")
            .expect("tracking link present");
        assert_eq!(post.upstream.0, tracked.this);

        let resolved = store
            .resolve_upstream_remote(&local)
            .await
            .expect("resolve_upstream_remote")
            .expect("upstream remote present");
        assert_eq!(resolved, remote.this);
    }

    #[tokio::test]
    async fn dependents_of_remote_lists_branches_and_tracking() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let replica = store.replica(profile, subject.clone(), "home");
        let addr = fake_address("origin");
        let remote = store.remote(&replica, "origin", subject, &addr);
        let local = store.branch_of(&replica, "main");
        let tracked = store.branch_of(&remote, "main");
        let tracking = store.tracking(&local, &tracked);

        meta.transaction()
            .assert(replica.clone())
            .assert(remote.clone())
            .assert(local)
            .assert(tracked.clone())
            .assert(tracking.clone())
            .commit()
            .perform(&operator)
            .await
            .expect("commit");

        let deps = store
            .dependents_of_remote(&remote)
            .await
            .expect("dependents_of_remote");

        assert_eq!(deps.branches.len(), 1);
        assert_eq!(deps.branches[0].this, tracked.this);
        assert_eq!(deps.tracking_links.len(), 1);
        assert_eq!(deps.tracking_links[0].this, tracking.this);
    }

    #[tokio::test]
    async fn list_all_branches_spans_local_and_remote_side() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let replica = store.replica(profile, subject.clone(), "home");
        let addr = fake_address("origin");
        let remote = store.remote(&replica, "origin", subject, &addr);
        let local = store.branch_of(&replica, "main");
        let tracked = store.branch_of(&remote, "main");

        meta.transaction()
            .assert(replica.clone())
            .assert(remote)
            .assert(local.clone())
            .assert(tracked.clone())
            .commit()
            .perform(&operator)
            .await
            .expect("commit");

        let all = store.list_all_branches().await.expect("list_all_branches");
        // Both the local and the remote-side branch must surface.
        // The query also picks up the `Remote` entity (same
        // `(name, origin)` shape), which the caller is expected to
        // filter — so we assert the *required* entries are present
        // rather than the exact length.
        let entities: Vec<_> = all.iter().map(|b| b.this.clone()).collect();
        assert!(entities.contains(&local.this));
        assert!(entities.contains(&tracked.this));
    }

    #[tokio::test]
    async fn list_tracking_for_replica_returns_all_replica_links() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let profile = did!("test:profile");
        let subject = did!("test:repo");
        let replica = store.replica(profile, subject.clone(), "home");
        let addr = fake_address("origin");
        let remote = store.remote(&replica, "origin", subject.clone(), &addr);
        let main = store.branch_of(&replica, "main");
        let main_tracked = store.branch_of(&remote, "main");
        let main_track = store.tracking(&main, &main_tracked);

        // Second branch with its own upstream, so we assert that
        // *every* tracking link surfaces — not just the first one.
        let dev = store.branch_of(&replica, "dev");
        let dev_tracked = store.branch_of(&remote, "dev");
        let dev_track = store.tracking(&dev, &dev_tracked);

        meta.transaction()
            .assert(replica.clone())
            .assert(remote)
            .assert(main)
            .assert(main_tracked)
            .assert(main_track.clone())
            .assert(dev)
            .assert(dev_tracked)
            .assert(dev_track.clone())
            .commit()
            .perform(&operator)
            .await
            .expect("commit");

        let links = store
            .list_tracking_for_replica(&replica)
            .await
            .expect("list_tracking_for_replica");
        assert_eq!(links.len(), 2);
        let upstreams: Vec<_> = links.iter().map(|l| l.upstream.0.clone()).collect();
        assert!(upstreams.contains(&main_track.upstream.0));
        assert!(upstreams.contains(&dev_track.upstream.0));
    }

    #[tokio::test]
    async fn replicas_isolated_per_profile() {
        let (operator, _repo, meta) = fixture().await;
        let store = MetaStore::new(&meta, &operator);

        let alice = did!("test:alice");
        let bob = did!("test:bob");
        let subject = did!("test:repo");

        let alice_replica = store.replica(alice.clone(), subject.clone(), "alice-home");
        let bob_replica = store.replica(bob.clone(), subject.clone(), "bob-home");

        meta.transaction()
            .assert(alice_replica.clone())
            .assert(bob_replica.clone())
            .commit()
            .perform(&operator)
            .await
            .expect("commit replicas");

        let alice_seen = store
            .list_replicas_for_profile(&alice)
            .await
            .expect("list alice");
        assert_eq!(alice_seen.len(), 1);
        assert_eq!(alice_seen[0].this(), alice_replica.this());

        let bob_seen = store
            .list_replicas_for_profile(&bob)
            .await
            .expect("list bob");
        assert_eq!(bob_seen.len(), 1);
        assert_eq!(bob_seen[0].this(), bob_replica.this());
    }
}
