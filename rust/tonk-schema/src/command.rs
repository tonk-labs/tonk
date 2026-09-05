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

pub mod legacy;

/// `tonk:delete-account`: purge the account from every service and
/// this device.
///
/// Dispatched from the hub's settings page after the person retyped
/// the account address. The worker checks the address, asks the page
/// for the account's passkey, signs `/void/customer/purge` with the
/// recovered root, and reports through [`crate::CeremonyStatus`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeleteAccount {
    /// The command entity (a fresh id per submission).
    pub this: Entity,
    /// The retyped account email.
    pub email: crate::domain::command::delete_account::Email,
}

impl Command for DeleteAccount {
    type Input = Self;
    type Output = ();
}

/// `tonk:authorize-device`: delegate the account to a waiting process.
///
/// The CLI opens `/settings/link?audience=&callback=&name=` and waits on
/// `callback`; the hub shows what is asking and, on approval, asserts
/// this. The worker asks the page for the passkey, mints the
/// `account -> device` grant with the recovered root, and sends the
/// page to the callback with the grant.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorizeDevice {
    /// The command entity (a fresh id per approval).
    pub this: Entity,
    /// The device DID to delegate to.
    pub audience: crate::domain::command::authorize_device::Audience,
    /// The waiting process's callback URL, base58.
    pub callback: crate::domain::command::authorize_device::Callback,
    /// The name the waiting process gave itself.
    pub name: crate::domain::command::authorize_device::Name,
}

impl Command for AuthorizeDevice {
    type Input = Self;
    type Output = ();
}

/// `tonk:add-passkey`: seal the account under a second passkey.
///
/// The worker asks the page for both ceremonies — the passkey that
/// holds the account, and the new one — and re-seals the secret.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddPasskey {
    /// The command entity (a fresh id per click).
    pub this: Entity,
    /// The account to add a passkey to. The verb carries no data of its
    /// own, so it needs one attribute simply to be nameable; see
    /// [`crate::domain::command::current::add_passkey::Account`].
    pub account: crate::domain::command::current::add_passkey::Account,
}

impl From<legacy::AddPasskey> for AddPasskey {
    fn from(legacy: legacy::AddPasskey) -> Self {
        Self {
            account: crate::domain::command::current::add_passkey::Account(legacy.marker.0),
            this: legacy.this,
        }
    }
}

impl Command for AddPasskey {
    type Input = Self;
    type Output = ();
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
    /// The address to look up.
    pub email: crate::domain::command::current::check_email::Email,
}

impl From<legacy::CheckEmail> for CheckEmail {
    fn from(legacy: legacy::CheckEmail) -> Self {
        Self {
            email: crate::domain::command::current::check_email::Email(legacy.email.0),
            this: legacy.this,
        }
    }
}

impl Command for CheckEmail {
    type Input = Self;
    type Output = ();
}

/// Create the account for an address, once the lookup said it is free.
///
/// The lookup ([`CheckEmail`]) and this were once the same shape
/// `{this, email}` under one shared DOM read path, so every keystroke's
/// lookup also decoded as a registration and the worker started a
/// passkey ceremony while the user was still typing. A marker attribute
/// patched that; the two now live in separate namespaces, so the shapes
/// cannot collide and the marker is gone.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegisterAccount {
    /// The command entity, minted per invocation.
    pub this: Entity,
    /// The address to register.
    pub email: crate::domain::command::current::register_account::Email,
}

impl From<legacy::RegisterAccount> for RegisterAccount {
    fn from(legacy: legacy::RegisterAccount) -> Self {
        Self {
            email: crate::domain::command::current::register_account::Email(legacy.email.0),
            this: legacy.this,
        }
    }
}

impl Command for RegisterAccount {
    type Input = Self;
    type Output = ();
}

/// Request to create a new space (repository) by local name.
///
/// Asserted transiently when a create form submits. The handler records
/// the replica (`status: blank`) so the Hub shows it installing, then
/// creates the repository, seeds the standard library, flips the status
/// to `initialized`, and navigates the creator into it.
///
/// Matched **name-only**, deliberately. An optional sync URL rides on
/// the same transient and is read straight from its facts rather than
/// declared here: a URL round-trips through JSON as `Value::Entity` (the
/// untagged decode picks it for any `:`-bearing string), so a
/// `String`-typed field would never decode one. That is a value
/// representation problem, not a command shape problem, and it is left
/// where it was.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateSpace {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The space's local name. Nothing asks for one: every create
    /// carries the `Untitled` sentinel, which the handler uniquifies
    /// against the existing labels ("Untitled", "Untitled 2", …), and
    /// the user renames the space in place on arrival.
    pub name: crate::domain::command::current::create_space::Name,
}

