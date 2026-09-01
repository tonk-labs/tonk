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

/// Attributes for the account-level space directory — one entry per
/// space, shared by every device on the account.
///
/// Distinct from the `xyz.tonk.replica` namespace on purpose: replica
/// rows are per-device (their entity hashes the device profile in),
/// and now that profile main syncs through the account every device's
/// rows land everywhere. A directory that queried replica attributes
/// would list one row per device per space. These attributes hang on
/// the repository's own entity instead, so all devices converge on
/// one entry.
pub mod space {
    use super::{Attribute, Entity};

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    pub struct Subject(pub Entity);

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    pub struct Status(pub Entity);

    /// Whether this device holds a local replica of the space. Written
    /// to the profile-main OVERLAY only — device-local, never
    /// replicated — so the Hub can style a directory row it cannot
    /// open locally (the hollow, replicate-on-first-visit space).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    #[cardinality(one)]
    pub struct Local(pub bool);

    /// The account providing this space with the access service. Its
    /// PRESENCE is the record that the space is provisioned; the sync
    /// engine retracts it when the service answers that the subject is
    /// no longer served, returning the row to local-only. The value is
    /// the providing account's DID entity — stable across replicas, so
    /// every device asserts the identical fact and the record converges
    /// (a timestamp here would give each writer its own value).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    #[cardinality(one)]
    pub struct Provider(pub Entity);

    /// The space's display name, mirrored into the account directory
    /// (the content-branch copy stays the editable source of truth) so
    /// every device can label a space it has not replicated yet.
    /// Written at create and updated by the rename command.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    pub struct Name(pub String);

    /// When this space was founded, unix seconds.
    ///
    /// Written once at creation and never updated, so it records the
    /// founding rather than the most recent mount. A space that arrived
    /// by invite has none: joining is not founding, and the absence is
    /// what distinguishes the two.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    #[cardinality(one)]
    pub struct FoundedAt(pub u64);

    /// The profile that founded the space.
    ///
    /// The account is already implied — the directory belongs to it —
    /// so this records WHICH DEVICE created the space, which is
    /// otherwise lost: the ownership delegation's audience is the
    /// account, and the founding device leaves no other trace.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.space")]
    #[cardinality(one)]
    pub struct FoundedBy(pub Entity);
}

/// Attributes describing a device authorization — an
/// `account -> profile` powerline as a device list presents it.
///
/// These hang off the DELEGATION's own entity (its blob hash), not a
/// separate record: the delegation IS the authorization, so a parallel
/// row could disagree with the proof it describes. Dialog already
/// decomposes issuer/audience/subject/expiration onto that entity;
/// these are the fields it does not carry.
///
/// Namespaced `xyz.tonk.device` rather than `xyz.tonk.authorization`,
/// which already means an invite's access proof. One namespace holding
/// both would make "authorization" ambiguous between a device link and
/// a share link.
/// Attributes of a passkey that can recover an account, keyed on the
/// custody DID its PRF output derives.
pub mod recovery {
    use super::Attribute;

    /// The credential id the authenticator returns, at creation and on
    /// every assertion. What an assertion names to select this credential.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.recovery")]
    #[cardinality(one)]
    pub struct CredentialId(pub String);

    /// Unix seconds at credential creation — when Tonk ran the ceremony,
    /// not anything the authenticator signs.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.recovery")]
    #[cardinality(one)]
    pub struct CreatedAt(pub u64);

    /// The browser and operating system where creation ran, e.g. `Chrome
    /// on macOS`. Never the password manager or storage provider —
    /// WebAuthn does not expose those reliably.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.recovery")]
    #[cardinality(one)]
    pub struct CreatedOn(pub String);

    /// The WebAuthn `user.name` this credential was created with — what a
    /// passkey manager lists the entry under.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.recovery")]
    #[cardinality(one)]
    pub struct Name(pub String);

    /// The WebAuthn `user.displayName`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.recovery")]
    #[cardinality(one)]
    pub struct DisplayName(pub String);
}

pub mod device {
    use super::{Attribute, Entity};

    /// When the device was linked, unix seconds.
    ///
    /// Distinct from the delegation's `notBefore`: that bounds validity,
    /// this records the act. A re-issued link with the same validity
    /// window still gets a new creation time.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.device")]
    #[cardinality(one)]
    pub struct CreatedAt(pub u64);

    /// Human label for the device, e.g. "Chrome on macOS".
    ///
    /// The same string the account ceremony sends onward as
    /// `deviceName`, kept here so a device list renders offline instead
    /// of requiring a round trip to the account service.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.device")]
    #[cardinality(one)]
    pub struct Title(pub String);

