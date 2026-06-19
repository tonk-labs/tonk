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

/// Request to mint a repository invite delegated to a
/// browser-generated audience DID.
///
/// Asserted transiently when the user submits the share form (a
/// `<form onsubmit=tonk/invite>` in the standard library). A
/// `<tonk-credential>` element generates an ephemeral keypair in the
/// browser, fills the form's `audience` input with its public
/// `did:key`, and keeps the private seed in the DOM. So the command
/// carries only a *public* DID — never a secret. The worker handler
/// delegates the repository's access to that DID and asserts an
/// `invitation` fact keyed by the DID; the view reads the resulting
/// (non-secret) delegation chain back and assembles the final URL
/// locally, joining it with the seed it still holds.
///
/// `audience` is read from `elements.audience.value`. The repository to
/// delegate is *not* a command field: the handler reads it from the
/// command's origin (`CommandEnv::origin`) — the branch the commit
/// landed in — so the form needs no `data-subject` stamp.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Invite {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The browser-generated audience DID, read from the form's
    /// `audience` input.
    pub audience: crate::domain::command::invite::Value,
}

/// `Invite` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (delegates + asserts the `invitation` fact).
impl Command for Invite {
    type Input = Self;
    type Output = ();
}

/// The durable fact a `tonk/invite` handler asserts: the delegation
/// chain it minted, **keyed by the audience DID** (`this`). The share
/// view queries `<tonk-display model=invitation entity={audience}>` to
/// read `access` back and assemble the final URL.
///
/// Storing this is safe: a delegation chain is a scoped capability, not
/// a secret. The secret (the ephemeral private seed) is held only by the
/// browser's `<tonk-credential>` and joined into the URL there.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Invitation {
    /// The audience DID the invite was issued to — the entity the share
    /// view addresses by `entity={audience}`.
    pub this: Entity,
    /// The base58 delegation chain (`?access=`).
    pub access: crate::domain::invitation::Access,
    /// The sync remote endpoint (`&remote=`), empty when local-only.
    pub remote: crate::domain::invitation::Remote,
}
