// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types without doc comments; suppress `missing_docs` here.
#![allow(missing_docs)]

//! Command concepts — transient effect triggers dispatched to
//! typed-Rust handlers in `tonk-worker` after a commit.
//!
//! A command is an ordinary [`Concept`] that is *asserted transiently*:
//! it triggers a handler and is swept from durable storage at the same
//! commit, so it fires exactly once and leaves no trace. The worker's
//! command registry matches a committed transient against these
//! concepts and runs the corresponding handler.
//!
//! These types only define the *shape* a command carries. The handler
//! that reacts to one lives in `tonk-worker`; the transient-ness is a
//! property of how the command is asserted (the
//! `dialog.concept/transient` marker), not of the type.

use dialog_artifacts::Entity;
use dialog_capability::Command;
use dialog_query::Concept;

use crate::domain::command::Value as SpaceName;

/// Request to create a new space (repository) by local name.
///
/// Asserted transiently when the user submits the Add Space form (a
/// `<form onsubmit=space/create>` defined in `profile.yaml`). The
/// notation event layer reads `name` from the form's
/// `elements.name.value` and POSTs the transient claim; the handler
/// records the replica (`status: blank`) so the Hub shows it
/// installing, then creates the repository, seeds the standard library,
/// and flips the status to `initialized`.
///
/// `name`'s attribute is a `dom.event.*` read-path so the same concept
/// the form asserts is the one the worker handler decodes — see
/// [`crate::domain::command::Value`].
///
/// Deliberately a single matched field. A command concept must keep
/// decoding against the descriptor an *older* version seeded — a profile
/// branch is seeded once and not re-seeded across versions, so its
/// `space/create` descriptor is frozen at the version that created it.
/// Adding a required field here would make the command silently fail to
/// match every such profile (the transient commits, no provider runs),
/// breaking all space creation.
///
/// The optional sync remote is therefore *not* a field here: the worker's
/// `CreateSpaceHandler` matches on `name` and reads the remote URL
/// directly from the transient's facts. It can't be a `String`-typed
/// concept field anyway — a URL round-trips through JSON and the worker's
/// untagged `Value` deserialization picks `Entity` for any string with a
/// `:`, so a `remote: String` field would never decode a URL. Reading the
/// artifact directly tolerates both `String` and `Entity`. The same
/// handler also serves the topbar's "Enable sync" form (which posts the
/// same `name`+`remote` shape against an existing space).
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateSpace {
    /// The command entity (a fresh id per invocation, derived by the
    /// worker from the predicate + payload).
    pub this: Entity,
    /// Local name for the new space, read from the form's `name` input.
    /// The create wizard supplies it from a hidden input carrying the
    /// `Untitled` sentinel (the user no longer types a name up front);
    /// the worker's handler uniquifies that to "Untitled N" against the
    /// existing space labels, and the user renames later.
    pub name: SpaceName,
}

/// `CreateSpace` is a [`dialog_capability::Command`]. Note the worker
/// registers a custom `CreateSpaceHandler` (not a plain `Provider`) so it
/// can read the optional remote from the facts; the `Command` impl is
/// kept for the decode/`Decode` machinery.
impl Command for CreateSpace {
    type Input = Self;
    type Output = ();
}

