//! [`Replica`] — this device's view of a repository.

// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types and associated functions without doc comments. Suppress
// the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_repository::SiteAddress;
use dialog_varsig::Did;

use crate::Branch;
use crate::Remote;
use crate::domain::remote::Address;
use crate::domain::replica::{Kind, Name, Profile, Status, Subject};
use crate::domain::space::{Status as SpaceDirStatus, Subject as SpaceDirSubject};
use crate::domain::sync::{Enabled, Status as SyncStatusAttr};
use crate::prelude::*;

/// A replica — this device's view of a specific repository.
///
/// The `this` entity is content-derived from the `(profile, subject)`
/// pair (see [`Origin`]), so:
///
/// - two devices holding the same profile converge on the same
///   replica entity for a given repository, and
/// - different profiles produce different replica entities even when
///   pointing at the same repository.
///
/// The concept lives on the repository's meta branch. It is
/// typically the first thing asserted when a repository is opened
/// locally: writing the replica record announces "this profile has
/// a local view of this repository" and anchors subsequent
/// per-replica facts (branches, upstream configuration, etc.).
///
/// # Redundant by design
///
/// The [`Subject`] and [`Profile`] attributes carry the same two DIDs
/// that went into hashing the entity. The redundancy is intentional:
/// the hash is a one-way function, so without these attributes it
/// would be impossible to answer queries like "find the replica this
/// profile has for subject X" — you would need to know both inputs
/// upfront and re-hash to locate the entity. The stored attributes
/// make the relationships discoverable through normal queries.
///
/// # Constructing
///
/// [`Replica::new`] takes the profile and subject DIDs and derives
/// every field consistently:
///
/// ```no_run
/// use dialog_varsig::Did;
/// use tonk_schema::Replica;
/// # fn example(profile: Did, subject: Did) -> Replica {
/// Replica::new(profile, subject)
/// # }
/// ```
///
/// # No name
///
/// The replica is a membership *index* — it records that this profile
/// has a view of this repository, not what the repository is called.
/// The repository's display name lives in its own `tonk/repository`
/// concept on its content branch, which syncs across devices; reading
/// it from there keeps every device's view of the name current,
/// whereas a profile-side cache only ever updated on the renaming
/// device.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Replica {
    /// The replica's entity. Derived from `(profile, subject)` via
    /// [`Origin::this`].
    pub this: Entity,
    /// Reference to the repository this replica is a view of.
    pub subject: Subject,
    /// Reference to the profile that owns this replica.
    pub profile: Profile,
    /// What this replica points at: [`Self::PROFILE`] for the
    /// profile's own self-replica (`subject == profile`),
    /// [`Self::REPOSITORY`] for a space the profile has joined or
    /// created. Lets a query select only real spaces (e.g. the
    /// Hub picker) without re-deriving the self-replica from the
    /// profile entity.
    pub kind: Kind,
}

/// The account-level directory entry for a space — what the Hub lists.
///
/// Keyed on the repository's OWN entity (`subject.this()`), not a
/// `(profile, subject)` hash, so every device on the account asserts
/// the same entry and they converge to one row per space. The
/// per-device [`Replica`] records keep existing alongside — they
/// carry this device's mount, its sync stamps, and dialog's own
/// replica facts — but the directory is account state: removing an
/// entry removes the space from the whole account's Hub.
///
/// `status` mirrors the replica's seeding status so the Hub's install
/// animation works; a later assert supersedes (cardinality one). The
/// display name is deliberately absent — the card renders it live
/// from the space's own branch.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Space {
    /// The repository's own entity.
    pub this: Entity,
    /// The repository's DID as an entity, for URL binding in views.
    pub subject: SpaceDirSubject,
    /// Seeding status — the same `tonk:blank` / `tonk:initialized`
    /// markers [`Replica`] uses.
    pub status: SpaceDirStatus,
}