impl From<legacy::CreateSpace> for CreateSpace {
    fn from(legacy: legacy::CreateSpace) -> Self {
        Self {
            name: crate::domain::command::current::create_space::Name(legacy.name.0),
            this: legacy.this,
        }
    }
}

/// `CreateSpace` is a [`dialog_capability::Command`]. Note the worker
/// registers a custom `CreateSpaceHandler` (not a plain `Provider`) so it
/// can read the optional remote from the facts; the `Command` impl is
/// kept for the decode/`Decode` machinery.
impl Command for CreateSpace {
    type Input = Self;
    type Output = ();
}

/// Create a notebook from the index's heading switcher, and drop the
/// author into it.
///
/// The handler does both halves: it writes the notebook and then posts a
/// `navigate` to the originating client. The navigation cannot happen in
/// the page, because the notebook's entity is derived when the fact is
/// written — the element that fired the command never learns it.
///
/// The fields are `title` and `body`. They were `created-title` and
/// `created-body` only so a retitle's `detail/title` would not also
/// decode as a create; the namespace does that now.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateNotebook {
    /// The command entity, minted per invocation.
    pub this: Entity,
    /// The title typed into the heading.
    pub title: crate::domain::command::current::create_notebook::Title,
    /// The draft's document, blocks and all.
    pub body: crate::domain::command::current::create_notebook::Body,
}

impl From<legacy::CreateNotebook> for CreateNotebook {
    fn from(legacy: legacy::CreateNotebook) -> Self {
        Self {
            title: crate::domain::command::current::create_notebook::Title(legacy.title.0),
            body: crate::domain::command::current::create_notebook::Body(legacy.body.0),
            this: legacy.this,
        }
    }
}

/// `CreateNotebook` is a [`dialog_capability::Command`]; the worker
/// registers a custom handler for it (the work needs the branch handle
/// and the originating client, which the decoded command does not
/// carry).
impl Command for CreateNotebook {
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

/// Mint a repository invite: a membership keypair, a delegation of the
/// space's access to its DID, and the private seed on the overlay.
///
/// The target space rides on the transient as a raw fact
/// (`xyz.tonk.invite/space`) that the handler reads opportunistically,
/// falling back to the dispatch origin — the FAB's share affordance is
/// routeless, so it must name its target, while a space's own share form
/// need not.
///
/// The timestamp makes each activation a distinct transient, so repeated
/// Share clicks reliably re-fire the handler and rotate the credential.
/// It used to be accompanied by a marker attribute, whose whole job was
/// to stop a [`PauseSync`] transient — an identical `{this, time}`
/// otherwise — from also decoding as an invite. Per-verb namespaces make
/// the two disjoint, so the marker is gone.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct Invite {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The activation's timestamp — one click from the next.
    pub time: crate::domain::command::current::invite::Time,
}

impl From<legacy::Invite> for Invite {
    fn from(legacy: legacy::Invite) -> Self {
        Self {
            time: crate::domain::command::current::invite::Time(legacy.time.0),
            this: legacy.this,
        }
    }
}

/// `Invite` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (generates the keypair, delegates, asserts the
/// authorization + overlay credential).
impl Command for Invite {
    type Input = Self;
    type Output = ();
}

/// Attach a sync remote to an existing space, and optionally mint an
/// invite once it is attached.
///
/// Dispatched routelessly by the share control when a user accepts the
/// offer to turn sync on. `space`, `remote` and the `share` request ride
/// on the transient as raw facts the handler reads directly — `remote`
/// because a URL cannot be a `String`-typed field (see
/// [`crate::domain::command::enable_sync::Remote`]), the other two for
/// symmetry with it.
///
/// This used to need a marker attribute, and to be a second command
/// distinct from the `space/enable-sync` form's, because that form
/// shared [`CreateSpace`]'s trigger attribute and anything registered
/// against it would have minted a new space instead of attaching to the
/// existing one. Both commands now have their own namespace, so neither
/// workaround is load-bearing.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct EnableSync {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The acceptance's timestamp — one click from the next.
    pub time: crate::domain::command::current::enable_sync::Time,
}

impl From<legacy::EnableSync> for EnableSync {
    fn from(legacy: legacy::EnableSync) -> Self {
        Self {
            time: crate::domain::command::current::enable_sync::Time(legacy.time.0),
            this: legacy.this,
        }
    }
}

/// `EnableSync` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (attaches the remote, then mints when asked).
impl Command for EnableSync {
    type Input = Self;
    type Output = ();
}