/// Load the requesting tab's site for its current path.
///
/// Asserted transiently by `<tonk-site>` (via the regular transact API) instead
/// of the legacy `POST /api/.../site` fetch. The element mints its own site
/// entity once (`site:<uuid>`) and supplies it as `this`, plus the route `path`.
/// The command rides the normal event path, so its ancestor `<tonk-repository>` /
/// `<tonk-branch>` annotate the origin repo/branch — a nested router stamps onto
/// the space's repo branch, the top-level one onto the profile branch, with no
/// special endpoint.
///
/// On navigation the same `<tonk-site>` re-asserts `tonk:load` with the SAME
/// `this` and a new `path`; the handler stamps the cardinality-one `tonk:site`
/// fields, which supersede in place, and the element's live subscription
/// re-renders — no teardown, no reload. Each `<tonk-site>` mints its own entity,
/// so two sites on one page (even on the same branch) never clobber.
///
/// The handler (`LoadHandler` in `tonk-worker`) does exactly what `register_site`
/// did: match `path` against the origin branch's `route!` table and stamp the
/// resolved [`crate::site::Site`] (plus captured route params) onto `this` in
/// that branch's overlay.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Load {
    /// The site entity to stamp (`site:<uuid>`), minted by the `<tonk-site>`.
    pub this: Entity,
    /// The route path the tab is at, matched against the origin branch's route
    /// table.
    pub path: crate::domain::command::load::Path,
}

/// `Load` is a [`dialog_capability::Command`]; its handler stamps the tab's
/// `tonk:site` onto the command's `this` from the origin branch.
impl Command for Load {
    type Input = Self;
    type Output = ();
}

/// Request to mint a repository invite.
///
/// Asserted transiently when the FAB's share control is clicked
/// (`<tonk-share>`, `tonk-fab`). The worker handler generates a fresh
/// membership keypair, delegates the repository's access to its DID,
/// asserts a durable [`Authorization`] (the public delegation chain) into
/// storage, and asserts the private seed as a [`Credential`] into the
/// reactor's session overlay (never replicated). The share view joins the
/// two via `tonk:invitation` and assembles the final URL.
///
/// Deliberately a minimal matched shape, like [`CreateSpace`]: a command
/// concept must keep decoding against the descriptor an *older* seeded
/// `core.yaml` carries, and every existing space's `tonk:invite` descriptor
/// is frozen at `{this, time, marker}` (no `space` field). A required
/// `space` field here would make those transients silently fail to match
/// (the transient commits, no handler runs) — see `CreateSpace`'s doc and
/// `docs/space-sync-remotes-and-launchpad.md` §3.1, which hit the identical
/// mistake with `CreateSpace.remote`.
///
/// The FAB's newer profile-dispatched share affordance (routeless, so
/// `CommandEnv::origin` is empty) still needs to name its target: it does
/// so by asserting the `xyz.tonk.invite/space` attribute on the same
/// transient WITHOUT it being a matched concept field — the worker's
/// `InviteHandler` reads it opportunistically from the raw facts
/// (`invite_space_from_facts`, mirroring `remote_from_facts`), falling back
/// to the dispatch origin when it's absent. The timestamp makes each click
/// a distinct transient so repeated Share clicks reliably re-fire the
/// handler and rotate the credential.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct Invite {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The submit event's timestamp — distinguishes one click from the
    /// next so the transient re-fires.
    pub time: crate::domain::command::invite::TimeStamp,
    /// Per-command marker (read from the share form's `data-invite`) that
    /// gives `Invite` an attribute no other command carries — so a
    /// `tonk:pause-sync` transient (identical `{this, time}` shape otherwise)
    /// does NOT also decode as an invite. See
    /// [`crate::domain::command::invite::Invite`].
    pub marker: crate::domain::command::invite::Invite,
}

/// `Invite` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (generates the keypair, delegates, asserts the
/// authorization + overlay credential).
impl Command for Invite {
    type Input = Self;
    type Output = ();
}

/// Toggle background sync for a space's replica.
///
/// Dispatched when the FAB's sync cap is alt/option-clicked. Carries the
/// target `space` (the DID to pause) and a timestamp so each click is a
/// distinct transient (re-firing the handler); the handler reads the
/// replica's current `auto-sync` preference for that space and flips it.
///
/// The `space` field is what lets this command live on and dispatch from the
/// PROFILE branch: the handler reads the target space from the command rather
/// than the dispatch origin, so the FAB's pause affordance needs no view or
/// command seeded on each space's own branch.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct PauseSync {
    /// The command entity (a fresh id per click).
    pub this: Entity,
    /// The click event's timestamp — distinguishes one click from the next
    /// so the transient re-fires.
    pub time: crate::domain::command::invite::TimeStamp,
    /// The target space DID — the replica to pause/resume. Read by the handler
    /// in place of the dispatch origin.
    pub space: crate::domain::command::pause_sync::Space,
    /// Per-command marker that gives `PauseSync` an attribute no other command
    /// carries — so this transient does NOT also decode as `tonk:invite` (which
    /// shares the same `{this, time}` shape). See
    /// [`crate::domain::command::pause_sync::PauseSync`].
    pub marker: crate::domain::command::pause_sync::PauseSync,
}