impl Space {
    /// A directory entry for `subject` with the given seeding status.
    pub fn new(subject: &Did, status: Status) -> Self {
        Self {
            this: subject.this(),
            subject: SpaceDirSubject(subject.this()),
            status: SpaceDirStatus(status.0),
        }
    }
}

/// The device-local "replicated here" marker for a directory row —
/// overlay-only (see [`domain::space::Local`]).
///
/// [`domain::space::Local`]: crate::domain::space::Local
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceLocal {
    /// The repository's own entity — the directory entity.
    pub this: Entity,
    /// Whether a local replica exists on this device.
    pub local: crate::domain::space::Local,
}

impl SpaceLocal {
    /// A locality stamp for `subject`.
    pub fn new(subject: &Did, local: bool) -> Self {
        Self {
            this: subject.this(),
            local: crate::domain::space::Local(local),
        }
    }
}

/// The account-directory copy of a space's display name, on the
/// directory entity. The space's own content branch remains the
/// editable source of truth (`tonk/repository`); this mirror exists so
/// a device that has not replicated the space can still label it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceName {
    /// The repository's own entity — the directory entity.
    pub this: Entity,
    /// The mirrored display name.
    pub name: crate::domain::space::Name,
}

impl SpaceName {
    /// A name mirror for `subject`.
    pub fn new(subject: &Did, name: impl Into<String>) -> Self {
        Self {
            this: subject.this(),
            name: crate::domain::space::Name(name.into()),
        }
    }
}

/// The account currently providing this space with the access
/// service, on the directory entity.
///
/// Its presence IS the provisioned state: `/provider/add` succeeded
/// for this subject, so its remote is served and a share can skip the
/// provisioning ceremony. The sync engine retracts it when the gate
/// answers that the subject is no longer served (a `Declined` refusal
/// retrying cannot clear), which returns the space to local-only.
///
/// The value is the providing ACCOUNT's DID entity rather than a
/// timestamp: every replica that observes the provisioning asserts the
/// identical fact, so the record converges, where per-writer
/// timestamps would give each device its own value.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceProvider {
    /// The repository's own entity — the directory entity.
    pub this: Entity,
    /// The account (customer) whose registration serves this space.
    pub provider: crate::domain::space::Provider,
}

impl SpaceProvider {
    /// Record that `provider` provides `subject`.
    pub fn new(subject: &Did, provider: &Did) -> Self {
        Self {
            this: subject.this(),
            provider: crate::domain::space::Provider(provider.this()),
        }
    }
}

/// Who founded a space and when, on the directory entity.
///
/// Founding is distinct from mounting: [`record_space_mount`] runs for
/// both created and joined spaces, so this is asserted only on the
/// creation path. A directory row without it is a space this account
/// joined rather than made, which is what lets the Hub tell "spaces I
/// created" from "spaces shared with me" without consulting delegations.
///
/// Deliberately NOT on the ownership delegation's entity. The delegation
/// is authoritative for *authority*; this is description, and it belongs
/// on the entity the Hub already renders — otherwise every row query
/// would join through `dialog.ucan/subject` to show a creation date.
///
/// [`record_space_mount`]: https://docs.rs/tonk-worker
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceFounded {
    /// The repository's own entity — the directory entity.
    pub this: Entity,
    /// When the space was created, unix seconds.
    pub founded_at: crate::domain::space::FoundedAt,
    /// The profile that created it.
    pub founded_by: crate::domain::space::FoundedBy,
}

impl SpaceFounded {
    /// A founding stamp for `subject`, created by `profile` at `at`.
    pub fn new(subject: &Did, profile: &Did, at: u64) -> Self {
        Self {
            this: subject.this(),
            founded_at: crate::domain::space::FoundedAt(at),
            founded_by: crate::domain::space::FoundedBy(profile.this()),
        }
    }
}