    /// Why the delegation exists, e.g. `case:device-link`.
    ///
    /// An entity label rather than text, so a typo cannot silently read
    /// as a different reason.
    ///
    /// Duplicated from the delegation's signed `meta` because `meta` is
    /// NOT decomposed into facts — it rides inside the envelope and
    /// cannot be queried. The signed copy stays authoritative; this one
    /// exists so a list can filter without opening envelopes.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.device")]
    #[cardinality(one)]
    pub struct Reason(pub Entity);
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

    /// The notebook title typed into the index's heading switcher.
    ///
    /// Its own attribute (`…detail/created-title`), NOT the
    /// `detail/title` a retitle carries: decode does not consider concept
    /// identity, so two transients of the same shape both decode from one
    /// event — every rename would also create a notebook.
    pub mod notebook {
        use super::Attribute;

        /// The title a create carries.
        ///
        /// The event detail key is `createdTitle`: every path segment is
        /// kebab→camel-cased at read time, so the hyphen here becomes a
        /// capital there. A detail key written `created-title` never
        /// matches, and the command silently fails to decode.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.detail")]
        pub struct CreatedTitle(pub String);

        /// The draft's whole document, so the notebook that gets created
        /// keeps what the author already wrote under the heading.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.detail")]
        pub struct CreatedBody(pub String);
    }

    /// The address read from the registration form's submit event:
    /// `event.currentTarget.elements.email.value` (the `<wa-input
    /// name="email">` inside `<form onsubmit=account/register>`).
    ///
    /// Same read-path convention as [`Value`]: one word, so the input's
    /// `name` and the attribute segment agree without kebab→camel
    /// conversion getting in the way.
    /// Marks a transient as an account registration, and not the
    /// address lookup that shares its shape.
    ///
    /// `CheckEmail` and `RegisterAccount` were both `{this, email}`, and
    /// decode does not consider concept identity — so every keystroke's
    /// lookup ALSO decoded as a registration, and the worker asked the
    /// page to run a passkey ceremony while the user was still typing.
    ///
    /// An `Entity`, not a `String`: the value (`tonk:register-account`)
    /// has a `:`, and the worker's untagged `Value` decode reads any
    /// `:`-bearing string as an `Entity`.
    pub mod register {
        use super::super::Entity;
        use super::Attribute;

        /// The marker only a registration carries.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct RegisterAccount(pub Entity);
    }

    pub mod email {
        use super::Attribute;

        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.elements.email")]
        pub struct Value(pub String);
    }

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

    /// The revocation relay read from the same submit event as the
    /// remote: `event.currentTarget.elements.revocation.value` (the
    /// hidden `<input name="revocation">` that
    /// `<tonk-default-remote relay-field="revocation">` fills).
    ///
    /// Its own module so the struct can be named `Value`, like every
    /// other control read here. The name is load-bearing: the struct
    /// name IS the attribute's last segment, and that segment is the JS
    /// property the event layer reads off the control. A `RevocationUrl`
    /// here would mint `…elements.revocation/revocation-url` and send the
    /// extractor after `form.elements.revocation.revocationUrl` —
    /// `undefined`, which aborts the whole command (no claim, no
    /// `preventDefault`) rather than degrading to a blank field.
    pub mod revocation {
        use super::Attribute;

        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.elements.revocation")]
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

    /// Attributes the `tonk:enable-sync` command carries.
    ///
    /// The share control dispatches this routelessly when a user accepts the
    /// offer to turn sync on, so the target space and the endpoint both travel
    /// on the transient rather than being inferred from a dispatch origin.
    pub mod enable_sync {
        use super::super::Entity;
        use super::Attribute;

        /// The submit event's timestamp. Makes each acceptance a distinct
        /// transient, and is echoed back on any refusal so the share control
        /// can match a result to the click that caused it.
        #[derive(Attribute, Clone, PartialEq, PartialOrd)]
        #[domain("dom.event")]
        pub struct TimeStamp(pub f64);

        /// The marker giving this command an attribute no other command
        /// carries, so a transient decodes as exactly one command. Same role
        /// as [`super::invite::Invite`]; the derived attribute is
        /// `dom.event.current-target.dataset/enable-sync`.
        ///
        /// An `Entity`, not a `String`: the value (`tonk:enable-sync`) has a
        /// `:`, and the worker's untagged `Value` decode reads any `:`-bearing
        /// string as an `Entity`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct EnableSync(pub Entity);

        /// The space to attach the remote to.
        ///
        /// An `Entity` for the same reason as [`EnableSync`]: the value is a
        /// `did:key:…`, and the worker's untagged `Value` decode reads any
        /// `:`-bearing string as an `Entity`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Space(pub Entity);

