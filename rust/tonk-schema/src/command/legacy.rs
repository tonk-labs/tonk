// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` here.
#![allow(missing_docs)]

//! The shape commands used to have — kept only so a branch seeded
//! before the change keeps working.
//!
//! A command is matched structurally: the set of attribute names a
//! transient carries is its whole identity. A branch that was seeded
//! with the old `command!:` descriptors still asserts these attributes,
//! and its pages still hold those descriptors, so retiring them
//! outright would break every space created before the migration.
//!
//! Nothing is written against these types. Each one has a
//! `From<legacy::X> for X` conversion beside the current shape in
//! [`super`], and each handler pairs the two through
//! [`dialog_reactor::Migrated`], which decodes either and hands the
//! handler the current shape. That is the whole compatibility surface:
//! deleting this module, its `From` impls, and one type parameter per
//! handler retires the old shape without touching a handler body.
//!
//! Two things these shapes have that the current ones do not, both
//! consequences of naming a field after the DOM read path that filled
//! it:
//!
//! - **`marker` fields.** Two verbs reading the same DOM path were the
//!   same concept, so several commands carry an otherwise pointless
//!   attribute purely to stay distinct from a sibling. Per-verb
//!   namespaces make that distinction structural, so the current shapes
//!   have none.
//! - **`prevent-default`.** A side effect spelled as a field. It stores
//!   no value, so a rule premise over it matches zero rows however
//!   successfully the command transacted; it now lives on the `event!:`
//!   declaration, where it is a property of the interaction rather than
//!   of the command.
//!
//! [`dialog_reactor::Migrated`]: https://docs.rs/dialog-reactor

use dialog_artifacts::Entity;
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

/// Create a notebook from the index's heading switcher, and drop the
/// author into it.
///
/// The handler does both halves: it writes the notebook and then posts a
/// `navigate` to the originating client. The navigation cannot happen in
/// the page, because the notebook's entity is derived when the fact is
/// written — the element that fired the command never learns it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateNotebook {
    /// The command entity, minted per invocation.
    pub this: Entity,
    /// The title typed into the heading.
    pub title: crate::domain::command::notebook::CreatedTitle,
    /// The draft's document, blocks and all.
    pub body: crate::domain::command::notebook::CreatedBody,
}

/// Ask whether an address is already registered, so the form can route
/// before anyone runs a ceremony.
///
/// Answers on the overlay as [`crate::EmailStatus`], not in a response
/// body: the form subscribes to that row and renders the branch it
/// names. Asserted as the user types, which is why the answer is
/// overlay-only.
///
/// Creating an account with an address that already has one runs the
/// whole WebAuthn ceremony and fails at the end, leaving an orphan
/// passkey in the authenticator. Asking first is what avoids that.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CheckEmail {
    /// The command entity, minted per invocation.
    pub this: Entity,
    /// The address to ask about, read from the form's `email` input.
    pub email: crate::domain::command::email::Value,
}

/// Register an account, from the form the registration overlay renders.
///
/// The page asserts this and then watches facts: `AccountCustomer`
/// appears once enrollment lands, and gains a provider at activation.
/// Nothing is read back from a response, because a command answers with
/// facts rather than a body.
///
/// The provider cannot finish this alone. Creating an account is a
/// WebAuthn ceremony, which needs a `window` and a user gesture, and the
/// service worker has neither; it asks the originating client to
/// authorize with a passkey and continues from what comes back.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegisterAccount {
    /// The command entity, minted per invocation.
    pub this: Entity,
    /// The address to register, read from the form's `email` input.
    pub email: crate::domain::command::email::Value,
    /// Per-command marker keeping this distinct from [`CheckEmail`],
    /// which is otherwise the same shape.
    ///
    /// Without it every keystroke's lookup also decoded as a
    /// registration, and a passkey prompt appeared while the user was
    /// still typing their address.
    pub marker: crate::domain::command::register::RegisterAccount,
}

/// `tonk:add-passkey`: seal the account under a second passkey.
///
/// The worker asks the page for both ceremonies — the passkey that
/// holds the account, and the new one — and re-seals the secret.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddPasskey {
    /// The command entity (a fresh id per click).
    pub this: Entity,
    /// The per-command marker; see [`crate::domain::command::add_passkey`].
    pub marker: crate::domain::command::add_passkey::AddPasskey,
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
/// `docs/evolving-command-concepts.md`, which records the same mistake with
/// `CreateSpace.remote`.
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

/// Attach a sync remote to an existing space, and optionally mint an invite
/// once it is attached.
///
/// Dispatched routelessly by the share control when a user accepts the offer
/// to turn sync on. `space`, `remote` and the `share` marker ride on the
/// transient as raw facts the handler reads directly — `remote` because a URL
/// cannot be a `String`-typed field (see
/// [`crate::domain::command::enable_sync::Remote`]), the other two for
/// symmetry with it.
///
/// This is deliberately NOT the `space/enable-sync` command seeded in
/// `core.yaml`: that one shares `CreateSpace`'s trigger attribute, so a
/// handler registered against it would fire alongside `CreateSpaceHandler`
/// and mint a new space instead of attaching to the existing one.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct EnableSync {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The acceptance timestamp — distinguishes one click from the next.
    pub time: crate::domain::command::enable_sync::TimeStamp,
    /// Per-command marker that keeps this command's shape distinct from
    /// every other transient's.
    pub marker: crate::domain::command::enable_sync::EnableSync,
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
    /// shares the `{this, value}` shape. A DISTINCT ATTRIBUTE (not a
    /// distinct marker value) is what keeps the shapes disjoint — see
    /// `domain::command::rename_repository::RenameRepository`'s doc.
    pub marker: crate::domain::command::rename_repository::RenameRepository,
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

/// Remove a member from the space this command fires in.
///
/// Asserted transiently by the roster row's expel form; the worker's
/// handler revokes the hop that admits the member under the remover's
/// own `/` chain, records it at the space's access service, and retracts
/// the member's roster rows. The service refuses a revocation minted
/// under a member's `/use` chain, so holding the space is what lets this
/// take effect, not the role fact.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExpelMember {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The DID the member's membership is keyed on, from `data-expel`.
    pub member: crate::domain::command::expel::Expel,
}

/// Request to redeem an invite URL and join its space.
///
/// Asserted transiently when `<tonk-page>` fires its `mount` event on the
/// `/join` view (`<tonk-page onmount=tonk/join>`). The element reads the
/// complete page URL, including the fragment the service worker cannot see,
/// and delivers it as `detail.href`.
///
/// The handler parses and claims that URL, driving the overlay-only
/// `tonk:join/status` (pending → failed, or retract + durable space on
/// success).
#[derive(Concept, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Join {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// Complete invite URL from `detail.href`.
    pub url: crate::domain::command::join::Href,
}

/// Redacted like the current shape's: the url carries the membership
/// seed in its fragment, so it must never reach a log.
impl std::fmt::Debug for Join {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("legacy::Join")
            .field("this", &self.this)
            .field("url", &"[redacted]")
            .finish()
    }
}

impl RegisterAccount {
    /// The value [`Self::marker`] carries. The current shape needs no
    /// marker — `xyz.tonk.command.register-account/email` cannot be
    /// confused with the lookup's `…check-email/email` — so this is part
    /// of the compatibility surface, not of the command.
    pub const MARKER: &str = "tonk:register-account";
}