/// A [`Replica`] as it was written before the [`Kind`] field
/// existed: `name` + `subject` + `profile`, no `kind`.
///
/// A `Replica` query requires every field, so it won't match a
/// record that predates `kind`. `LegacyReplica` drops `kind` and
/// therefore matches *every* replica — kinded or not. Used only by
/// the `repo-vs-profile` migration to enumerate records that still
/// need stamping; new code should query [`Replica`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LegacyReplica {
    /// The replica's entity.
    pub this: Entity,
    /// Human-readable name for the repository on this replica.
    pub name: Name,
    /// Reference to the repository this replica is a view of.
    pub subject: Subject,
    /// Reference to the profile that owns this replica.
    pub profile: Profile,
}

/// The [`Kind`] of a replica as a standalone fact: just `this` and
/// `kind`.
///
/// Asserting one stamps a `kind` onto an existing replica entity
/// without re-asserting the whole [`Replica`] (which would require
/// re-deriving every other field). The migration uses it to
/// backfill `kind` on legacy records.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceKind {
    /// The replica entity being stamped.
    pub this: Entity,
    /// What the replica points at — see [`Replica::kind`].
    pub kind: Kind,
}

impl SpaceKind {
    /// A `kind` stamp for the given replica entity.
    pub fn new(this: Entity, kind: Kind) -> Self {
        Self { this, kind }
    }
}

/// The seeding [`Status`] of a replica as a standalone fact: just
/// `this` and `status`.
///
/// A repository's content branch is seeded asynchronously after the
/// replica is recorded, so the replica is first stamped
/// [`Replica::BLANK`] and flipped to [`Replica::INITIALIZED`] once the
/// seed completes. Asserting one stamps the status onto an existing
/// replica entity (cardinality one, so a later assert supersedes)
/// without re-asserting the whole [`Replica`]. The Hub reads it to
/// reflect the install state on each card.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceStatus {
    /// The replica entity being stamped.
    pub this: Entity,
    /// The replica's seeding status — see [`Replica::BLANK`] /
    /// [`Replica::INITIALIZED`].
    pub status: Status,
}

impl SpaceStatus {
    /// A `status` stamp for the given replica entity.
    pub fn new(this: Entity, status: Status) -> Self {
        Self { this, status }
    }
}

/// The auto-sync *preference* of a replica as a standalone fact: just
/// `this` and `enabled`.
///
/// The DURABLE half of `tonk/sync` — a boolean (`true` syncing, `false`
/// paused), committed to the profile meta branch and keyed on the replica
/// entity, so the service worker's background-sync loop reads it and skips a
/// paused replica, and so it survives a worker restart. Private — replica
/// records never replicate, so pausing on this device doesn't pause sync for
/// other members. Stamped onto the replica entity (cardinality one, a later
/// assert supersedes) without re-asserting the whole [`Replica`].
///
/// Separate from [`ReplicaSyncStatus`] (the transient observation) so each
/// resolves independently: the durable preference is always present, the
/// live status may lag — a single two-field concept would only resolve
/// once both existed (the join-status lesson).
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaSyncEnabled {
    /// The replica entity being stamped.
    pub this: Entity,
    /// Whether auto-sync is on (`true`) or paused (`false`) for this replica.
    pub enabled: Enabled,
}

impl ReplicaSyncEnabled {
    /// An `enabled` stamp for the given replica entity.
    pub fn new(this: Entity, enabled: bool) -> Self {
        Self {
            this,
            enabled: Enabled(enabled),
        }
    }
}

/// The live sync *status* of a replica as a standalone fact: just `this`
/// and `status`.
///
/// The OVERLAY half of `tonk/sync` — `sync:synced` / `sync:syncing` /
/// `sync:offline`, stamped by the sweep (transient, never persisted), a
/// live observation of how this replica's head compares to its upstream.
/// Keyed on the replica entity so it folds into the same subscription as
/// [`ReplicaSyncEnabled`]; the chip subscribes to the replica and renders
/// both.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaSyncStatus {
    /// The replica entity being stamped.
    pub this: Entity,
    /// The observed sync status — synced / syncing / offline.
    pub status: SyncStatusAttr,
}

