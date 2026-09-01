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

impl Command for CheckEmail {
    type Input = Self;
    type Output = ();
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

impl RegisterAccount {
    /// The value [`Self::marker`] carries.
    pub const MARKER: &str = "tonk:register-account";
}

impl Command for RegisterAccount {
    type Input = Self;
    type Output = ();
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

/// `Invite` is a [`dialog_capability::Command`]; its handler lives in
/// `tonk-worker` (generates the keypair, delegates, asserts the
/// authorization + overlay credential).
impl Command for Invite {
    type Input = Self;
    type Output = ();
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
    /// shares the `{this, value}` shape. A DISTINCT ATTRIBUTE (not a
    /// distinct marker value) is what keeps the shapes disjoint — see
    /// `domain::command::rename_repository::RenameRepository`'s doc.
    pub marker: crate::domain::command::rename_repository::RenameRepository,
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
