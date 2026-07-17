//! Per-concept attribute types in the `xyz.tonk.*` sub-domains.
//!
//! Each concept owns its own attribute namespace
//! (`xyz.tonk.replica`, `xyz.tonk.branch`, `xyz.tonk.remote`) so
//! its descriptor never matches entities of another shape — a
//! `Branch:` query would otherwise return [`Remote`] entities
//! since both have a `name` and an `origin` claim under the
//! shared `xyz.tonk` namespace.
//!
//! [`TrackingBranch`] reuses the `xyz.tonk.branch` namespace
//! because a tracking branch *is* a local branch with one extra
//! relation; its entities should still surface in a `branch:`
//! query.
//!
//! [`Remote`]: crate::Remote
//! [`TrackingBranch`]: crate::TrackingBranch

// The `#[derive(Attribute)]` macro generates helper types and
// associated functions without doc comments. Suppress the
// crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Attribute;
use dialog_repository::SiteAddress;

/// Attributes that live on [`Replica`] entities only.
///
/// [`Replica`]: crate::Replica
pub mod replica {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Profile(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Kind(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.replica")]
    pub struct Status(pub Entity);
}

/// Attributes for the `tonk/sync` concept — a replica's sync state.
///
/// Two orthogonal entity-valued fields, both keyed on the replica entity
/// (the `tonk/sync` fact's `this`), on the profile meta branch — private,
/// per-device-per-space, never replicated:
///
/// - `enabled` — the DURABLE pause *preference* (`sync:active` /
///   `sync:paused`). The user toggles it; the service worker reads it to
///   decide whether to sync this replica. Survives a worker restart.
/// - `status` — the live *observation* (`sync:synced` / `sync:syncing` /
///   `sync:offline`), written to the OVERLAY by the sweep (transient).
///
/// Keeping them separate avoids overloading one value: the chip reads
/// `enabled` for paused-vs-running and `status` for the running detail.
pub mod sync {
    use super::{Attribute, Entity};

    /// Whether auto-sync is on for a replica — `true` syncing, `false` paused.
    ///
    /// A boolean preference, not an entity URI: pause is simply on or off. Lives
    /// on the replica entity (profile meta branch, private), so pausing on this
    /// device never reaches other members. Cardinality one — toggling
    /// supersedes the prior value rather than accumulating.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.sync")]
    #[cardinality(one)]
    pub struct Enabled(pub bool);

    /// The live sync *observation* — one of many variants (`sync:idle` /
    /// `sync:pending` / `sync:offline` / `sync:local` / `sync:paused`). An
    /// entity URI because it is multi-valued, unlike the boolean preference.
    /// Written to the `state:here` overlay so the sealed chip reads it.
    /// Cardinality one, so the chip's fold always sees the latest, never a
    /// stale accumulated value.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.sync")]
    #[cardinality(one)]
    pub struct Status(pub Entity);
}

/// Attributes for the `tonk:site` concept — a tab's location and matched route.
///
/// Keyed on the per-tab `site` entity (the `X-Tonk-Site` value, a `site:<uuid>`
/// parsed to an [`Entity`]), stamped by the service worker onto the
/// Level-0-resolved branch's overlay. All cardinality one: a navigation
/// re-stamps the same entity and supersedes, so the site always reflects the
/// tab's latest location + route. Route models pick whichever of these they
/// need (e.g. `replica`) as their own fields, so they resolve on the site entity.
///
/// [`Entity`]: dialog_artifacts::Entity
pub mod site {
    use super::{Attribute, Entity};

    /// The matched document path on this site.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Path(pub String);

    /// The document fragment (URL hash) on this site.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Anchor(pub String);

    /// The space (repository name) the tab is on — the `did:key:…` routing key
    /// parsed from the URL's `/space/{segment}` prefix.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Space(pub String);

    /// The active branch name the tab is on — the `{branch}` component parsed
    /// from the space segment (`{branch}@{name}`, defaults to `"main"`).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Branch(pub String);

    /// The active replica entity for this site (this device's replica of the
    /// space the tab is on).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Replica(pub Entity);

    /// The matched route entity (the route-table entry that matched the path).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Route(pub Entity);

    /// The matched route's concept — the model the shell mounts on the site.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.site")]
    #[cardinality(one)]
    pub struct Concept(pub Entity);
}

/// Attributes for the durable `tonk:route` table the SW reads to build its
/// matchit router: a path pattern → the route model to mount.
pub mod route {
    use super::{Attribute, Entity};

    /// The axum/matchit path pattern, fed to `matchit::insert`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.route")]
    #[cardinality(one)]
    pub struct Path(pub String);

    /// The route model to mount when this path matches.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.route")]
    #[cardinality(one)]
    pub struct Concept(pub Entity);
}

/// Attributes for transient *command* concepts — the effect triggers
/// dispatched to typed-Rust handlers after a commit. A command is a
/// plain concept marked transient; these are the fields its triggers
/// carry.
pub mod command {
    use super::Attribute;

    /// The space name read from the Add Space form's submit event:
    /// `event.currentTarget.elements.name.value` (the `<wa-input
    /// name="name">` inside the `<form onsubmit=space/create>`).
    ///
    /// The `the:` is a `dom.event.*` read-path so the notation event
    /// layer populates it from the form on submit. The handler decodes
    /// the command by this same attribute — form source and handler
    /// decode agree on one attribute. Written kebab-case
    /// (`current-target`); the event layer converts to `currentTarget`
    /// at read time. The struct is named `Value` because the attribute
    /// is `…elements.name/value` (domain + `/value`).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dom.event.current-target.elements.name")]
    pub struct Value(pub String);

    /// The remote URL read from a space form's submit event:
    /// `event.currentTarget.elements.remote.value` (the `<wa-input
    /// name="remote">` inside the create / enable-sync forms).
    ///
    /// Single word `remote` (not `remote-url`) deliberately: every
    /// path segment is kebab→camel-cased at read time, so a hyphen
    /// would force the input's `name` to be `remoteUrl`. Keeping it one
    /// word lets the form field and the read-path agree on `remote`.
    ///
    /// Optional in practice: an empty input coerces to `""` (the input
    /// element still resolves, so the field is never *unresolved*), and
    /// the handler reads `""` as "no remote — local-only".
    pub mod remote {
        use super::Attribute;

        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.elements.remote")]
        pub struct Value(pub String);
    }

    /// Attributes the `tonk:load` command carries.
    ///
    /// Unlike the other commands these are NOT `dom.event` read-paths: a
    /// `<tonk-site>` asserts this transient programmatically (not from a form
    /// submit), supplying the values directly. The handler stamps the tab's
    /// `tonk:site` onto the command's `this` (the `site:<uuid>` the `<tonk-site>`
    /// minted) on the origin branch's overlay — the same work the legacy `/site`
    /// endpoint did, now triggered through the regular transact path so a nested
    /// `<tonk-site>` routes via its ancestor `<tonk-repository>`/`<tonk-branch>`
    /// context with no special fetch.
    pub mod load {
        use super::Attribute;

        /// The route path the tab is at (e.g. `/`, `/inspector`). The handler
        /// matches it against the origin branch's `route!` table. Named so the
        /// attribute is `xyz.tonk.site/path` — the same attribute the stamped
        /// `Site` records the path under, so the command and the fact align.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.site")]
        pub struct Path(pub String);
    }

    /// Attributes the `tonk:invite` command reads from its submit event.
    ///
    /// The membership keypair is generated by the worker handler (so its
    /// private seed can be written to the session overlay), so the
    /// command carries no audience DID — only the click's timestamp,
    /// which makes each invocation a distinct transient so repeated Share
    /// clicks reliably re-fire the handler (and rotate the credential).
    pub mod invite {
        use super::super::Entity;
        use super::Attribute;

        /// The event timestamp, read from the share form's submit event
        /// (`event.timeStamp`, a `DOMHighResTimeStamp` — a double, so the
        /// field is `f64`/`Float`). The struct is named `TimeStamp` so the
        /// derived attribute is `dom.event/time-stamp`, matching the
        /// command's `the:`.
        #[derive(Attribute, Clone, PartialEq, PartialOrd)]
        #[domain("dom.event")]
        pub struct TimeStamp(pub f64);

        /// A per-command marker read from a `data-invite` attribute on the
        /// share form. Its sole purpose is to give `tonk:invite` an attribute
        /// no other command carries: a transient command is matched by which
        /// attributes it carries, and `tonk:invite` + `tonk:pause-sync` would
        /// otherwise share an identical `{this, time}` shape and BOTH decode
        /// from one transient (so pausing also minted an invite). A distinct
        /// marker per command makes their shapes differ, so each transient
        /// decodes as exactly one command. The derived attribute is
        /// `dom.event.current-target.dataset/invite`.
        /// An `Entity`, not a `String`: the marker value (`tonk:invite`) has a
        /// `:`, and the worker's untagged `Value` decode reads any `:`-bearing
        /// string as an `Entity`. A `String` field would then fail to decode
        /// the fact (`Entity(tonk:invite)` ≠ Text).
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct Invite(pub Entity);

        /// The target space DID — the repository to mint the invite for.
        /// Lets `tonk:invite` be dispatched from the profile branch: the
        /// FAB's routeless share claim asserts this attribute so the
        /// worker's `InviteHandler` can read the target from the raw facts
        /// (`invite_space_from_facts`) instead of the dispatch origin
        /// (`CommandEnv::origin`, empty for a routeless profile-branch
        /// dispatch).
        ///
        /// NOT a field on [`crate::command::Invite`]: every existing space's
        /// `tonk:invite` descriptor is frozen without it (see that type's
        /// doc), so it stays a fact the handler reads opportunistically,
        /// mirroring `pause_sync::Space`'s intent without being a matched
        /// concept field. The derived attribute is `xyz.tonk.invite/space`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.invite")]
        pub struct Space(pub Entity);
    }

    /// Attributes the `tonk:pause-sync` command carries.
    pub mod pause_sync {
        use super::super::Entity;
        use super::Attribute;

        /// The per-command marker — `tonk:pause-sync`'s distinct attribute, so
        /// it never shares a shape with `tonk:invite` (see
        /// [`super::invite::Invite`]). An `Entity` for the same reason as the
        /// invite marker. The derived attribute is
        /// `dom.event.current-target.dataset/pause-sync`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct PauseSync(pub Entity);

        /// The target space DID (its subject `did:key`). The pause affordance
        /// (the FAB's `<ui-sync-status>`) carries this so the command names the
        /// space to pause explicitly, rather than the handler reading it off
        /// the dispatch origin. That lets the command be defined and dispatched
        /// on the PROFILE branch — the FAB depends on nothing seeded per-space.
        /// The derived attribute is `xyz.tonk.pause-sync/space`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.pause-sync")]
        pub struct Space(pub Entity);
    }

    /// Attributes the `tonk/rename-repository` command carries when the FAB
    /// dispatches it from the PROFILE branch.
    pub mod rename_repository {
        use super::super::Entity;
        use super::Attribute;

        /// The new repository name, read from the chip's `<tonk-editable>` on
        /// commit. The derived attribute is
        /// `dom.event.current-target/value`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target")]
        pub struct Value(pub String);

        /// The target space DID — the repository to rename. Read by the handler
        /// in place of the dispatch origin, so the command can be defined and
        /// dispatched on the PROFILE branch and the FAB depends on nothing
        /// seeded per-space. Mirrors `pause_sync::Space`. The derived attribute
        /// is `xyz.tonk.rename-repository/space`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.rename-repository")]
        pub struct Space(pub Entity);

        /// Per-command marker. Decoding matches on which ATTRIBUTES are
        /// present, never on their values — so a same-named `Rename` marker
        /// here and in `command::rename` would both derive
        /// `dom.event.current-target.dataset/rename`, making this command's
        /// attribute set a strict subset of `profile/rename`'s and letting a
        /// repo-rename transient decode as BOTH commands (the bug this type
        /// name fixes: renaming a space's repository was also renaming the
        /// user's profile). What keeps the two shapes disjoint is a
        /// DISTINCT ATTRIBUTE, not a distinct marker VALUE — same precedent
        /// as `command::remove::Remove` (`space/remove`'s `data-remove`,
        /// deliberately not `data-subject`, so it can't also decode every
        /// rename). The derived attribute is
        /// `dom.event.current-target.dataset/rename-repository`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct RenameRepository(pub Entity);
    }

    /// Attributes the `profile/rename` command reads from the identity
    /// chip's `<tonk-editable>` commit event.
    pub mod rename {
        use super::super::Entity;
        use super::Attribute;

        /// The new name, read from `event.currentTarget.value` on commit
        /// (blur/Enter) — the same read-path the repo-title editable uses.
        /// The struct is named `Value` so the attribute is
        /// `dom.event.current-target/value`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target")]
        pub struct Value(pub String);

        /// Per-command marker read from the editable's `data-rename`
        /// attribute. Gives `profile/rename` an attribute the declarative
        /// `tonk/rename-repository` transient does NOT carry, so a
        /// repo-title edit (which also writes `current-target/value`)
        /// never also decodes as a `ProfileRename`. An `Entity` because
        /// the marker value (`tonk:profile`) carries a `:` — see the
        /// invite marker note. Derived attribute:
        /// `dom.event.current-target.dataset/rename`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct Rename(pub Entity);
    }

    /// Attributes the `space/remove` command reads from its submit event.
    pub mod remove {
        use super::super::Entity;
        use super::Attribute;

        /// The subject DID of the space to remove, read from the Hub
        /// confirm form's `data-remove` attribute. Deliberately NOT
        /// `dataset/subject`: the declarative `tonk/rename-repository`
        /// transient (core.yaml) already carries `dataset/subject`, and
        /// a remove command matched on `subject` alone would ALSO decode
        /// every rename — deleting the space being renamed. The
        /// distinctly named attribute is both the payload and the
        /// command's unique shape, so no separate marker field is
        /// needed. An `Entity` because the value (a did:key) carries a
        /// `:` — see the invite marker note. Derived attribute:
        /// `dom.event.current-target.dataset/remove`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct Remove(pub Entity);
    }

    /// Attributes the `tonk:join` command reads from the `<tonk-page>`
    /// `mount` event's `detail` — a flat, URL-shaped record (fields mirror
    /// the DOM `URL` interface). The service worker can't see the `#hash`,
    /// so `<tonk-page>` reads it page-side and delivers it (with the
    /// `search`) in the event detail.
    ///
    /// The command reads the whole `search` + `hash` strings (always
    /// present — `URL.search`/`URL.hash` are `""` when empty, never
    /// `undefined`) rather than individual `searchParams`: a missing
    /// optional param (e.g. `remote`) would read as `undefined` and the
    /// event extractor aborts the command on that. The handler parses the
    /// reassembled URL with `Invite::parse_url`, which already handles the
    /// optional remote.
    pub mod join {
        use super::Attribute;

        /// The full query string incl. the leading `?` (faithful to
        /// `URL.search`), from `detail.search`. Carries `access` and the
        /// optional `remote`; the handler parses it.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.detail")]
        pub struct Search(pub String);

        /// The invite's `#seed` fragment incl. the leading `#` (faithful to
        /// `URL.hash`, the handler strips it), from `detail.hash`. The part
        /// only the page can see.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.detail")]
        pub struct Hash(pub String);
    }
}

/// Attributes on the transient self-identity overlay the topbar chip
/// reads (`state:self`). Overlay-only — never persisted, never replicated.
pub mod identity {
    use super::{Attribute, Entity};

    /// The self profile DID (feeds `<tonk-sigil did=>`). Derived
    /// attribute: `xyz.tonk.identity/did`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.identity")]
    pub struct Did(pub Entity);

    /// The self display name (the editable chip label). Derived
    /// attribute: `xyz.tonk.identity/name`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.identity")]
    pub struct Name(pub String);
}

/// Attributes on the profile's own identity facts, written to the
/// profile's meta branch (private, never replicated) and keyed by the
/// profile DID entity.
pub mod profile {
    use super::Attribute;

    /// The member's chosen display name override. Absent until the user
    /// renames themselves; `tonk-worker` falls back to a deterministic
    /// `petname` when it is missing. Cardinality-one (last write wins).
    /// The struct is named `DisplayName` so the derived attribute is
    /// `xyz.tonk.profile/display-name`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.profile")]
    pub struct DisplayName(pub String);
}

/// Attributes that live on a repository's own `tonk/repository`
/// concept — the repository's self-describing name, stored on its
/// content branch and keyed by the subject DID.
///
/// [`RepositoryName`]: crate::RepositoryName
pub mod repo {
    use super::Attribute;

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.repo")]
    pub struct Name(pub String);
}

/// Attributes on the durable `tonk:authorization` concept — the
/// delegation chain a `tonk:invite` handler minted, keyed by the
/// membership DID it was issued to. The chain is a scoped capability,
/// not a secret, so it replicates like any fact. The matching private
/// seed lives on the overlay-only [`credential`] concept and is joined
/// into the invite URL by `tonk:invitation`.
pub mod authorization {
    use super::Attribute;

    /// The base58-encoded delegation chain — the `?access=` parameter of
    /// the invite URL the view assembles.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.authorization")]
    pub struct Proof(pub String);

    /// The UCAN access-service endpoint for sync, when the repository
    /// advertises a remote — the optional `&remote=` parameter suffix.
    /// Empty when the repository is local-only.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.authorization")]
    pub struct Remote(pub String);
}

/// Attributes on the `tonk:credential` concept — the private ed25519
/// seed of a membership principal, and the finished invite URL built
/// from it. Asserted only into the reactor's session overlay (never
/// replicated, never written to the branch tree), so the secret stays
/// out of storage. `tonk:invitation` joins it with [`authorization`] so
/// the share view can render the link.
pub mod credential {
    use super::Attribute;

    /// The base58-encoded ed25519 keypair seed — the `#` fragment of the
    /// invite URL.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.credential")]
    pub struct Seed(pub String);

    /// The complete invite URL, assembled by the mint handler and
    /// shortened when the shortcut service answers.
    ///
    /// Secret-bearing: the seed rides in its `#` fragment. It belongs on
    /// this concept precisely because this concept is overlay-only, so
    /// the URL inherits the same never-replicated guarantee as [`Seed`]
    /// rather than needing one of its own.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.credential")]
    #[cardinality(one)]
    pub struct Link(pub String);
}

/// Attributes on the overlay-only `tonk:join/status` concept — the state
/// of an in-flight join attempt at the fixed `tonk:join/status` entity.
/// On success the whole fact is retracted and the durable space record is
/// asserted instead; a failure leaves `status: failed` (+ `reason`/`kind`)
/// in the overlay, session-scoped and never replicated.
pub mod join {
    use super::{Attribute, Entity};

    /// The attempt's state: `tonk:pending` while claiming, `tonk:failed`
    /// on error. (Success retracts the fact rather than setting a value.)
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.join")]
    pub struct Status(pub Entity);

    /// A human-readable failure message, set only when `status: failed`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.join")]
    pub struct Reason(pub String);

    /// The failure class: `malformed` | `audience-mismatch` | `claim-failed`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.join")]
    pub struct Kind(pub String);
}

/// Attributes that live on [`Branch`] entities (and
/// [`TrackingBranch`], which extends `Branch`).
///
/// [`Branch`]: crate::Branch
/// [`TrackingBranch`]: crate::TrackingBranch
pub mod branch {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Origin(pub Entity);

    /// The upstream branch a local branch is tracking. Direction-
    /// explicit counterpart to [`Origin`]: asserting
    /// `local -upstream-> remote_branch` records that the local
    /// branch tracks the remote branch.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.branch")]
    pub struct Upstream(pub Entity);
}

/// Attributes that live on [`Membership`] entities only.
///
/// [`Membership`]: crate::Membership
pub mod membership {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Member(pub Entity);

    /// The invitation a membership was claimed through — the
    /// [`InvitedVia`] stamp's payload.
    ///
    /// [`InvitedVia`]: crate::InvitedVia
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Invitation(pub Entity);

    /// The member's role in the space — `tonk:founder` for the
    /// creator, `tonk:member` for everyone who joined via an invite.
    /// The [`MemberRole`] stamp's payload. Cardinality one — a member
    /// has a single role, and the join path is first-wins so a founder
    /// who reclaims their own invite is never demoted.
    ///
    /// [`MemberRole`]: crate::MemberRole
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    #[cardinality(one)]
    pub struct Role(pub Entity);

    /// A member's self-asserted display name for the repository —
    /// the [`MemberName`] stamp's payload.
    ///
    /// [`MemberName`]: crate::MemberName
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.membership")]
    pub struct Name(pub String);
}

/// Attributes that live on [`Invitation`] entities only.
///
/// [`Invitation`]: crate::Invitation
pub mod invitation {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Inviter(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation")]
    pub struct Audience(pub Entity);
}

/// Attributes that live on [`Remote`] entities only.
///
/// [`Remote`]: crate::Remote
pub mod remote {
    use super::{Attribute, Entity, SiteAddress};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Name(pub String);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Origin(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Subject(pub Entity);

    /// Serialized [`SiteAddress`] bytes — the opaque payload a
    /// remote uses to locate a peer.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote")]
    pub struct Address(pub Vec<u8>);

    impl Address {
        /// Encode a [`SiteAddress`] as dag-cbor bytes.
        pub fn encode(address: &SiteAddress) -> Self {
            let bytes = serde_ipld_dagcbor::to_vec(address)
                .expect("SiteAddress is serde-serializable and dag-cbor-compatible");
            Self(bytes)
        }

        /// Decode the stored dag-cbor bytes back into a
        /// [`SiteAddress`].
        pub fn decode(
            &self,
        ) -> Result<SiteAddress, serde_ipld_dagcbor::DecodeError<std::convert::Infallible>>
        {
            serde_ipld_dagcbor::from_slice(&self.0)
        }
    }
}