impl ReplicaSyncStatus {
    /// A `status` stamp for the given replica entity.
    pub fn new(this: Entity, status: SyncStatusAttr) -> Self {
        Self { this, status }
    }
}

impl Replica {
    /// Build a replica concept from a profile DID and a subject DID.
    ///
    /// Derives `this` from `(profile, subject)` and fills in the
    /// `subject` and `profile` attributes from the same DIDs so
    /// every field is consistent with the entity hash. The display
    /// name is not stored here — see the type docs.
    pub fn new(profile: Did, subject: Did) -> Self {
        let kind = if subject == profile {
            Self::profile_kind()
        } else {
            Self::repository_kind()
        };
        Self::with_kind(profile, subject, kind).expect("built-in replica kind is valid")
    }

    /// Build a replica with one of the three recognized kinds.
    ///
    /// This is the explicit constructor for system replicas. It rejects
    /// arbitrary kind URIs so callers cannot accidentally create a fourth
    /// replica class that ordinary space enumeration does not understand.
    pub fn with_kind(profile: Did, subject: Did, kind: Kind) -> Option<Self> {
        if kind != Self::profile_kind()
            && kind != Self::repository_kind()
            && kind != Self::account_kind()
            && kind != Self::ledger_kind()
        {
            return None;
        }
        Some(Self {
            // Derive `this` via dialog's own `Replica` so a tonk `Replica` and a
            // dialog `Replica` for the same `(profile, subject)` are the SAME
            // entity. They are the same thing — this device's view of a repo —
            // and the library's `tonk/replica` / `tonk:binder` concepts key on
            // `dialog.replica/*`, so they must land on this entity. (Deriving a
            // separate `This::Replica`-tagged hash split them, which silently
            // put binder/replica facts on two different entities.)
            this: dialog_repository::schema::Replica::new(profile.clone(), subject.clone()).this,
            subject: Subject(subject.this()),
            profile: Profile(profile.this()),
            kind,
        })
    }

    /// Build the hidden account-system replica.
    pub fn account(profile: Did, subject: Did) -> Self {
        Self::with_kind(profile, subject, Self::account_kind())
            .expect("account replica kind is valid")
    }

    /// Build the replica for the service's own ledger space.
    ///
    /// The account holds a read grant over it and nothing more, so it
    /// is never a space in the Hub sense. Kinding it keeps it out of
    /// space enumeration for the same reason the account replica is
    /// kinded: the directory selects on `kind`, and an unkinded row
    /// would show up as a space the user could rename or leave.
    pub fn ledger(profile: Did, subject: Did) -> Self {
        Self::with_kind(profile, subject, Self::ledger_kind())
            .expect("ledger replica kind is valid")
    }

    /// `kind` URI for the profile's own self-replica.
    pub const PROFILE: &'static str = "tonk:profile";

    /// `kind` URI for a space (a repository the profile joined or
    /// created).
    pub const REPOSITORY: &'static str = "tonk:repository";

    /// `kind` URI for the hidden root-owned account repository.
    pub const ACCOUNT: &'static str = "tonk:account";

    /// `kind` URI for the service-owned ledger space the account reads.
    pub const LEDGER: &'static str = "tonk:ledger";

    /// The [`Kind`] for the profile's own self-replica.
    pub fn profile_kind() -> Kind {
        Kind(Self::PROFILE.parse().expect("tonk:profile parses"))
    }

    /// The [`Kind`] for a space.
    pub fn repository_kind() -> Kind {
        Kind(Self::REPOSITORY.parse().expect("tonk:repository parses"))
    }

    /// The [`Kind`] for the hidden account-system repository.
    pub fn account_kind() -> Kind {
        Kind(Self::ACCOUNT.parse().expect("tonk:account parses"))
    }

    /// The [`Kind`] for the service-owned ledger space.
    pub fn ledger_kind() -> Kind {
        Kind(Self::LEDGER.parse().expect("tonk:ledger parses"))
    }