/// Toggle background sync for a space's replica.
///
/// Dispatched when the FAB's sync cap is alt/option-clicked. Carries the
/// target `space` (the DID to pause) and a timestamp so each click is a
/// distinct transient; the handler reads the replica's current
/// `auto-sync` preference for that space and flips it.
///
/// Naming the target rather than firing on it is what lets this live on
/// and dispatch from the PROFILE branch, so the FAB's pause affordance
/// needs nothing seeded per space.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct PauseSync {
    /// The command entity (a fresh id per click).
    pub this: Entity,
    /// The click's timestamp — one click from the next.
    pub time: crate::domain::command::current::pause_sync::Time,
    /// The target space DID — the replica to pause/resume, read in place
    /// of the dispatch origin.
    pub space: crate::domain::command::pause_sync::Space,
}

impl From<legacy::PauseSync> for PauseSync {
    fn from(legacy: legacy::PauseSync) -> Self {
        Self {
            time: crate::domain::command::current::pause_sync::Time(legacy.time.0),
            space: legacy.space,
            this: legacy.this,
        }
    }
}

/// `PauseSync` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (flips the replica's durable `auto-sync` preference).
impl Command for PauseSync {
    type Input = Self;
    type Output = ();
}

/// Rename a space's repository from the FAB.
///
/// The space-side `tonk/rename-repository` rule (`core.yaml`) cannot
/// consume a claim dispatched on the profile branch, so this carries its
/// target `space` and is executed by a worker handler instead — the
/// [`PauseSync`] pattern. That is what lets the FAB's name chip depend
/// on nothing seeded per space.
///
/// It shared `currentTarget.value` with [`ProfileRename`], and a marker
/// attribute is what kept the two shapes disjoint. They now have
/// separate namespaces, so neither carries one.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenameRepository {
    /// The command entity (a fresh id per commit).
    pub this: Entity,
    /// The new repository name.
    pub name: crate::domain::command::current::rename_repository::Name,
    /// The target space DID — the repository to rename.
    pub space: crate::domain::command::rename_repository::Space,
}

impl From<legacy::RenameRepository> for RenameRepository {
    fn from(legacy: legacy::RenameRepository) -> Self {
        Self {
            name: crate::domain::command::current::rename_repository::Name(legacy.name.0),
            space: legacy.space,
            this: legacy.this,
        }
    }
}

impl Command for RenameRepository {
    type Input = Self;
    type Output = ();
}

/// Rename the signed-in member (set their display name).
///
/// Asserted transiently when the topbar identity chip's
/// `<tonk-editable>` commits. The handler persists the override to the
/// profile meta branch and re-stamps `MemberName` on the origin space.
///
/// See [`RenameRepository`] for the marker these two used to need.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileRename {
    /// The command entity (a fresh id per commit).
    pub this: Entity,
    /// The new display name.
    pub name: crate::domain::command::current::profile_rename::Name,
}

impl From<legacy::ProfileRename> for ProfileRename {
    fn from(legacy: legacy::ProfileRename) -> Self {
        Self {
            name: crate::domain::command::current::profile_rename::Name(legacy.name.0),
            this: legacy.this,
        }
    }
}

impl Command for ProfileRename {
    type Input = Self;
    type Output = ();
}

/// Request to remove a space from this device: retract its replica
/// record from the profile meta branch (the Hub row's source of truth),
/// detach it from the reactor/sync, and delete its local storage.
///
/// Removal is device-local: a synced space can be rejoined via an invite
/// link, and server-side data is untouched. An owned hosted space does
/// NOT submit this — `<ui-space-remove>` routes that verb through the
/// reviewed account-space deletion flow instead.
///
/// The field is called `subject`, which is what it is. It used to be
/// read from `data-remove` rather than the honest `data-subject` for one
/// reason: a `dataset/subject` field would also have matched every
/// [`RenameRepository`] transient and turned each rename into a
/// deletion.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoveSpace {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The subject DID of the space to remove.
    pub subject: crate::domain::command::current::remove_space::Subject,
}

impl From<legacy::RemoveSpace> for RemoveSpace {
    fn from(legacy: legacy::RemoveSpace) -> Self {
        Self {
            subject: crate::domain::command::current::remove_space::Subject(legacy.subject.0),
            this: legacy.this,
        }
    }
}

/// `RemoveSpace` is a [`dialog_capability::Command`]; the worker
/// registers a custom `RemoveSpaceHandler` (the work needs the profile
/// handle, the reactor cache, and storage — state the decoded command
/// doesn't carry).
impl Command for RemoveSpace {
    type Input = Self;
    type Output = ();
}