/// `PauseSync` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (flips the replica's durable `auto-sync` preference).
impl Command for PauseSync {
    type Input = Self;
    type Output = ();
}

/// Rename a space's repository from the FAB.
///
/// The space-side `tonk/rename-repository` rule (`core.yaml`) cannot consume a
/// claim dispatched on the profile branch, so this carries its target `space`
/// and is executed by a worker handler instead — the `PauseSync` pattern. That
/// is what lets the FAB's name chip depend on nothing seeded per-space.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct RenameRepository {
    /// The command entity (a fresh id per commit).
    pub this: Entity,
    /// The new name, read from the editable's value on commit.
    pub name: crate::domain::command::rename_repository::Value,
    /// The target space DID — the repository to rename.
    pub space: crate::domain::command::rename_repository::Space,
    /// Per-command marker distinguishing this from `profile/rename`, which
    /// shares the `{this, value}` shape.
    pub marker: crate::domain::command::rename_repository::Rename,
}

impl Command for RenameRepository {
    type Input = Self;
    type Output = ();
}

/// Request to rename the current profile (set the member display name).
///
/// Asserted transiently when the topbar identity chip's `<tonk-editable>`
/// commits. Carries the new `name` (read from `currentTarget.value`) and
/// a `marker` (`data-rename`) that distinguishes it from the declarative
/// `tonk/rename-repository` transient, which shares the `current-target/
/// value` attribute. The handler persists the override to the profile
/// meta branch and re-stamps `MemberName` on the origin space.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileRename {
    /// The command entity (a fresh id per commit).
    pub this: Entity,
    /// The new display name.
    pub name: crate::domain::command::rename::Value,
    /// Per-command marker (`data-rename="tonk:profile"`).
    pub marker: crate::domain::command::rename::Rename,
}

impl Command for ProfileRename {
    type Input = Self;
    type Output = ();
}

/// Request to remove a space from this device: retract its replica
/// record from the profile meta branch (the Hub row's source of
/// truth), detach it from the reactor/sync, and delete its local
/// storage.
///
/// Asserted transiently when the user confirms a Hub row's delete
/// overlay (`<form onsubmit=space/remove data-remove={subject}>` in
/// `profile.yaml`). Removal is device-local: a synced space can be
/// rejoined via an invite link; server-side data is untouched.
///
/// Deliberately a single matched field, like [`CreateSpace`], so an
/// older profile descriptor keeps decoding it. The field also doubles
/// as the command's distinct shape: `dataset/remove` is read by no
/// other command, whereas a `dataset/subject` field would also match
/// every `tonk/rename-repository` transient (which carries
/// `dataset/subject`) and turn each rename into a deletion — see
/// [`crate::domain::command::remove::Remove`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoveSpace {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The subject DID of the space to remove, from `data-remove`.
    pub subject: crate::domain::command::remove::Remove,
}

/// `RemoveSpace` is a [`dialog_capability::Command`]; the worker
/// registers a custom `RemoveSpaceHandler` (the work needs the profile
/// handle, the reactor cache, and storage — state the decoded command
/// doesn't carry).
impl Command for RemoveSpace {
    type Input = Self;
    type Output = ();
}