        /// The UCAN access-service endpoint to attach as `origin`.
        ///
        /// Read from the raw facts rather than being a matched field: a URL
        /// round-trips through JSON and the worker's untagged `Value` decode
        /// picks `Entity` for any string with a `:`, so a `String`-typed field
        /// would never decode one. The handler tolerates both representations,
        /// the same way `remote_from_facts` does for `CreateSpace`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Remote(pub String);

        /// Explicit immutable-artifact relay attached beside the remote.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct RevocationUrl(pub String);

        /// Present when the caller wants an invite minted once the remote is
        /// attached. Absent means attach only.
        ///
        /// An `Entity` for the same reason as [`EnableSync`]: the sentinel
        /// value (`tonk:share`) carries a `:`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enable-sync")]
        pub struct Share(pub Entity);
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

    /// Attributes of the `member/promote` command, dispatched by the FAB's
    /// roster once the page has minted the admin hop.
    pub mod enroll {
        use super::Attribute;

        /// The address to register the account under. Absent on a
        /// re-enrollment, where the account's recorded address stands:
        /// derived attribute `xyz.tonk.enroll/email`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Email(pub String);

        /// The account-signed deposits a passkey ceremony minted, hex,
        /// joined by commas. Empty on a resend, where the worker chains
        /// a device-issued set through the `root -> device` grant
        /// instead.
        ///
        /// One string rather than a list because a command's fields are
        /// scalars, and these travel together or not at all.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Deposits(pub String);

        /// The passkey custody space's DID, which the carried recovery
        /// invocation acts on: derived attribute
        /// `xyz.tonk.enroll/custody`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Custody(pub String);

        /// The custody space's consent to being provisioned by this
        /// account, hex: derived attribute `xyz.tonk.enroll/consent`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Consent(pub String);

        /// The pre-signed cell write the ceremony minted, hex: derived
        /// attribute `xyz.tonk.enroll/recovery`.
        ///
        /// Enrollment verifies it and activation performs it, so a
        /// signup finishes in one act rather than leaving custody to a
        /// step that can be missed.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Recovery(pub String);

        /// The sealed account secret the recovery write publishes, hex:
        /// derived attribute `xyz.tonk.enroll/sealed`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.enroll")]
        pub struct Sealed(pub String);
    }

    /// Attributes of the `account/resend-activation` command, dispatched
    /// by the account panel's resend button while activation is pending.
    pub mod resend {
        use super::Attribute;

        /// The click's timestamp — distinguishes one press from the
        /// next so the transient re-fires, and gives this command an
        /// attribute no other command carries: derived attribute
        /// `xyz.tonk.resend-activation/at`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.resend-activation")]
        pub struct At(pub u64);
    }

    pub mod promote {
        use super::super::Entity;
        use super::Attribute;

        /// The DID the member's membership is keyed on. Derived attribute:
        /// `xyz.tonk.promote/member`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.promote")]
        pub struct Member(pub Entity);

        /// The space the promotion is in: dispatched routeless from the FAB,
        /// the command names its target rather than firing on it.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.promote")]
        pub struct Space(pub Entity);

        /// The hop the page minted under the passkey, `promoter-account ->
        /// member` over the space at `/`, as base58 of the serialized chain.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.promote")]
        pub struct Chain(pub String);
    }

    /// Attributes of the `member/expel` command.
    pub mod expel {
        use super::super::Entity;
        use super::Attribute;

        /// The DID of the member to remove, read from the roster row's
        /// `data-expel`. Distinct from `dataset/remove`, which removes a
        /// space from this device. Derived attribute:
        /// `dom.event.current-target.dataset/expel`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.current-target.dataset")]
        pub struct Expel(pub Entity);
    }

    /// Attributes the `tonk:join` command reads from `<tonk-page>`'s
    /// `mount` event. The page delivers the complete URL because the
    /// service worker cannot observe its fragment.
    pub mod join {
        use super::Attribute;

        /// Complete invite URL from `detail.href`.
        #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("dom.event.detail")]
        pub struct Href(pub String);
    }
}

/// Attributes on the transient overlay row answering "is this address
/// registered?" for the registration form.
///
/// Overlay-only, deliberately. The form asks as the user types, so a
/// durable fact per answer would write a row per keystroke into a branch
/// that syncs. The overlay is per-session and unreplicated, which is
/// what a question about a half-typed address deserves.
pub mod email_status {
    use super::Attribute;

    /// The address the answer is about, so a stale answer for an
    /// address the user has since edited is recognisable as stale.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.email-status")]
    #[cardinality(one)]
    pub struct Address(pub String);