/// Promote a member of a space to admin.
///
/// Dispatched by the FAB's roster after the page minted the admin hop
/// under the passkey: the guest asks the outer page to delegate `/` on
/// the space to the member's account, and the page answers with the hop
/// signed by the promoter's account root. The worker's handler proves the
/// promoter's own `/` chain from the space db, appends the hop, checks it
/// is the one asked for, retains the chain beside the invites, and stamps
/// `MemberRole::admin`. No device key sits in the admin chain, so signing
/// a device out never takes the admins it promoted with it.
///
/// Routeless like `tonk:pause-sync`: the command names its `space` rather
/// than firing on it, so the FAB needs nothing seeded per space.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PromoteMember {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The DID the member's membership is keyed on.
    pub member: crate::domain::command::promote::Member,
    /// The space the promotion is in.
    pub space: crate::domain::command::promote::Space,
    /// The page-minted `promoter-account -> member` hop, base58.
    pub chain: crate::domain::command::promote::Chain,
}

impl Command for PromoteMember {
    type Input = Self;
    type Output = ();
}

/// Register this profile's account as a customer of the access service.
///
/// A command rather than a request: the outcome is the `AccountCustomer`
/// fact, which every device on the account reads and every tab showing
/// registration state already subscribes to. A caller that used to await
/// a receipt watches that fact instead, which is also what lets a
/// confirmation performed elsewhere reach a waiting screen.
///
/// Idempotent by the service's own rule: enrolling an account that is
/// still awaiting activation resends its link, and one already active is
/// answered as active rather than refused.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EnrollCustomer {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The address to enroll. Empty means the account's recorded one,
    /// which is what the login and resend paths want.
    pub email: crate::domain::command::enroll::Email,
}

impl Command for EnrollCustomer {
    type Input = Self;
    type Output = ();
}

/// Ask the access service to mail this account's activation link again.
///
/// No address and no ceremony: the enrollment's rows stand at the
/// service, so the only thing left is the mail, and the worker signs the
/// self-subjected `/customer/resend` invocation with its own device key.
/// Deliberately NOT a re-enrollment — that path runs a passkey ceremony
/// the person waiting on an email never asked for.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResendActivation {
    /// The command entity (a fresh id per press).
    pub this: Entity,
    /// When the resend was pressed, so a second press re-fires.
    pub at: crate::domain::command::resend::At,
}

impl Command for ResendActivation {
    type Input = Self;
    type Output = ();
}

/// Revoke a member's access to a space.
///
/// Asserted transiently by the roster row's expel control; the handler
/// revokes the hop that admits the member under the remover's own `/`
/// chain, records it at the space's access service, and retracts the
/// member's roster rows. The service refuses a revocation minted under a
/// member's `/use` chain, so holding the space is what lets this take
/// effect, not the role fact.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExpelMember {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The DID the member's membership is keyed on.
    pub member: crate::domain::command::current::expel_member::Member,
}

impl From<legacy::ExpelMember> for ExpelMember {
    fn from(legacy: legacy::ExpelMember) -> Self {
        Self {
            member: crate::domain::command::current::expel_member::Member(legacy.member.0),
            this: legacy.this,
        }
    }
}

impl Command for ExpelMember {
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
    /// The sync remote endpoint (`&remote=`). Never empty — `run_invite` is
    /// the only handler that asserts an `Authorization`, and it refuses to
    /// mint one at all for a repository with no shareable remote.
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

/// Where a space's invite has got to — the single row the share control
/// renders, keyed by the space's **subject** DID.
///
/// Three nouns are close here and must stay distinct:
/// [`Invite`] is the COMMAND (the intent to share), [`crate::Invitation`]
/// is the durable record of a MINTED invite (keyed on the delegation
/// CID, used for revocation), and this is the per-space STATE the
/// control renders.
///
/// The share control subscribes to this and nothing else. It reads
/// `status`: `granted` copies the `url`, `requested` keeps waiting, and
/// **anything else** shows failed. That default is what lets a new
/// terminal status ship without touching the control, and it is why the
/// control never enumerates the terminal set.
///
/// Overlay-only. The url carries the membership seed in its fragment, so
/// it must not reach storage, and the row is a per-session view of an
/// in-flight request rather than a durable record. The durable record of
/// a minted invite is [`crate::Invitation`], keyed on the delegation CID.
///
/// `url` is optional, so one row covers every state with no sentinel
/// value: a request in flight simply has no url yet.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InviteState {
    /// The space's subject DID.
    pub this: Entity,
    /// One of the `invite:*` markers; see [`InviteState::REQUESTED`].
    pub status: crate::domain::invite::Status,
    /// The finished invite URL, once there is one.
    pub url: Option<crate::domain::invite::Url>,
}