/// The durable fact a `tonk:invite` handler asserts: the public
/// delegation chain it minted, **keyed by the membership DID** (`this`).
///
/// Storing this is safe: a delegation chain is a scoped capability, not
/// a secret. The secret (the private seed) lives only on the
/// overlay-only [`Credential`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Authorization {
    /// The membership DID the invite was issued to.
    pub this: Entity,
    /// The base58 delegation chain (`?access=`).
    pub proof: crate::domain::authorization::Proof,
    /// The sync remote endpoint (`&remote=`), empty when local-only.
    pub remote: crate::domain::authorization::Remote,
}

/// The overlay-only fact a `tonk:invite` handler asserts: the private
/// seed of the membership keypair and the finished invite URL built from
/// it, **keyed by the membership DID** (`this`). Asserted into the
/// reactor's session overlay — never written to the branch tree, never
/// replicated — so the secrets stay out of storage while remaining
/// queryable by the share view.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Credential {
    /// The membership DID — the same entity its [`Authorization`] is
    /// keyed by, so `tonk:invitation` joins them.
    pub this: Entity,
    /// The base58 ed25519 seed (`#` fragment).
    pub seed: crate::domain::credential::Seed,
    /// The complete invite URL, shortened when the shortcut service
    /// answered. Carries the seed in its fragment, hence overlay-only.
    pub link: crate::domain::credential::Link,
}

/// Request to redeem an invite URL and join its space.
///
/// Asserted transiently when `<tonk-page>` fires its `mount` event on the
/// `/join` view (`<tonk-page onmount=tonk/join>`). The element reads the
/// page's location and delivers the parsed invite as the event `detail`;
/// this command picks the pieces it needs out of `detail`. The
/// `#fragment` (the seed) is the part the service worker can't see, so it
/// must come from the page through this command.
///
/// The handler reassembles the URL from `access` + `remote` + `fragment`,
/// parses + validates it, and claims — driving the overlay-only
/// `tonk:join/status` (pending → failed, or retract + durable space on
/// success).
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Join {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The full query string (incl. `?`), from `detail.search` — carries
    /// `access` and the optional `remote`. Read whole (not per-param) so a
    /// missing optional `remote` doesn't abort the command; the handler
    /// reassembles + parses it.
    pub search: crate::domain::command::join::Search,
    /// The `#seed` fragment (incl. `#`), from `detail.hash` — page-only.
    pub hash: crate::domain::command::join::Hash,
}

/// `Join` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (reassembles + claims the invite, drives `JoinStatus`).
impl Command for Join {
    type Input = Self;
    type Output = ();
}

/// The overlay-only fact tracking an in-flight join, at the fixed
/// `tonk:join/status` entity (`this`). Just `status` — `tonk:pending`
/// while claiming, `tonk:failed` on error — so this resolves the moment a
/// join starts (a concept that also required `reason`/`kind` would only
/// resolve once those exist, i.e. never in the pending state; see the
/// invite `tonk:invitation` join for the same all-fields-required
/// gotcha). On success the handler retracts this fact and asserts the
/// durable space record instead; the failure detail lives on the
/// separate [`JoinFailure`] concept at the same entity. Overlay-only, so
/// the Hub (durable replicas) never shows in-flight or failed joins.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JoinStatus {
    /// The fixed `tonk:join/status` entity.
    pub this: Entity,
    /// `tonk:pending` | `tonk:failed`.
    pub status: crate::domain::join::Status,
}

/// The failure detail for a join, at the same `tonk:join/status` entity —
/// asserted (overlay-only) alongside `status: tonk:failed`. A separate
/// concept from [`JoinStatus`] so the pending state (status only) still
/// resolves; the view reads this for the error message when `status` is
/// `tonk:failed`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JoinFailure {
    /// The fixed `tonk:join/status` entity (same as [`JoinStatus`]).
    pub this: Entity,
    /// Human-readable failure message.
    pub reason: crate::domain::join::Reason,
    /// Failure class: `malformed` | `audience-mismatch` | `claim-failed`.
    pub kind: crate::domain::join::Kind,
}