    /// What the access service said: `unregistered` (create an
    /// account), `active` (sign in), `pending` (sign in, then the
    /// waiting screen), `suspended` (terminal), `invalid` (not an
    /// address at all), or `unavailable` (the service could not be
    /// reached, which is not an answer about the address).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.email-status")]
    #[cardinality(one)]
    pub struct State(pub String);
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

/// Attributes on the device-local roster of profiles this browser knows,
/// kept in the registry profile's repository on a branch that never syncs.
/// One entity per profile storage name; the account, provider, and email
/// stamps are absent for a local workspace.
pub mod roster {
    use super::Attribute;

    /// The storage name the profile opens under: the activation handle.
    ///
    /// The only fact about a profile that lives on the device rather than on
    /// that profile's own account branch. Display name and address are read
    /// from there; copies here could only go stale.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.roster")]
    #[cardinality(one)]
    pub struct Name(pub String);
}

/// Root-owned account state replicated through the hidden account repository.
pub mod account {
    use super::{Attribute, Entity};

    /// The account-wide display name. Cardinality-one merge semantics choose a
    /// deterministic winner when linked devices write concurrently.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct DisplayName(pub String);

    /// The account's registration state with the access service, as one
    /// of `Registered`, `Active`, or `Suspended`.
    ///
    /// A fact rather than a locally cached HTTP answer, so every device
    /// on the account reads the same status through an ordinary query
    /// and converges on it through sync. Cardinality-one: an account has
    /// one registration state, and concurrent linked-device writes
    /// converge deterministically rather than accumulating.
    ///
    /// A string rather than a typed enum because the value system stores
    /// scalars; [`tonk_account::customer::CustomerStatus`] round-trips
    /// through `as_str`/`parse`, so an unrecognised value read from a
    /// newer build parses as absent rather than as a wrong status.
    ///
    /// [`tonk_account::customer::CustomerStatus`]: https://docs.rs/tonk-account
    /// When enrollment recorded the address, unix seconds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct RegisteredAt(pub u64);

    /// When activation was observed, unix seconds. Its presence is what
    /// makes an account served.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct ActivatedAt(pub u64);

    /// When the service withdrew, unix seconds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct SuspendedAt(pub u64);

    /// The derived status label: `case:onboarding`, `case:registered`,
    /// `case:active`, or `case:suspended`. Never written directly — the
    /// deductive rules in `profile.yaml` conclude it.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct Status(pub Entity);

    /// When this device minted its onboarding account, unix seconds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct OnboardingMintedAt(pub u64);

    /// The local keypair holding the onboarding account.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct OnboardingCustodian(pub Entity);

    /// When a real account took over from the onboarding one, unix seconds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct OnboardingRetiredAt(pub u64);

    /// Why the service withdrew, in words a person can act on.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct SuspensionReason(pub String);

    /// The email address the account enrolled with.
    ///
    /// Recorded alongside the status so a device that never ran the
    /// enrollment still knows which address the activation link went to.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct CustomerEmail(pub String);

    /// The account's provider: the UCAN access-service endpoint this
    /// account is a customer of.
    ///
    /// Named for the account's relationship, not the space's plumbing. A
    /// space has a *remote* (an `origin` its `main` tracks); the account
    /// has a *provider*, which is who serves those remotes and who
    /// `/provider/add` provisions consumers under. A space's remote
    /// normally points at this address, but the two are different facts
    /// about different subjects.
    ///
    /// A fact rather than a value re-derived per call site: the address
    /// used to come from the page (`https://{origin}/ucan/`, filled into
    /// a hidden form field) and from the signed account descriptor, so
    /// two paths could disagree about where a space syncs. The service
    /// names it in the registration receipt, so every attach path reads
    /// one answer, and a device that never ran the registration reads
    /// the same one through sync.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct ProviderAddress(pub String);
    /// Where anything sealed for this account is addressed: the X25519
    /// public key as a `did:key:z6LS…` entity. Every device can seal to
    /// it; only a live passkey ceremony derives the private half and can
    /// open the result. Derived from the account secret, so rotation
    /// publishes a new one; cardinality-one because sealed rows name
    /// their own recipient and need no history here.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct SealedInbox(pub Entity);
}

/// Attributes of a seed sealed to an account, in the account space. One
/// one row per principal; see [`crate::SecretPrincipal`].
pub mod custody {
    use super::{Attribute, Entity};

    /// Who can open a sealed message: the X25519 `did:key` the bytes were
    /// sealed to, as an entity.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.secret")]
    #[cardinality(one)]
    pub struct To(pub Entity);