    /// `status` URI for a freshly created replica whose content
    /// branch has not been seeded yet.
    pub const BLANK: &'static str = "tonk:blank";

    /// `status` URI for a replica whose content branch has been
    /// seeded.
    pub const INITIALIZED: &'static str = "tonk:initialized";

    /// The [`Status`] for a not-yet-seeded replica.
    pub fn blank_status() -> Status {
        Status(Self::BLANK.parse().expect("tonk:blank parses"))
    }

    /// The [`Status`] for a seeded replica.
    pub fn initialized_status() -> Status {
        Status(Self::INITIALIZED.parse().expect("tonk:initialized parses"))
    }

    /// `tonk/sync` `status` URI: the user paused auto-sync. The OVERLAY twin of
    /// the durable boolean `enabled = false` preference, so the chip can show
    /// `paused` (the durable bool lives on the replica, off the chip's branch).
    pub const PAUSED: &'static str = "sync:paused";

    /// The fixed entity the live sync `status` overlay is keyed on —
    /// `state:here`, a well-known singleton (like `tonk:join/status`) so the
    /// chip can subscribe without resolving this device's replica entity.
    /// One space is in scope per page (each sealed `/space` is its own
    /// guest), so a singleton suffices; the durable pause preference still
    /// lives per-replica. Defers the "scope the view to the local replica"
    /// question.
    pub const SYNC_STATE_HERE: &'static str = "state:here";

    /// The fixed entity the self-identity overlay is keyed on — a
    /// well-known singleton so the topbar chip can subscribe without
    /// resolving this device's membership entity. One space per page, so
    /// a singleton suffices (same rationale as `SYNC_STATE_HERE`).
    pub const SELF_STATE_HERE: &'static str = "state:self";

    /// `tonk/sync` `status` URI: up to date, nothing to do.
    pub const IDLE: &'static str = "sync:idle";

    /// `tonk/sync` `status` URI: a sync is in flight / due. One in-progress
    /// state — the worker holds a single lock for the whole pull+push, so the
    /// chip can't observe finer phases mid-sync anyway.
    pub const PENDING: &'static str = "sync:pending";

    /// `tonk/sync` `status` URI: a remote is configured but unreachable
    /// (network down, remote unavailable) — a real "offline" state.
    pub const OFFLINE: &'static str = "sync:offline";

    /// `tonk/sync` `status` URI: no remote is configured — the repo is
    /// local-only. Distinct from [`OFFLINE`](Self::OFFLINE) (a remote exists
    /// but can't be reached).
    pub const LOCAL: &'static str = "sync:local";

    /// Remote authorization was revoked; terminal until authority changes.
    pub const REVOKED: &'static str = "sync:revoked";

    /// Reconciliation conflicted with concurrent remote/local updates.
    pub const CONFLICT: &'static str = "sync:conflict";

    /// The configured service is temporarily unavailable.
    pub const UNAVAILABLE: &'static str = "sync:unavailable";

