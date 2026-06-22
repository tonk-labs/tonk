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

/// Request to mint a repository invite.
///
/// Asserted transiently when the user submits the share form (a
/// `<form onsubmit=tonk:invite>` in the standard library). The worker
/// handler generates a fresh membership keypair, delegates the
/// repository's access to its DID, asserts a durable [`Authorization`]
/// (the public delegation chain) into storage, and asserts the private
/// seed as a [`Credential`] into the reactor's session overlay (never
/// replicated). The share view joins the two via `tonk:invitation` and
/// assembles the final URL.
///
/// The command carries only the click's `time` — no audience, no
/// secret. The repository to delegate is read from the command's origin
/// (`CommandEnv::origin`), the branch the commit landed in. The
/// timestamp makes each click a distinct transient so repeated Share
/// clicks reliably re-fire the handler and rotate the credential.
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

/// Toggle background sync for the origin repo's replica.
///
/// Dispatched when the sync chip is clicked. Carries only a timestamp so
/// each click is a distinct transient (re-firing the handler); the handler
/// reads the replica's current `auto-sync` preference and flips it. The
/// repo is read from the command origin, the profile from the worker — the
/// two that key the replica.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct PauseSync {
    /// The command entity (a fresh id per click).
    pub this: Entity,
    /// The click event's timestamp — distinguishes one click from the next
    /// so the transient re-fires.
    pub time: crate::domain::command::invite::TimeStamp,
    /// Per-command marker (read from the pause form's `data-pause-sync`) that
    /// gives `PauseSync` an attribute no other command carries — so this
    /// transient does NOT also decode as `tonk:invite` (which shares the same
    /// `{this, time}` shape). See [`crate::domain::command::pause_sync::PauseSync`].
    pub marker: crate::domain::command::pause_sync::PauseSync,
}

/// `PauseSync` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (flips the replica's durable `auto-sync` preference).
impl Command for PauseSync {
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
/// seed of the membership keypair, **keyed by the membership DID**
/// (`this`). Asserted into the reactor's session overlay — never written
/// to the branch tree, never replicated — so the secret stays out of
/// storage while remaining queryable by the share view.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Credential {
    /// The membership DID — the same entity its [`Authorization`] is
    /// keyed by, so `tonk:invitation` joins them.
    pub this: Entity,
    /// The base58 ed25519 seed (`#` fragment).
    pub seed: crate::domain::credential::Seed,
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