impl InviteState {
    /// Asked for, and in progress. The control keeps waiting.
    pub const REQUESTED: &str = "invite:requested";
    /// Minted; `url` is present.
    pub const GRANTED: &str = "invite:granted";
    /// The account's service was withdrawn. Terminal.
    pub const SUSPENDED: &str = "invite:suspended";
    /// The upstream is not a UCAN endpoint, so no invite URL can
    /// express it. Terminal.
    pub const UNSHAREABLE: &str = "invite:unshareable";

    /// A request in flight, with no url yet.
    pub fn requested(space: Entity) -> Self {
        Self::marker(space, Self::REQUESTED)
    }

    /// A granted invite carrying its url.
    pub fn granted(space: Entity, url: String) -> Self {
        Self {
            this: space,
            status: crate::domain::invite::Status(
                Self::GRANTED.parse().expect("invite:granted parses"),
            ),
            url: Some(crate::domain::invite::Url(url)),
        }
    }

    /// A terminal state naming why no invite is possible.
    pub fn denied(space: Entity, status: &str) -> Self {
        Self::marker(space, status)
    }

    fn marker(space: Entity, status: &str) -> Self {
        Self {
            this: space,
            status: crate::domain::invite::Status(
                status.parse().expect("an invite:* marker parses"),
            ),
            url: None,
        }
    }
}

/// The overlay-only fact a refused `tonk:invite` asserts: why the mint did
/// not happen, keyed by the space's **subject** DID (`this`) — the same
/// entity [`Credential`] is keyed by, so one subject carries both the
/// success and the refusal.
///
/// All three fields are asserted together. A concept resolves only when
/// every declared field is present, so a partial assert would never resolve
/// (the same all-fields-required gotcha `JoinStatus`/`JoinFailure` are split
/// to avoid).
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct ShareBlocked {
    /// The space's subject DID.
    pub this: Entity,
    /// Refusal class: `not-synced` | `unshareable-remote` | `attach-failed`.
    pub blocked: crate::domain::share::Blocked,
    /// The sentence shown to the user.
    pub detail: crate::domain::share::Detail,
    /// The refused command's timestamp, echoed so the share control can tell
    /// this refusal from a replay of an older one.
    pub time: crate::domain::share::Time,
}

/// Redeem an invite URL and join its space.
///
/// Fired by the /join view's page element, which reads the page location
/// (the service worker cannot see the `#fragment`, where an open
/// invite's seed rides). The handler claims the invite and drives
/// `tonk:join/status` (pending → failed, or retract + durable replica on
/// success).
#[derive(Concept, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Join {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// The complete invite URL, fragment included.
    pub url: crate::domain::command::current::join::Url,
}

impl From<legacy::Join> for Join {
    fn from(legacy: legacy::Join) -> Self {
        Self {
            url: crate::domain::command::current::join::Url(legacy.url.0),
            this: legacy.this,
        }
    }
}

impl std::fmt::Debug for Join {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Join")
            .field("this", &self.this)
            .field("url", &"[redacted]")
            .finish()
    }
}

/// `Join` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (claims the carried invite URL and drives `JoinStatus`).
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

#[cfg(test)]
mod share_blocked {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_share_attribute_names() {
        assert_eq!(
            crate::domain::share::Blocked::the().to_string(),
            "xyz.tonk.share/blocked"
        );
        assert_eq!(
            crate::domain::share::Detail::the().to_string(),
            "xyz.tonk.share/detail"
        );
        assert_eq!(
            crate::domain::share::Time::the().to_string(),
            "xyz.tonk.share/time"
        );
    }
}

#[cfg(test)]
mod enable_sync {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_carries_a_marker_no_other_command_carries() {
        use crate::domain::command::enable_sync;

        assert_eq!(
            enable_sync::EnableSync::the().to_string(),
            "dom.event.current-target.dataset/enable-sync"
        );
        assert_eq!(
            enable_sync::Space::the().to_string(),
            "xyz.tonk.enable-sync/space"
        );
        assert_eq!(
            enable_sync::Remote::the().to_string(),
            "xyz.tonk.enable-sync/remote"
        );
        assert_eq!(
            enable_sync::Share::the().to_string(),
            "xyz.tonk.enable-sync/share"
        );
        // Shared verbatim with every other command's timestamp, so it must
        // stay on the `dom.event` domain rather than this command's own.
        assert_eq!(
            enable_sync::TimeStamp::the().to_string(),
            "dom.event/time-stamp"
        );
    }
}