    /// The live `status` value for a paused replica — the overlay twin of the
    /// durable [`PAUSED`](Self::PAUSED) `enabled` preference, so the chip shows
    /// `paused` immediately without waiting for a status sweep (which a paused
    /// replica skips). Shares the URI with the durable value: status and
    /// enabled are different attributes, so the same `sync:paused` URI reads
    /// unambiguously on each.
    pub fn paused_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::PAUSED.parse().expect("sync:paused parses"))
    }

    /// The `status` value for an in-flight / due sync.
    pub fn pending_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::PENDING.parse().expect("sync:pending parses"))
    }

    /// The `status` value for an unreachable remote (a real offline state).
    pub fn offline_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::OFFLINE.parse().expect("sync:offline parses"))
    }

    /// The `status` value for revoked remote authority.
    pub fn revoked_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::REVOKED.parse().expect("sync:revoked parses"))
    }

    /// The `status` value for a reconciliation conflict.
    pub fn conflict_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::CONFLICT.parse().expect("sync:conflict parses"))
    }

    /// The `status` value for a temporarily unavailable service.
    pub fn unavailable_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::UNAVAILABLE.parse().expect("sync:unavailable parses"))
    }

    /// The `status` value for a local-only space. Also published when
    /// the service answers that the subject is no longer served — a
    /// remote nothing will serve is a remote in name only, and `local`
    /// is what drives the connect affordance that re-provisions it.
    pub fn local_status() -> SyncStatusAttr {
        SyncStatusAttr(Self::LOCAL.parse().expect("sync:local parses"))
    }

    /// Map a head-comparison [`SyncState`](crate::SyncState) to the
    /// replica's settled `sync` `status` value. `idle` when up to date,
    /// `pending` when there's drift (a reconcile is due), and `local` when
    /// there is no remote configured. (A reachable-but-failed remote is
    /// `offline`, published separately when the status fetch errors.)
    pub fn sync_status_attr(state: crate::SyncState) -> SyncStatusAttr {
        use crate::SyncState as S;
        let uri = match state {
            S::Synced => Self::IDLE,
            S::Behind | S::Ahead | S::Diverged => Self::PENDING,
            S::NoUpstream => Self::LOCAL,
        };
        SyncStatusAttr(uri.parse().expect("sync status uri parses"))
    }

    /// The replica's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }

    /// Create a [`Branch`] concept on this replica.
    ///
    /// `name` is anything convertible into a [`branch::Name`],
    /// matching the [`Branch::new`] signature.
    ///
    /// [`branch::Name`]: crate::domain::branch::Name
    pub fn branch(&self, name: impl Into<crate::domain::branch::Name>) -> Branch {
        Branch::new(self, name)
    }

    /// Create a [`Remote`] concept on this replica.
    ///
    /// `name` accepts anything convertible into [`Name`]; the
    /// [`SiteAddress`] is encoded into an [`Address`] internally
    /// (we can't surface that as a `From` impl without clashing
    /// with the blanket one the `Attribute` derive emits — see
    /// [`Address::encode`]).
    pub fn remote(
        &self,
        name: impl Into<crate::domain::remote::Name>,
        subject: Did,
        address: &SiteAddress,
    ) -> Remote {
        Remote::new(self, subject, Address::encode(address), name)
    }
}

impl AsRef<Entity> for Replica {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::did;

    #[test]
    fn same_origin_same_entity() {
        let a = Replica::new(did!("test:p"), did!("test:r"));
        let b = Replica::new(did!("test:p"), did!("test:r"));
        assert_eq!(a.this.to_string(), b.this.to_string());
    }

    #[test]
    fn different_profile_different_entity() {
        let a = Replica::new(did!("test:p1"), did!("test:r"));
        let b = Replica::new(did!("test:p2"), did!("test:r"));
        assert_ne!(a.this.to_string(), b.this.to_string());
    }

    #[test]
    fn subject_and_profile_reflect_inputs() {
        let profile = did!("test:profile-x");
        let subject = did!("test:repo-y");
        let replica = Replica::new(profile.clone(), subject.clone());
        assert_eq!(replica.profile.0.to_string(), profile.as_str());
        assert_eq!(replica.subject.0.to_string(), subject.as_str());
    }

    #[test]
    fn account_replica_keeps_the_same_origin_entity_but_a_distinct_kind() {
        let profile = did!("test:profile-x");
        let subject = did!("test:account-y");
        let account = Replica::account(profile.clone(), subject.clone());
        let space = Replica::new(profile, subject);

        assert_eq!(account.this, space.this);
        assert_eq!(account.kind, Replica::account_kind());
        assert_eq!(space.kind, Replica::repository_kind());
    }

    #[test]
    fn explicit_constructor_rejects_unknown_kinds() {
        let unknown = Kind("tonk:unknown".parse().unwrap());
        assert!(Replica::with_kind(did!("test:p"), did!("test:r"), unknown).is_none());
    }
}