    /// Who sealed a message, when that is known and worth recording.
    /// Optional: most sealing has no meaningful sender, and naming one
    /// would invent an author the seal does not bind.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.secret")]
    #[cardinality(one)]
    pub struct Sender(pub Entity);

    /// The sealed bytes of a message.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.secret")]
    #[cardinality(one)]
    pub struct Message(pub Vec<u8>);

    /// What a sealed principal is: `tonk:space` for a space's signing key,
    /// `tonk:invite` for an invite principal's.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.secret")]
    #[cardinality(one)]
    pub struct Kind(pub Entity);

    /// The message whose plaintext is a principal's ed25519 seed.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.secret")]
    #[cardinality(one)]
    pub struct Seed(pub Entity);
}

/// Attributes that describe a repository on its content branch, keyed by the
/// subject DID. They travel with the repository rather than belonging to one
/// profile or replica.
///
/// [`RepositoryName`]: crate::RepositoryName
/// [`RepositoryAgents`]: crate::RepositoryAgents
pub mod repo {
    use super::Attribute;

    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.repo")]
    pub struct Name(pub String);

    /// Markdown agent context carried by the repository subject.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.repo")]
    #[cardinality(one)]
    pub struct Agents(pub String);
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

    /// The UCAN access-service endpoint for sync — the `&remote=` parameter
    /// suffix. Never empty: `run_invite` refuses to mint an invite (and so
    /// never asserts this) for a repository with no shareable remote, since
    /// one that carried no remote would strand its recipient in a space that
    /// can never fill.
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

/// Attributes on the overlay-only `tonk:share/blocked` fact — why a share
/// click could not mint an invite. Keyed on the space's subject entity, the
/// same entity [`crate::command::Credential`] is keyed by, so the share
/// control reads both off one subject.
///
/// Overlay-only, so it is session-scoped and never replicated: a refusal is
/// this device's answer to this click, not a property of the space.
/// Attributes on the per-space invite state the share control renders.
///
/// One row per space, replaced as the state moves. Overlay-only: the
/// url carries a secret seed in its fragment, and the row is a
/// per-session view of an in-flight request rather than a durable
/// record. The durable record of a *minted* invite is
/// [`crate::Invitation`], keyed on the delegation CID — a different
/// thing that happens to share the noun.
pub mod invite {
    use super::Attribute;
    use dialog_artifacts::Entity;

    /// Where this space's invite has got to.
    ///
    /// An entity, not a string: a string is a poor discriminator, and
    /// this follows [`crate::domain::replica::Status`]
    /// (`tonk:blank` / `tonk:initialized`). One of
    /// `invite:requested`, `invite:granted`, `invite:suspended`,
    /// `invite:unshareable`.
    ///
    /// A denial IS a status — there is no separate reason field. Two
    /// fields would encode one fact and make illegal states
    /// representable (granted-with-a-reason, denied-without-one).
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invite")]
    #[cardinality(one)]
    pub struct Status(pub Entity);

    /// The finished invite URL, present only once granted.
    ///
    /// Optional, so one row covers every state without a sentinel: a
    /// request in flight simply has no url yet.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invite")]
    #[cardinality(one)]
    pub struct Url(pub String);
}

pub mod share {
    use super::Attribute;

    /// The refusal class: `not-synced` | `unshareable-remote` |
    /// `attach-failed`. Only `not-synced` is repairable by attaching a
    /// remote, so it is the only one that offers the prompt.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Blocked(pub String);

    /// The sentence shown to the user.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Detail(pub String);

    /// The timestamp of the command this refusal answers, echoed back from
    /// the transient that triggered it.
    ///
    /// Load-bearing. The fact is cardinality-one on the subject, so it
    /// lingers in the overlay and replays on every resubscribe; the share
    /// control acts only on a refusal whose timestamp matches the click it
    /// is currently holding a clipboard write for, and ignores every other
    /// frame. That is what makes the fact safe to leave in place instead of
    /// retracting it on the next success.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.share")]
    #[cardinality(one)]
    pub struct Time(pub f64);
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

/// Operational metadata attached to an [`Invitation`](crate::Invitation)
/// without changing the backward-readable invitation record.
pub mod invitation_execution {
    use super::Attribute;

    /// Stable audience mode: `open` or `scoped`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.invitation-execution")]
    pub struct Kind(pub String);
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

/// Operational metadata attached to a [`Remote`](crate::Remote) without
/// changing the backward-readable remote record.
pub mod remote_execution {
    use super::Attribute;

    /// Explicit endpoint that accepts immutable revocation artifacts.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.remote-execution")]
    pub struct RevocationUrl(pub String);
}
