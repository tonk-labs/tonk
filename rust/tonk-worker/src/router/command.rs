//! Command registration and the providers' run after a commit.
//!
//! [`command_providers`] builds the worker's command vocabulary, carried
//! on [`TonkState`]: one [`Registration`] per command, each naming the
//! surface it runs on ([`Surface`]). The vocabulary is registered on a
//! branch's dialog stack as that stack's providers
//! ([`CommandProviders::provide`], once per branch, through
//! [`BranchState::provide_once`]), and [`run_commands`] performs the
//! effect a dispatch landed: dialog runs every provider whose concept
//! each command's rows satisfy and writes its lifecycle rows, and the
//! worker drains the polls the request scheduled after.
//!
//! A command is a claim of a transient concept. A transaction through
//! the reactor writes it `.at(control_layer())`, so dialog takes it as
//! the dispatch's stimulus (the branch's rules read it for one tick) and,
//! once the dispatch has published, lands it in the stack's `control:`
//! layer for one instant, which the dispatch's report names
//! ([`Landed::effect`](crate::reactor::Landed::effect)). Nothing of a
//! command is stored or replicated.
//!
//! A command is a [`dialog_capability::Command`] run by a
//! [`Provider<C>`](dialog_capability::Provider) — there is no handler
//! function. The provider is [`CommandEnv`], a cheap handle over
//! [`AppState`]; registering a command requires `CommandEnv: Provider<C>`,
//! so capability is a compile-time gate. A command is self-contained: its
//! `execute` does its own IO and commits its own outcomes through the env,
//! and answers dialog with no rows of its own.
//!
//! The stack provider for a command is built per branch
//! ([`Scope`]): one provider per registration, answering every concept
//! the command decodes from (a migrated command's current and legacy
//! shapes, `Provider::also`). Its handler captures the state and the
//! branch the stack is of, reads the tick the command landed in from
//! the command itself ([`Tick`]: the asking client, `control:
//! tonk.session/client client:<id>`, which the committing route stamps
//! beside the commands and dialog hands each command as
//! `Command::tick`), decodes the command exactly as before ([`Decode`],
//! [`Migrated`]) and runs `Provider::execute`. A command sent for a
//! branch that has no stack yet (an absent repository, a branch nobody
//! has made) goes through the profile's stack with its origin in the
//! tick's rows ([`relay_commands`]).
//!
//! [`TonkState`]: crate::worker::TonkState
//! [`BranchState::provide_once`]: crate::reactor::BranchState::provide_once

use std::sync::Arc;

use dialog_capability::{Command, Provider};
use dialog_common::{ConditionalSend, ConditionalSync};
use dialog_corpus::{Changes, Entity, Fact, Value};
use dialog_query::ConceptDescriptor;
use dialog_repository::{
    CommitError, Effect, EffectReport, Failure, Provider as CommandProvider, Stack,
};
use tonk_common::log;

use super::AppState;
use crate::reactor::{Decode, Migrated, control, stamp};

/// The environment commands run against — a cheap handle (clone of
/// [`AppState`]) that implements
/// [`Provider<C>`](dialog_capability::Provider) for each command `C` the
/// worker supports. The stack provider hands a clone to the decoded
/// command; `execute` does the work, reaching the operator/reactor by
/// re-locking through the `AppState`.
///
/// Capability is structural: a command runs iff `CommandEnv: Provider<C>`
/// is implemented. Registering a command requires that bound, so an
/// unsupported command won't even register. (The runtime UCAN-style gate
/// — the operator actually *holding* the capability — layers on top of
/// this later.)
#[derive(Clone)]
pub struct CommandEnv {
    state: AppState,
    origin: CommandOrigin,
}

/// The repository + branch a command was triggered in, and the client
/// that asked. The repository and branch are the stack's the command
/// landed on, unless the tick it landed in says otherwise
/// ([`control::repo`], [`control::branch`]: a command relayed through
/// the profile's stack for a branch that has no stack yet); the client
/// is the tick's [`control::client`] row, which the committing route
/// stamps beside the commands.
///
/// [`control::repo`]: crate::reactor::control::repo
/// [`control::branch`]: crate::reactor::control::branch
/// [`control::client`]: crate::reactor::control::client
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOrigin {
    /// The repository name (its routing key).
    pub repo: String,
    /// The branch name.
    pub branch: String,
    /// The service-worker client the triggering request originated from,
    /// when known. A handler whose effect is a page capability (e.g.
    /// navigation) posts a message back to this exact client — the service
    /// worker has no `window`, and a transient command never lands in a
    /// branch a subscription could observe, so the originating client is
    /// the only channel back to the page that asked for the effect.
    pub client: Option<crate::router::ClientId>,
}

impl CommandOrigin {
    /// The origin of a command that landed on `stack`'s branch in
    /// `tick`: the branch the tick names, else the stack's own, and the
    /// tick's client.
    fn of_tick(stack: &Scope, tick: Option<Tick>) -> Self {
        let tick = tick.unwrap_or_default();
        let (repo, branch) = match (tick.repo, tick.branch) {
            (Some(repo), Some(branch)) => (repo, branch),
            _ => (stack.repo.clone(), stack.branch.clone()),
        };
        Self {
            repo,
            branch,
            client: tick.client,
        }
    }
}

/// What the `control:` entity's rows say of one tick: who asked for its
/// commands, and which branch they were sent for when it is not the
/// stack's own. Read from the rows dialog hands each command of the
/// tick ([`Tick::of_rows`] over `Command::tick`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// The asking client, from `control: tonk.session/client client:<id>`.
    pub client: Option<crate::router::ClientId>,
    /// The repository the commands were sent for, from
    /// `control: tonk.session/repo`.
    pub repo: Option<String>,
    /// The branch the commands were sent for, from
    /// `control: tonk.session/branch`.
    pub branch: Option<String>,
}

impl Tick {
    /// The tick whose `control:` entity's rows are `rows` (a command's
    /// `tick()`); `None` when the tick carries none.
    pub fn of_rows(rows: &[Fact]) -> Option<Self> {
        if rows.is_empty() {
            return None;
        }
        let text = |the: dialog_corpus::Relation| {
            rows.iter().find_map(|row| match &row.is {
                Value::String(text) if row.the == the => Some(text.clone()),
                _ => None,
            })
        };
        Some(Self {
            client: rows.iter().find_map(|row| match &row.is {
                Value::Entity(entity) if row.the == control::client() => {
                    control::client_id(entity).map(crate::router::ClientId)
                }
                _ => None,
            }),
            repo: text(control::repo()),
            branch: text(control::branch()),
        })
    }

    /// Write this tick's rows on the `control:` entity into `changes`'
    /// `control:` share, beside the commands written there.
    fn stamp(&self, changes: &mut Changes) {
        if let Some(client) = &self.client {
            client_row(changes, client);
        }
        if let Some(repo) = &self.repo {
            stamp(changes, control::repo(), Value::String(repo.clone()));
        }
        if let Some(branch) = &self.branch {
            stamp(changes, control::branch(), Value::String(branch.clone()));
        }
    }
}

/// Write the asking `client` as the tick's `control: tonk.session/client
/// client:<id>` row into `changes`' `control:` share, beside the
/// commands written there. A client id that makes no URI is not
/// written.
pub(crate) fn client_row(changes: &mut Changes, client: &crate::router::ClientId) {
    match control::client_entity(&client.0) {
        Some(entity) => stamp(changes, control::client(), Value::Entity(entity)),
        None => log!(
            "commands: client id {:?} makes no entity; not stamped",
            client.0
        ),
    }
}

impl CommandEnv {
    /// Build the env over a clone of the shared state, scoped to the
    /// `origin` the triggering commit happened in.
    pub fn new(state: AppState, origin: CommandOrigin) -> Self {
        Self { state, origin }
    }

    /// Borrow the underlying state — `Provider` impls re-lock through
    /// this to reach the operator and reactor.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// The repository + branch this command was triggered in. A handler
    /// that operates on "the repo I fired in" (e.g. minting an invite
    /// for it) reads the origin repo name here and loads the repository
    /// through `state()`, rather than receiving the subject as a command
    /// field.
    pub fn origin(&self) -> &CommandOrigin {
        &self.origin
    }

    /// The service-worker client the triggering request came from, when
    /// known. A handler posts a page-capability effect (e.g. navigation)
    /// back to this client.
    pub fn client(&self) -> Option<&crate::router::ClientId> {
        self.origin.client.as_ref()
    }

    /// Whether the triggering commit landed on the profile branch — the
    /// Hub/FAB evaluation surface. `transact_profile` never names a
    /// repo, so an empty origin repo IS the profile; a non-empty one is
    /// a content branch, whose facts were matched by shape, not by who
    /// asked.
    pub fn from_profile(&self) -> bool {
        self.origin.repo.is_empty()
    }

    /// Whether a command fired here may act on the space at
    /// `target_key`.
    ///
    /// The rule that scopes every space-targeting command: a space may
    /// act on ITSELF (origin == target), and the PROFILE branch may act
    /// on any space by DID — it is the surface the Hub and FAB dispatch
    /// from. A content branch naming a DIFFERENT space is refused: a
    /// same-shaped fact committed on any joined space's branch (its own
    /// notation, or a same-origin POST to its `/transact`) must not
    /// reach into other spaces. This is a dispatch-level containment
    /// boundary, like Level-0 path routing — it has to live here
    /// because the operator itself holds time-bounded
    /// `Subject::any()` authority (see `session.rs`) and so cannot
    /// distinguish targets.
    pub fn may_target_space(&self, target_key: &str) -> bool {
        self.from_profile() || self.origin.repo == target_key
    }
}

/// Where a command runs: the profile branch — the Hub/FAB evaluation
/// surface, the one the user actually drives — a space (content)
/// branch, or either.
///
/// This is the vocabulary-level expression of the containment rule
/// [`CommandEnv::may_target_space`] enforces per provider: WHERE a
/// transient committed decides WHAT it can mean. The profile carries
/// the full vocabulary. A space branch carries only the commands that
/// are genuinely that space's to run; everything else (space lifecycle,
/// joins, account/passkey ceremonies, member promotion) simply does not
/// exist there, so a same-shaped fact committed on a joined space's
/// branch is refused instead of relying on each provider to notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The profile branch alone.
    Profile,
    /// A space branch alone: what the profile has none of.
    Space,
    /// Either: a space acting on itself, dispatchable from the profile
    /// too — the FAB is routeless.
    Both,
}

impl Surface {
    /// Whether a command of this surface may run from `origin`.
    pub fn admits(self, origin: &CommandOrigin) -> bool {
        match self {
            Surface::Profile => origin.repo.is_empty(),
            Surface::Space => !origin.repo.is_empty(),
            Surface::Both => true,
        }
    }
}

/// What a stack's providers are built for: the state they re-lock
/// through, and the branch the stack is of — the origin of every
/// command that lands on it, unless the command's rows say otherwise.
#[derive(Clone)]
pub struct Scope {
    /// The shared state the providers' commands run against.
    pub state: AppState,
    /// The repository the stack's branch is of; empty for the profile.
    pub repo: String,
    /// The stack's branch.
    pub branch: String,
}

/// A command's decode, from the rows of its entity.
#[cfg(not(target_arch = "wasm32"))]
type Decoder<C> = Arc<dyn Fn(&[Fact]) -> Option<C> + Send + Sync>;
/// A command's decode, from the rows of its entity.
#[cfg(target_arch = "wasm32")]
type Decoder<C> = Arc<dyn Fn(&[Fact]) -> Option<C>>;

/// What a provider answers dialog with, boxed so every registration's
/// run has one type.
#[cfg(not(target_arch = "wasm32"))]
type Answer = futures_util::future::BoxFuture<'static, Result<Changes, Failure>>;
/// What a provider answers dialog with, boxed so every registration's
/// run has one type.
#[cfg(target_arch = "wasm32")]
type Answer = futures_util::future::LocalBoxFuture<'static, Result<Changes, Failure>>;

/// A decoded command, ready to run against the [`CommandEnv`] of its
/// tick.
#[cfg(not(target_arch = "wasm32"))]
type Run = Box<dyn FnOnce(CommandEnv) -> Answer + Send>;
/// A decoded command, ready to run against the [`CommandEnv`] of its
/// tick.
#[cfg(target_arch = "wasm32")]
type Run = Box<dyn FnOnce(CommandEnv) -> Answer>;

/// Decodes a landed command as one registration's command, or answers
/// `None` when its rows are not that command.
#[cfg(not(target_arch = "wasm32"))]
type Decoding = Arc<dyn Fn(&dialog_repository::Command) -> Option<Run> + Send + Sync>;
/// Decodes a landed command as one registration's command, or answers
/// `None` when its rows are not that command.
#[cfg(target_arch = "wasm32")]
type Decoding = Arc<dyn Fn(&dialog_repository::Command) -> Option<Run>>;

/// One command the worker provides: the concepts its rows decode from,
/// the surface it runs on, and how a landed command decodes as it. Its
/// stack provider claims a landed command whose rows satisfy one of
/// its concepts, every required field having a row
/// ([`dialog_repository::Provider::claims`]): one provider per
/// registration, answering each of its concepts (`Provider::also`), so
/// a migrated command's current and legacy shapes are one provider,
/// both decoding to the current shape.
pub struct Registration {
    surface: Surface,
    concepts: Vec<&'static ConceptDescriptor>,
    decode: Decoding,
}

impl Registration {
    /// Where the command runs.
    pub fn surface(&self) -> Surface {
        self.surface
    }

    /// The concepts whose commands this registration's providers claim.
    pub fn concepts(&self) -> &[&'static ConceptDescriptor] {
        &self.concepts
    }

    /// Whether this registration's stack provider claims the command
    /// whose rows are `rows`: whether they satisfy one of its concepts,
    /// every required field of it having a row of its attribute and of
    /// its value type. The rule is dialog's
    /// ([`dialog_repository::Provider::claims`]), asked of a provider
    /// over the same concepts.
    pub fn claims(&self, rows: &[Fact]) -> bool {
        provider_over(&self.concepts, |_| async {
            Ok::<Changes, Failure>(Changes::new())
        })
        .is_some_and(|provider| provider.claims(rows))
    }

    /// Whether the command may run from `origin`.
    pub fn admits(&self, origin: &CommandOrigin) -> bool {
        self.surface.admits(origin)
    }
}

/// A stack provider answering the commands of every one of `concepts`
/// with `handler`: dialog's one provider over the first, also answering
/// the rest ([`dialog_repository::Provider::also`]). `None` for no
/// concept.
fn provider_over<F, Fut>(
    concepts: &[&'static ConceptDescriptor],
    handler: F,
) -> Option<CommandProvider>
where
    F: Fn(dialog_repository::Command) -> Fut + ConditionalSend + ConditionalSync + 'static,
    Fut: std::future::Future<Output = Result<Changes, Failure>> + ConditionalSend + 'static,
{
    let (first, rest) = concepts.split_first()?;
    Some(
        rest.iter()
            .fold(CommandProvider::new(first, handler), |provider, concept| {
                provider.also(concept)
            }),
    )
}

/// The refusal of a command `name` from `env`'s origin, whose surface
/// it is not of.
fn refusal(name: &Entity, env: &CommandEnv) -> Failure {
    Failure::new(format!(
        "{name} is no command of {}",
        if env.from_profile() {
            "the profile".to_owned()
        } else {
            format!("space '{}'", env.origin().repo)
        }
    ))
}

/// Run `command`, which the provider of the registration named `name`
/// claimed: from the origin its tick names, refused when the surface
/// does not admit it from there, else decoded as the registration's
/// command through `decode` and run against the env of that origin.
///
/// Dialog runs every provider whose concept the command's rows satisfy,
/// in registration order, so a command whose rows satisfy two
/// registrations' concepts runs both, as the worker's registry ran
/// every command that decoded before the commands were a stack's
/// providers.
async fn dispatch(
    scope: Scope,
    surface: Surface,
    name: Entity,
    decode: Decoding,
    command: dialog_repository::Command,
) -> Result<Changes, Failure> {
    let origin = CommandOrigin::of_tick(&scope, Tick::of_rows(command.tick()));
    let env = CommandEnv::new(scope.state.clone(), origin);
    if !surface.admits(env.origin()) {
        return Err(refusal(&name, &env));
    }
    let Some(run) = decode(&command) else {
        return Err(Failure::new(format!("the rows do not decode as {name}")));
    };
    run(env).await
}

/// The worker's command vocabulary: every command it provides, each with
/// the surface it runs on. Registered on each branch's stack as its
/// providers ([`provide`](Self::provide)).
#[derive(Default)]
pub struct CommandProviders {
    registrations: Arc<Vec<Registration>>,
}

impl CommandProviders {
    /// An empty vocabulary. Production uses [`command_providers`]; tests
    /// use this to install doubles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the command type `C`, run on `surface`. Lighter than
    /// passing a handler: the behaviour is the
    /// [`Provider<C>`](dialog_capability::Provider) impl on
    /// [`CommandEnv`], so registration is just the type. The
    /// `CommandEnv: Provider<C>` bound means a command can only be
    /// registered if the env has the capability to run it. Chainable.
    pub fn command<C>(self, surface: Surface) -> Self
    where
        C: Decode + Command<Input = C, Output = ()> + ConditionalSend + ConditionalSync + 'static,
        CommandEnv: Provider<C>,
    {
        let decode: Decoder<C> = Arc::new(|rows| {
            crate::reactor::facts_entity(rows).and_then(|this| C::decode(this, rows))
        });
        self.register(surface, C::concepts(), decode)
    }

    /// Register the command type `Current`, also accepting the
    /// deprecated `Legacy` shape on its behalf and converting it. The
    /// [`Self::command`] counterpart for a command whose shape changed;
    /// see [`Migrated`]. Chainable.
    pub fn migrated<Current, Legacy>(self, surface: Surface) -> Self
    where
        Current: Decode
            + Command<Input = Current, Output = ()>
            + ConditionalSend
            + ConditionalSync
            + 'static,
        Legacy: Decode + Into<Current> + ConditionalSend + ConditionalSync + 'static,
        CommandEnv: Provider<Current>,
    {
        let migrated: Migrated<Current, Legacy> = Migrated::new();
        let decode: Decoder<Current> = Arc::new(move |rows| migrated.decode(rows));
        self.register(surface, Migrated::<Current, Legacy>::concepts(), decode)
    }

    /// Register a command answered by `answer` itself: a handler over
    /// the [`CommandEnv`] of the command's tick and the landed command,
    /// whose result is the rows dialog writes as the answer, each in the
    /// layer it names. The typed commands answer with nothing and commit
    /// through the env; this is for a command whose answer is the point
    /// (`intent/suggest`, answered in the stack's `session:` layer).
    /// Every command its concept claims is its. Chainable.
    pub fn answering<F, Fut>(
        self,
        surface: Surface,
        concept: &'static ConceptDescriptor,
        answer: F,
    ) -> Self
    where
        F: Fn(CommandEnv, dialog_repository::Command) -> Fut
            + ConditionalSend
            + ConditionalSync
            + 'static,
        Fut: std::future::Future<Output = Result<Changes, Failure>> + ConditionalSend + 'static,
    {
        let answer = Arc::new(answer);
        let decode: Decoding = Arc::new(move |command: &dialog_repository::Command| {
            let answer = answer.clone();
            let command = command.clone();
            let run: Run = Box::new(move |env: CommandEnv| -> Answer {
                Box::pin(async move { answer(env, command).await })
            });
            Some(run)
        });
        self.push(Registration {
            surface,
            concepts: vec![concept],
            decode,
        })
    }

    /// Register a command decoding through `decode`, claimed by
    /// `concepts`, run by the [`Provider<C>`](dialog_capability::Provider)
    /// impl on [`CommandEnv`].
    fn register<C>(
        self,
        surface: Surface,
        concepts: Vec<&'static ConceptDescriptor>,
        decode: Decoder<C>,
    ) -> Self
    where
        C: Command<Input = C, Output = ()> + ConditionalSend + ConditionalSync + 'static,
        CommandEnv: Provider<C>,
    {
        let decode: Decoding = Arc::new(move |command: &dialog_repository::Command| {
            // Decode synchronously, then hand the owned command to the
            // run: the provider's `execute` re-locks through the env, so
            // nothing here holds a lock.
            let decoded = decode(command.rows())?;
            let run: Run = Box::new(move |env: CommandEnv| -> Answer {
                Box::pin(async move {
                    env.execute(decoded).await;
                    Ok(Changes::new())
                })
            });
            Some(run)
        });
        self.push(Registration {
            surface,
            concepts,
            decode,
        })
    }

    /// Add `registration`, after every one registered so far.
    fn push(mut self, registration: Registration) -> Self {
        Arc::get_mut(&mut self.registrations)
            .expect("the vocabulary is built before it is provided")
            .push(registration);
        self
    }

    /// Every command registered, in registration order — the order
    /// dialog tries their providers in.
    pub fn registrations(&self) -> &[Registration] {
        &self.registrations
    }

    /// `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    /// Register every command's providers on `stack`, built for the
    /// branch at `repo` / `branch` over `state`: one stack provider per
    /// registration, answering each of its concepts and running the
    /// command as [`dispatch`] does. Done once per branch, through
    /// [`BranchState::provide_once`](crate::reactor::BranchState::provide_once).
    pub fn provide(
        &self,
        stack: &Stack,
        state: AppState,
        repo: String,
        branch: String,
    ) -> Result<(), CommitError> {
        let scope = Scope {
            state,
            repo,
            branch,
        };
        for registration in self.registrations.iter() {
            let Some(first) = registration.concepts.first() else {
                continue;
            };
            let name = first.this();
            let surface = registration.surface;
            let decode = registration.decode.clone();
            let scope = scope.clone();
            if let Some(provider) = provider_over(
                &registration.concepts,
                move |command: dialog_repository::Command| {
                    dispatch(
                        scope.clone(),
                        surface,
                        name.clone(),
                        decode.clone(),
                        command,
                    )
                },
            ) {
                stack.provide(provider)?;
            }
        }
        Ok(())
    }
}

/// Build the command vocabulary. Registration is just the type and the
/// surface — the behaviour is the `Provider<C>` impl on [`CommandEnv`].
///
/// The same vocabulary builds for EVERY target — the browser, the CLI,
/// a TUI, a test. A command whose effect needs a page (a passkey
/// ceremony, a redirect) still registers everywhere; its provider
/// refuses visibly on a host without one rather than silently not
/// existing there. Where a chain genuinely needs a browser API, the
/// `cfg` sits on that leaf (`delete_space_storage_for`,
/// `worker_origin`, the `navigate` client messaging), never on a
/// registration.
///
/// Three commands register through wrapper request types rather than
/// their schema concept, because their transients carry facts outside
/// the matched shape (a frozen descriptor can't grow a field, and a
/// URL/DID deserializes as `Value::Entity`, which a `String` field
/// can't decode): [`CreateSpaceRequest`] reads the optional `remote`,
/// [`InviteRequest`] the optional target `space`, and
/// [`EnableSyncRequest`] its `space`/`remote`/`share` trio. Each
/// hand-implements [`Decode`](crate::reactor::Decode) to combine the
/// migrated concept decode with those raw-fact reads.
///
/// [`EnableSyncRequest`] is deliberately its own command
/// ([`tonk_schema::command::EnableSync`]), not a second registration on
/// `space/enable-sync`: that trigger attribute belongs to `CreateSpace`,
/// whose provider always mints a fresh identity first, so anything
/// registered against it would attach the remote to a brand-new space
/// rather than the existing one the FAB names.
///
/// A space branch runs only what is genuinely its own: loading a site
/// ([`Load`](tonk_schema::command::Load), whose target IS the origin),
/// renaming itself
/// ([`RenameRepository`](tonk_schema::command::RenameRepository), which
/// names its space and is refused cross-space by `may_target_space`),
/// requesting an invite for itself (the space view's share surface
/// dispatches on the space branch and its refusal publishes there),
/// and — until membership moves fully profile-side —
/// [`ExpelMember`](tonk_schema::command::ExpelMember), whose target is
/// likewise the origin space, and installing a library component
/// ([`InstallComponent`](tonk_schema::command::InstallComponent)) into
/// the origin space, which the profile has none of.
///
/// [`CreateSpaceRequest`]: super::repository::CreateSpaceRequest
/// [`InviteRequest`]: super::repository::InviteRequest
/// [`EnableSyncRequest`]: super::repository::EnableSyncRequest
pub fn command_providers() -> CommandProviders {
    use Surface::{Both, Profile, Space};
    CommandProviders::new()
        .command::<super::repository::CreateSpaceRequest>(Profile)
        // A space may request an invite FOR ITSELF: the space view's
        // blank-canvas share (and the seeded `tonk:invite` descriptor)
        // dispatches on the space's own branch, and the refusal flow
        // ("sharing unavailable") publishes to that same branch's
        // overlay. `may_target_space` keeps it self-scoped — a space
        // cannot mint for another space. Profile-side is still the
        // destination once that surface moves.
        .command::<super::repository::InviteRequest>(Both)
        .command::<super::repository::AgentHandoffRequest>(Both)
        .command::<super::repository::EnableSyncRequest>(Profile)
        .command::<tonk_schema::command::Load>(Both)
        .command::<tonk_schema::command::PromoteMember>(Profile)
        .command::<tonk_schema::command::PublishSpace>(Profile)
        .command::<tonk_schema::command::UnpublishSpace>(Profile)
        .command::<tonk_schema::command::InviteAgent>(Profile)
        .command::<tonk_schema::command::InviteByEmail>(Profile)
        .command::<tonk_schema::command::EnrollCustomer>(Profile)
        .command::<tonk_schema::command::ResendActivation>(Profile)
        .command::<tonk_schema::command::DeleteAccount>(Profile)
        .command::<tonk_schema::command::SendEmailInvitation>(Profile)
        .command::<tonk_schema::command::ClaimEmailInvitation>(Profile)
        .command::<super::ceremony::AuthorizeDeviceRequest>(Profile)
        // Signing in through another deployment is new, so neither half
        // has a legacy shape to migrate.
        .command::<tonk_schema::command::SignInVia>(Profile)
        .command::<tonk_schema::command::FinishSignInVia>(Profile)
        .command::<tonk_schema::command::AddRecoveryPhrase>(Profile)
        .migrated::<tonk_schema::command::AddPasskey, tonk_schema::command::legacy::AddPasskey>(Profile)
        .migrated::<tonk_schema::command::ExpelMember, tonk_schema::command::legacy::ExpelMember>(Both)
        .migrated::<tonk_schema::command::RemoveSpace, tonk_schema::command::legacy::RemoveSpace>(Profile)
        .migrated::<tonk_schema::command::Join, tonk_schema::command::legacy::Join>(Profile)
        .migrated::<tonk_schema::command::CheckEmail, tonk_schema::command::legacy::CheckEmail>(Profile)
        .migrated::<tonk_schema::command::RegisterAccount, tonk_schema::command::legacy::RegisterAccount>(Profile)
        .migrated::<tonk_schema::command::PauseSync, tonk_schema::command::legacy::PauseSync>(Profile)
        .migrated::<tonk_schema::command::ProfileRename, tonk_schema::command::legacy::ProfileRename>(Profile)
        .migrated::<tonk_schema::command::RenameRepository, tonk_schema::command::legacy::RenameRepository>(Both)
        // Replication and update checks are the Hub's to ask for: it is
        // the surface that lists spaces this device may not hold, and
        // `ForgetInvite` clears a row that lives on this branch anyway.
        // Switching profiles is new, so it has no legacy shape to migrate.
        .command::<tonk_schema::command::AddProfile>(Profile)
        .command::<tonk_schema::command::SwitchProfile>(Profile)
        .command::<tonk_schema::command::SignOut>(Profile)
        .command::<tonk_schema::command::Home>(Profile)
        // The bar's own acts: each records a request on the asking tab's
        // site, which that tab's bar performs (see `site_request`).
        .command::<tonk_schema::command::AddAccount>(Profile)
        .command::<tonk_schema::command::ShareLink>(Profile)
        .command::<tonk_schema::command::ViewMembers>(Profile)
        .command::<tonk_schema::command::ConnectAgent>(Profile)
        .command::<tonk_schema::command::ConnectSpace>(Profile)
        // Opens the introspection overlay; the same site-request shape,
        // answered by the overlay rather than the bar.
        .command::<tonk_schema::command::Inspect>(Profile)
        // What the page in a tab has selected, recorded on its site.
        .command::<tonk_schema::command::SiteSelect>(Both)
        // A branch a page asked for that its space does not have yet.
        .command::<tonk_schema::command::CreateBranch>(Profile)
        // What was typed in the command palette, interpreted.
        .command::<tonk_schema::command::Interpret>(Both)
        // What the line could mean: answered in the stack's own layer, on
        // each branch the page asks (the space's and the profile's).
        .answering(Both, super::suggest::concept(), super::suggest::respond)
        .command::<tonk_schema::command::ReplicateSpace>(Profile)
        .command::<tonk_schema::command::ForgetInvite>(Profile)
        // A space may ask whether ITS OWN seed is behind: the check
        // targets the origin, and `may_target_space` keeps it there.
        // Replication is deliberately absent from a space — it cannot
        // ask to be pulled onto a device that does not have it yet,
        // because the request would have to arrive on the branch it is
        // asking for.
        .command::<tonk_schema::command::CheckUpdate>(Both)
        // A space installs a library component into itself; the profile
        // has no components to install.
        .command::<tonk_schema::command::InstallComponent>(Space)
}

/// How many answer dispatches one request's chain may run: a provider's
/// answer that lands commands of its own continues it, and nothing in
/// the worker's providers does, so the bound only catches a rule that
/// re-dispatches what it is premised on.
const EFFECT_CHAIN_LIMIT: usize = 16;

/// The branch whose stack `origin`'s commands land on: the profile's
/// own repository for an empty origin repo.
fn stack_of<'a>(
    reactor: &'a crate::Reactor,
    origin: &'a CommandOrigin,
) -> crate::reactor::BranchReference<'a> {
    if origin.repo.is_empty() {
        reactor.profile_repository().branch(&origin.branch)
    } else {
        reactor.repository(&origin.repo).branch(&origin.branch)
    }
}

/// Perform `effect`, the stack's providers over the commands a dispatch
/// landed on the branch at `origin`, then drain the polls the request
/// scheduled. With no effect (a dispatch that landed no command, or
/// wrote nothing), the polls are still drained.
///
/// Called by a mutation path (e.g. `/transact`) after it answered, with
/// `AppState` in hand. The vocabulary is registered on the branch's
/// stack first if this is the branch's first effect
/// ([`CommandProviders::provide`]). Dialog runs, for each command in
/// order, every provider whose concept its rows satisfy, then one answer dispatch
/// through the stack that writes each command's lifecycle row
/// (`dialog.command/answered`, `failed` or `unhandled`), and reports
/// whether that dispatch landed commands of its own, whose effect
/// continues the chain. A command is self-contained: it does its own IO
/// and commits through the [`CommandEnv`], so there's no outcome
/// buffer to commit here, and its answer to dialog is empty.
///
/// The providers run one after another, not concurrently: that is the
/// order dialog performs an effect in.
///
/// TODO(stm): commands have no transactional isolation. A command reads
/// durable state through the env, decides, and commits — but between the
/// read and the commit another commit may have changed what it read. The
/// goal is STM-like optimistic concurrency: track the observed
/// revision/read-set and commit-or-conflict, re-running on conflict.
pub async fn run_commands(state: &AppState, origin: CommandOrigin, effect: Option<Effect>) {
    let operator = {
        let tonk = state.read().await;
        if effect.as_ref().is_some_and(|effect| !effect.is_empty()) {
            match stack_of(&tonk.reactor, &origin)
                .acquire(&tonk.operator)
                .await
            {
                Ok(session) => {
                    if let Err(error) = session.state.provide_once(|stack| {
                        tonk.commands.provide(
                            stack,
                            state.clone(),
                            origin.repo.clone(),
                            origin.branch.clone(),
                        )
                    }) {
                        log!("commands: registering the providers on {origin:?} failed: {error}");
                    }
                }
                Err(error) => {
                    log!("commands: no branch at {origin:?} to provide on: {error}");
                }
            }
        }
        tonk.operator.clone()
    };

    // Nothing here holds the state lock: every provider re-locks through
    // its env.
    let mut effect = effect;
    let mut answered = false;
    for _ in 0..EFFECT_CHAIN_LIMIT {
        let Some(next) = effect.take().filter(|effect| !effect.is_empty()) else {
            break;
        };
        match next.perform(&operator).await {
            Ok(report) => {
                report_effect(&origin, &report);
                answered |= !report.answered().is_empty();
                if !report.remaining() {
                    break;
                }
                effect = Some(report.effect());
            }
            Err(error) => {
                log!("commands: the answer dispatch at {origin:?} failed: {error}");
                break;
            }
        }
    }

    // Drain every poll the request scheduled — the triggering commit plus
    // anything its providers committed or wrote to the overlay — in one
    // pass. This is the single point that turns scheduled writes into
    // subscription broadcasts; coalesced by branch identity. An answer a
    // provider gave lands through the effect's own dispatch, outside the
    // reactor's commit, in a layer the branch's subscriptions read
    // (`intent/suggest` answers in `session:`), so the branch is polled
    // for it too.
    let tonk = state.read().await;
    if answered
        && let Ok(session) = stack_of(&tonk.reactor, &origin)
            .acquire(&tonk.operator)
            .await
    {
        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&session.state));
    }
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Log what an effect did not do: a provider that failed, with its
/// reason, and a command no provider claims. The latter is the one
/// place a command whose rows satisfy no concept can be seen: the
/// command landed, so the page believes it asked.
fn report_effect(origin: &CommandOrigin, report: &EffectReport) {
    let surface = if origin.repo.is_empty() {
        "the profile".to_owned()
    } else {
        format!("space '{}'", origin.repo)
    };
    for (entity, reason) in report.failed() {
        log!("commands: {entity} on {surface}: {reason}");
    }
    for entity in report.unhandled() {
        log!("commands: no provider on {surface} claims {entity}");
    }
}

/// Send `commands`, sent for the branch at `origin` which has no stack —
/// an absent repository, a branch nobody has made yet — through the
/// profile's stack, their origin in the tick's rows, and perform their
/// effect there. A command for such a branch has nothing durable to
/// land beside and no library whose rules could read it, so the
/// profile's stack, which every worker holds, carries it to its
/// provider; the provider takes the origin from the tick
/// ([`CommandOrigin::of_tick`]), so it acts on the branch the command
/// was sent for.
pub async fn relay_commands(state: &AppState, origin: CommandOrigin, commands: Changes) {
    let profile = {
        let tonk = state.read().await;
        CommandOrigin {
            repo: String::new(),
            branch: tonk.active_branch.clone(),
            client: origin.client.clone(),
        }
    };
    let mut sent = Changes::new();
    Tick {
        client: origin.client.clone(),
        repo: Some(origin.repo.clone()),
        branch: Some(origin.branch.clone()),
    }
    .stamp(&mut sent);
    sent.merge(commands);
    match invoke_on(state, &profile, sent).await {
        Ok(effect) => run_commands(state, profile, Some(effect)).await,
        Err(error) => {
            log!("commands: relaying {origin:?}'s commands through the profile failed: {error}")
        }
    }
}

/// Invoke `commands` through the stack of the branch at `origin`,
/// under its transactor lock, and return the effect of what landed.
async fn invoke_on(
    state: &AppState,
    origin: &CommandOrigin,
    commands: Changes,
) -> Result<Effect, crate::TonkWorkerError> {
    let tonk = state.read().await;
    let session = stack_of(&tonk.reactor, origin)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| crate::TonkWorkerError::NotFound(error.to_string()))?;
    let _transacting = session.transactor().lock().await;
    let report = session
        .state
        .stack()
        .invoke(commands)
        .perform(&tonk.operator)
        .await
        .map_err(|error| crate::TonkWorkerError::Internal(error.to_string()))?;
    Ok(report.effect())
}

/// The trigger attributes of the key commands: those that change who may
/// use a space or what it is called (`tonk/invite`, `member/expel`,
/// `tonk/rename-repository`), in their current and legacy shapes. A write
/// that dispatches a claim of any of them runs a key command, however the
/// document spelled it.
pub fn key_command_attributes() -> Vec<String> {
    use crate::reactor::{Decode as _, Migrated};
    use tonk_schema::command::{ExpelMember, RenameRepository, legacy};
    let mut attributes = super::repository::InviteRequest::trigger_attributes();
    attributes.extend(
        Migrated::<ExpelMember, legacy::ExpelMember>::new()
            .trigger_attributes()
            .iter()
            .cloned(),
    );
    attributes.extend(
        Migrated::<RenameRepository, legacy::RenameRepository>::new()
            .trigger_attributes()
            .iter()
            .cloned(),
    );
    // A space renames itself through `core.yaml`'s rule over the space's
    // own `tonk/rename-repository`, whose attributes are not the
    // handler's.
    attributes.extend(
        [
            "xyz.tonk.rename-repository/subject",
            "xyz.tonk.rename-repository/name",
        ]
        .map(str::to_owned),
    );
    attributes.sort();
    attributes.dedup();
    attributes
}

/// Invoke `commands` as the branch at `origin` would land them — through
/// its stack when it has one, the client stamped on the tick, or relayed through the profile's when it has none — and
/// perform their effect: what a route does in two steps, for a test or
/// a caller holding the rows rather than a request.
#[cfg(test)]
pub(crate) async fn invoke_commands(state: &AppState, origin: CommandOrigin, commands: Changes) {
    let mut sent = Changes::new();
    if let Some(client) = &origin.client {
        client_row(&mut sent, client);
    }
    sent.merge(commands.clone());
    match invoke_on(state, &origin, sent).await {
        Ok(effect) => run_commands(state, origin, Some(effect)).await,
        Err(crate::TonkWorkerError::NotFound(_)) => relay_commands(state, origin, commands).await,
        Err(error) => log!("commands: invoking at {origin:?} failed: {error}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(missing_docs)]

    use super::*;
    use dialog_corpus::Statement;
    use dialog_query::{Attribute, Concept, Entity, the};
    use std::sync::Mutex;

    // A test command whose provider RECORDS each invocation's tag and
    // the client it ran for, so a test can observe whether (and with
    // what) `execute` ran. The provider does no IO.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct PingTag(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Ping {
        pub this: Entity,
        pub tag: PingTag,
    }

    impl dialog_capability::Command for Ping {
        type Input = Self;
        type Output = ();
    }

    /// What `Ping`'s provider was handed, in invocation order: the tag
    /// and the client the env carried. The tests `drain` it; serialized
    /// within the single-threaded test run.
    static PING_LOG: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

    /// The tests that read `Ping`'s log drain it, so they take turns: one
    /// running beside another would read, or take, its entries.
    pub(crate) async fn ping_turn() -> tokio::sync::MutexGuard<'static, ()> {
        static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        TURN.lock().await
    }

    /// Take and clear the recorded invocations. The provider writes it
    /// on every target; the tests here and the transact route's native
    /// test read it.
    pub(crate) fn drain_ping_log() -> Vec<(String, Option<String>)> {
        std::mem::take(&mut *PING_LOG.lock().unwrap())
    }

    /// The tags of the recorded invocations alone.
    pub(crate) fn drain_ping_tags() -> Vec<String> {
        drain_ping_log().into_iter().map(|(tag, _)| tag).collect()
    }

    #[dialog_capability::provider]
    impl dialog_capability::Provider<Ping> for CommandEnv {
        async fn execute(&self, command: Ping) {
            PING_LOG
                .lock()
                .unwrap()
                .push((command.tag.0, self.client().map(|client| client.0.clone())));
        }
    }

    // A test command sharing `Ping`'s tag attribute, registered before
    // it: dialog hands it every `Ping`, whose rows lack its mark.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct EchoMark(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Echo {
        pub this: Entity,
        pub tag: PingTag,
        pub mark: EchoMark,
    }

    impl dialog_capability::Command for Echo {
        type Input = Self;
        type Output = ();
    }

    #[dialog_capability::provider]
    impl dialog_capability::Provider<Echo> for CommandEnv {
        async fn execute(&self, command: Echo) {
            PING_LOG
                .lock()
                .unwrap()
                .push((format!("echo:{}", command.tag.0), None));
        }
    }

    // A test command of its own for the evaluate route's test, logging
    // apart from `Ping`, so tests running beside it do not take its
    // entries.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct EvaluatedNote(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Evaluated {
        pub this: Entity,
        pub note: EvaluatedNote,
    }

    impl dialog_capability::Command for Evaluated {
        type Input = Self;
        type Output = ();
    }

    /// What `Evaluated`'s provider was handed.
    static EVALUATED_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    #[dialog_capability::provider]
    impl dialog_capability::Provider<Evaluated> for CommandEnv {
        async fn execute(&self, command: Evaluated) {
            EVALUATED_LOG.lock().unwrap().push(command.note.0);
        }
    }

    fn ping_transient(of: &str, tag: &str) -> Changes {
        let mut changes = Changes::new();
        // `PingTag`'s attribute name snake-cases the struct name.
        the!("xyz.tonk.command/ping-tag")
            .of(of.parse::<Entity>().expect("entity URI"))
            .is(tag.to_string())
            .assert(&mut changes);
        changes
    }

    /// The rows `changes` asserts, as a landed command's rows are.
    fn rows_of(changes: &Changes) -> Vec<Fact> {
        changes
            .clone()
            .into_instructions()
            .into_iter()
            .map(|instruction| match instruction {
                dialog_corpus::Instruction::Assert(artifact, _)
                | dialog_corpus::Instruction::Retract(artifact) => artifact,
            })
            .collect()
    }

    /// The registrations whose providers claim the command `changes`
    /// asserts, and admit it from `origin`: what dialog runs every one
    /// of.
    fn claiming<'a>(
        providers: &'a CommandProviders,
        origin: &CommandOrigin,
        changes: &Changes,
    ) -> Vec<&'a Registration> {
        let rows = rows_of(changes);
        providers
            .registrations()
            .iter()
            .filter(|registration| registration.claims(&rows) && registration.admits(origin))
            .collect()
    }

    #[dialog_common::test]
    fn it_registers_a_command_type_by_its_provider() {
        // `command::<Ping>()` compiles only because `CommandEnv:
        // Provider<Ping>` — the capability gate. The registered type
        // claims its trigger.
        let providers = command_providers().command::<Ping>(Surface::Both);
        let changes = ping_transient("did:key:zPing", "hi");
        assert_eq!(
            claiming(&providers, &CommandOrigin::default(), &changes).len(),
            1,
            "the registered Ping command should claim its trigger"
        );
    }

    /// Dialog runs every provider whose concept a command's rows satisfy
    /// (every required field present), so a command of one concept also
    /// runs the provider of any other registration's concept whose
    /// required fields are among its own: as the worker's registry ran
    /// every command that decoded. Such a pair is known, among the legacy
    /// shapes: a legacy `RegisterAccount` carries the email that is all
    /// a legacy `CheckEmail` requires, so both run, as they did in the
    /// registry. A new pair fails this test, so it is added knowingly,
    /// with a test of what both commands do with it. A concept is
    /// registered once: dialog keeps one provider per concept.
    #[dialog_common::test]
    fn a_commands_rows_satisfy_no_other_commands_concept_unknowingly() {
        use std::collections::{BTreeMap, BTreeSet};
        let providers = command_providers();
        let mut seen: BTreeSet<String> = Default::default();
        // Each concept's required attributes, with its registration. A
        // concept's entity is a hash, so a shape is named by what it
        // requires.
        let mut required: Vec<(usize, BTreeSet<String>)> = Vec::new();
        for (index, registration) in providers.registrations().iter().enumerate() {
            for concept in registration.concepts() {
                let name = concept.this().to_string();
                assert!(
                    seen.insert(name.clone()),
                    "{name} is registered twice; dialog keeps one provider per concept"
                );
                required.push((
                    index,
                    concept
                        .with()
                        .iter()
                        .filter(|(_, field)| !field.is_optional())
                        .map(|(_, field)| field.the().to_string())
                        .collect(),
                ));
            }
        }
        // A command of `a` satisfies `b` when `b` requires nothing `a`
        // does not: then `b`'s provider runs it too.
        let mut also_runs: BTreeMap<Vec<String>, BTreeSet<Vec<String>>> = Default::default();
        for (i, of_a) in &required {
            for (j, of_b) in &required {
                if i != j && of_b.is_subset(of_a) {
                    also_runs
                        .entry(of_a.iter().cloned().collect())
                        .or_default()
                        .insert(of_b.iter().cloned().collect());
                }
            }
        }
        let email = "dom.event.current-target.elements.email/value".to_owned();
        assert_eq!(
            also_runs,
            [(
                vec![
                    "dom.event.current-target.dataset/register-account".to_owned(),
                    email.clone()
                ],
                [vec![email]].into_iter().collect()
            )]
            .into_iter()
            .collect(),
            "a legacy register-account (its email and marker) is the only command whose rows satisfy another command's concept (a legacy check-email: the email alone)"
        );
    }

    /// The vocabulary split itself: every transient here is CLAIMED by
    /// its command (each shape is admitted at least once on the
    /// profile), so a zero on a space origin means the command is
    /// refused there — not that the probe was malformed.
    #[dialog_common::test]
    fn a_space_origin_is_refused_the_profile_only_commands() {
        let providers = command_providers();
        let profile = CommandOrigin::default();
        let space = CommandOrigin {
            repo: "did:key:zSomeSpace".to_string(),
            branch: "main".to_string(),
            client: None,
        };
        let entity = |uri: &str| uri.parse::<Entity>().expect("entity URI");

        // Profile-only shapes: lifecycle, joining, membership grants.
        let mut create = Changes::new();
        the!("xyz.tonk.command.create-space/name")
            .of(entity("cmd:create"))
            .is("Probe".to_string())
            .assert(&mut create);

        let mut remove = Changes::new();
        the!("xyz.tonk.command.remove-space/subject")
            .of(entity("cmd:remove"))
            .is(entity("did:key:zVictimSpace"))
            .assert(&mut remove);

        let mut join = Changes::new();
        the!("xyz.tonk.command.join/url")
            .of(entity("cmd:join"))
            .is("https://tonk.xyz/join#secret".to_string())
            .assert(&mut join);

        let mut promote = Changes::new();
        the!("xyz.tonk.promote/member")
            .of(entity("cmd:promote"))
            .is(entity("did:key:zMember"))
            .assert(&mut promote);
        the!("xyz.tonk.promote/space")
            .of(entity("cmd:promote"))
            .is(entity("did:key:zSomeSpace"))
            .assert(&mut promote);
        the!("xyz.tonk.promote/chain")
            .of(entity("cmd:promote"))
            .is("z6chain".to_string())
            .assert(&mut promote);

        for (name, changes) in [
            ("space/create", &create),
            ("space/remove", &remove),
            ("tonk/join", &join),
            ("member/promote", &promote),
        ] {
            assert!(
                !claiming(&providers, &profile, changes).is_empty(),
                "`{name}` must be claimed on the profile — a zero here means the \
                 probe shape no longer matches and this test proves nothing"
            );
            assert!(
                claiming(&providers, &space, changes).is_empty(),
                "`{name}` must be refused from a space branch"
            );
        }

        // Space-side shapes: a space acting on itself. These stay
        // dispatchable from the profile too — the FAB is routeless.
        let mut load = Changes::new();
        the!("xyz.tonk.site/path")
            .of(entity("site:probe"))
            .is("/".to_string())
            .assert(&mut load);

        let mut rename = Changes::new();
        the!("xyz.tonk.command.rename-repository/name")
            .of(entity("cmd:rename"))
            .is("Renamed".to_string())
            .assert(&mut rename);
        the!("xyz.tonk.rename-repository/space")
            .of(entity("cmd:rename"))
            .is(entity("did:key:zSomeSpace"))
            .assert(&mut rename);

        let mut expel = Changes::new();
        the!("xyz.tonk.command.expel-member/member")
            .of(entity("cmd:expel"))
            .is(entity("did:key:zMember"))
            .assert(&mut expel);

        for (name, changes) in [
            ("tonk/load", &load),
            ("tonk/rename-repository", &rename),
            ("member/expel", &expel),
        ] {
            for (surface, origin) in [("profile", &profile), ("space", &space)] {
                assert!(
                    !claiming(&providers, origin, changes).is_empty(),
                    "`{name}` must be dispatchable from the {surface}"
                );
            }
        }
    }

    /// The providers' run itself, natively over real state: a command
    /// invoked through the profile's stack reaches `Ping`'s provider
    /// through dialog's effect, with what the env carried.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    mod run {
        use super::*;
        use crate::router::AppState;

        /// A state whose vocabulary is `Ping` alone.
        async fn state_with_ping() -> AppState {
            let state = native::test_state().await;
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands = CommandProviders::new().command::<Ping>(Surface::Both);
            }
            state
        }

        #[dialog_common::test]
        async fn it_runs_the_provider_with_the_decoded_command() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = state_with_ping().await;
            invoke_commands(
                &state,
                CommandOrigin::default(),
                ping_transient("did:key:zPing", "hello"),
            )
            .await;
            assert_eq!(
                drain_ping_tags(),
                vec!["hello".to_string()],
                "the provider should run once with the decoded tag"
            );
        }

        /// A command reaches its provider with the client that invoked
        /// it: the route stamps the client on the tick, a row of the
        /// `control:` entity beside the command, and the provider takes
        /// it from the tick. This is the path a navigation command
        /// (`tonk:home`) takes to post back to the tab that asked.
        #[dialog_common::test]
        async fn a_command_reaches_its_provider_with_the_client_that_invoked_it() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = state_with_ping().await;
            invoke_commands(
                &state,
                CommandOrigin {
                    client: Some(crate::router::ClientId("tab-42".to_owned())),
                    ..CommandOrigin::default()
                },
                ping_transient("did:key:zPing", "navigate"),
            )
            .await;
            assert_eq!(
                drain_ping_log(),
                vec![("navigate".to_string(), Some("tab-42".to_string()))],
                "the provider sees the client the command was invoked with"
            );

            // Without a client, the provider sees none: the row is the
            // only channel, and nothing is left over from the last one.
            invoke_commands(
                &state,
                CommandOrigin::default(),
                ping_transient("did:key:zPing", "nobody"),
            )
            .await;
            assert_eq!(drain_ping_log(), vec![("nobody".to_string(), None)],);
        }

        #[dialog_common::test]
        async fn it_runs_every_decoded_command_in_a_batch() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = state_with_ping().await;

            // Two distinct Ping entities in one batch → two invocations.
            let mut changes = ping_transient("did:key:zA", "alpha");
            the!("xyz.tonk.command/ping-tag")
                .of("did:key:zB".parse::<Entity>().unwrap())
                .is("beta".to_string())
                .assert(&mut changes);

            invoke_commands(&state, CommandOrigin::default(), changes).await;

            let mut tags = drain_ping_tags();
            tags.sort();
            assert_eq!(
                tags,
                vec!["alpha".to_string(), "beta".to_string()],
                "the effect runs each decoded command's provider"
            );
        }

        /// Every provider whose concept a command's rows satisfy runs.
        /// A `Ping` (its tag alone) satisfies `Ping` and not `Echo`,
        /// which requires its mark too, so `Ping`'s provider alone runs
        /// it though `Echo`, registered first, shares the tag; an `Echo`
        /// (tag and mark) satisfies both, so both run, in registration
        /// order, as the worker's registry ran every command that
        /// decoded.
        #[dialog_common::test]
        async fn every_provider_whose_concept_the_rows_satisfy_runs() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = native::test_state().await;
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands = CommandProviders::new()
                    .command::<Echo>(Surface::Both)
                    .command::<Ping>(Surface::Both);
            }
            invoke_commands(
                &state,
                CommandOrigin::default(),
                ping_transient("did:key:zPing", "plain"),
            )
            .await;
            assert_eq!(
                drain_ping_tags(),
                vec!["plain".to_string()],
                "a ping satisfies Ping alone: Echo requires its mark"
            );

            let mut echo = ping_transient("did:key:zEcho", "loud");
            the!("xyz.tonk.command/echo-mark")
                .of("did:key:zEcho".parse::<Entity>().unwrap())
                .is("!".to_string())
                .assert(&mut echo);
            invoke_commands(&state, CommandOrigin::default(), echo).await;
            assert_eq!(
                drain_ping_tags(),
                vec!["echo:loud".to_string(), "loud".to_string()],
                "an echo satisfies Echo and Ping: both run, in registration order"
            );
        }

        /// Perform `rows` as a command landed on the branch at `origin`,
        /// the vocabulary registered on its stack first, and report what
        /// the effect did.
        async fn perform(state: &AppState, origin: &CommandOrigin, rows: Changes) -> EffectReport {
            let operator = {
                let tonk = state.read().await;
                let session = stack_of(&tonk.reactor, origin)
                    .acquire(&tonk.operator)
                    .await
                    .expect("the branch opens");
                session
                    .state
                    .provide_once(|stack| {
                        tonk.commands.provide(
                            stack,
                            state.clone(),
                            origin.repo.clone(),
                            origin.branch.clone(),
                        )
                    })
                    .expect("the vocabulary registers");
                tonk.operator.clone()
            };
            let effect = invoke_on(state, origin, rows)
                .await
                .expect("the command lands");
            effect
                .perform(&operator)
                .await
                .expect("the effect performs")
        }

        /// The one overlap the vocabulary keeps, deliberately. A legacy
        /// `RegisterAccount` (the form's email and its marker) also
        /// satisfies the legacy `CheckEmail` (the email alone), so both
        /// providers run, as the worker's registry ran both: two answered
        /// rows, and the lookup's `EmailStatus` row, the one the same
        /// form produces on every keystroke. Nobody current posts the
        /// legacy shapes (the register dialog posts the current ones,
        /// whose attributes are disjoint, so a current `RegisterAccount`
        /// satisfies its concept alone), and a shell from before the
        /// migration cannot be given a marker, so the overlap stays
        /// rather than leaving those shells' lookups unhandled.
        #[dialog_common::test]
        async fn a_legacy_register_account_runs_the_lookup_too() {
            let state = native::test_state().await;
            let origin = {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands = command_providers();
                CommandOrigin {
                    repo: String::new(),
                    branch: tonk.active_branch.clone(),
                    client: None,
                }
            };
            let address = "ada@example.com";
            let entity = |uri: &str| uri.parse::<Entity>().expect("entity URI");

            let legacy = entity("cmd:legacy-register");
            let mut rows = Changes::new();
            the!("dom.event.current-target.elements.email/value")
                .of(legacy.clone())
                .is(address.to_string())
                .assert(&mut rows);
            the!("dom.event.current-target.dataset/register-account")
                .of(legacy.clone())
                .is(entity("tonk:register-account"))
                .assert(&mut rows);
            let report = perform(&state, &origin, rows).await;
            assert_eq!(
                report.answered(),
                &[legacy.clone(), legacy.clone()],
                "the registration and the lookup both answer a legacy register-account"
            );
            assert!(report.failed().is_empty(), "{:?}", report.failed());
            assert!(report.unhandled().is_empty(), "{:?}", report.unhandled());
            let looked_up = {
                let tonk = state.read().await;
                let session = stack_of(&tonk.reactor, &origin)
                    .acquire(&tonk.operator)
                    .await
                    .expect("the branch opens");
                session
                    .handle()
                    .overlay()
                    .export()
                    .iter()
                    .any(|(_, attribute, change)| {
                        attribute.to_string() == "xyz.tonk.email-status/address"
                            && matches!(
                                change,
                                dialog_corpus::Change::Assert(Value::String(text), _) if text == address
                            )
                    })
            };
            assert!(looked_up, "the lookup answered the address on the overlay");

            let current = entity("cmd:register");
            let mut rows = Changes::new();
            the!("xyz.tonk.command.register-account/email")
                .of(current.clone())
                .is(address.to_string())
                .assert(&mut rows);
            let report = perform(&state, &origin, rows).await;
            assert_eq!(
                report.answered(),
                &[current],
                "a current register-account satisfies its own concept alone"
            );
            assert!(report.failed().is_empty(), "{:?}", report.failed());
        }

        /// A command a document asserts through the evaluate route is
        /// answered by its provider, as one asserted through the transact
        /// route is: the document commits through the branch's stack, and
        /// the publish's report hands its commands to the providers.
        #[dialog_common::test]
        async fn the_evaluate_route_answers_a_command_through_its_provider() {
            let _turn = ping_turn().await;
            use axum::body::Body;
            use axum::http::{Request, StatusCode};
            use tower::ServiceExt as _;

            let state = native::test_state().await;
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands =
                    CommandProviders::new().command::<Evaluated>(Surface::Both);
            }
            let branch = state.read().await.active_branch.clone();
            let (app, _lsp) = crate::api_router_from_state(state.clone());
            let evaluate = |document: &'static str| {
                let app = app.clone();
                let uri = format!("/api/repository/profile:tonk/branch/{branch}/evaluate");
                async move {
                    let response = app
                        .oneshot(
                            Request::builder()
                                .uri(uri)
                                .method("POST")
                                .header("content-type", "application/yaml")
                                .body(Body::from(document))
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(response.status(), StatusCode::OK);
                }
            };
            evaluate(
                "command!: &test/note\n  description: Note.\n  with:\n    note:\n      description: The note.\n      the: xyz.tonk.command/evaluated-note\n      as: text\n",
            )
            .await;
            assert!(
                EVALUATED_LOG.lock().unwrap().is_empty(),
                "a declaration runs nothing"
            );

            evaluate("test/note!:\n  note: \"evaluated\"\n").await;
            assert_eq!(
                std::mem::take(&mut *EVALUATED_LOG.lock().unwrap()),
                vec!["evaluated".to_string()],
                "the provider answers the command the document asserted"
            );
        }

        #[dialog_common::test]
        async fn it_runs_nothing_when_no_command_decodes() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = state_with_ping().await;
            let mut changes = Changes::new();
            the!("xyz.tonk.unrelated/noise")
                .of("did:key:zNoise".parse::<Entity>().unwrap())
                .is("x".to_string())
                .assert(&mut changes);

            invoke_commands(&state, CommandOrigin::default(), changes).await;
            assert!(drain_ping_log().is_empty());
        }

        /// A command the surface refuses never reaches its provider: a
        /// `Profile` command sent from a space origin fails in dialog's
        /// effect before `execute`.
        #[dialog_common::test]
        async fn a_surface_refuses_a_command_from_the_wrong_origin() {
            let _turn = ping_turn().await;
            let _ = drain_ping_log();
            let state = native::test_state().await;
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands =
                    CommandProviders::new().command::<Ping>(Surface::Profile);
            }
            invoke_commands(
                &state,
                CommandOrigin {
                    repo: "did:key:zSomeSpace".to_owned(),
                    branch: "main".to_owned(),
                    client: None,
                },
                ping_transient("did:key:zPing", "contained"),
            )
            .await;
            assert!(
                drain_ping_log().is_empty(),
                "a profile-only command must not run from a space origin"
            );
        }
    }

    /// Native end-to-end dispatch over REAL commands — the test the
    /// whole target-agnostic registry exists for. Its failure mode is
    /// silent absence: a `command_providers()` whose native arm returned
    /// an empty registry compiled green while no command ran anywhere
    /// but the browser, so nothing short of dispatching a real command
    /// against real state proves the conversion means anything.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(crate) mod native {
        use super::*;
        use crate::router::AppState;
        use crate::worker::TonkState;
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::Replica;
        use tonk_schema::prelude::DidExt as _;

        /// A full native `TonkState` over default storage — the same
        /// construction `account_state`'s native tests use, minus the
        /// access service (nothing here needs an account). The registry
        /// installed is the REAL one, not a test double.
        pub(crate) async fn test_state() -> AppState {
            let name = format!("command-dispatch-test-{}", rand::random::<u64>());
            // Isolate the directory-level system key as well as the profile.
            let directory = dialog_effects::storage::Directory::At(
                std::env::temp_dir()
                    .join(&name)
                    .to_string_lossy()
                    .into_owned(),
            );
            let (storage, profile) = crate::device::open_profile_at(&name, directory.clone())
                .await
                .unwrap();
            let session = crate::session::open(&profile).await.unwrap();
            let reactor = crate::Reactor::new(profile.credential().clone());
            let state = TonkState::from_parts(
                session.operator,
                session.expires_at,
                crate::router::repository::PROFILE_BRANCH.to_owned(),
                crate::worker::TonkShared {
                    profile,
                    storage,
                    profile_name: name.clone(),
                    reactor,
                    admission: Default::default(),
                    reject_admission_content_reads: Default::default(),
                    retiring: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    view_bindings: Default::default(),
                    bridges: Default::default(),
                    sync_queue: Default::default(),
                    commands: crate::router::command_providers(),
                    clients: Default::default(),
                    seed_upgrades: Default::default(),
                    account_keys: Default::default(),
                    profile_library: Default::default(),
                    registry: crate::device::Registry::new(name.clone(), directory),
                    profile_transition: Default::default(),
                    context_generation: Default::default(),
                    profile_swaps: Default::default(),
                },
            );
            crate::router::repository::bootstrap_profile(&state)
                .await
                .unwrap();
            std::sync::Arc::new(crate::StateLock::new(state))
        }

        /// The subject DIDs of every user-space replica the profile
        /// lists — the Hub's source of truth, read the same way
        /// `require_real_space` reads it.
        async fn space_subjects(state: &AppState) -> Vec<dialog_varsig::Did> {
            let tonk = state.read().await;
            let meta = tonk
                .reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await
                .unwrap();
            let rows: Vec<Replica> = meta
                .handle()
                .query()
                .select(Query::<Replica> {
                    this: Term::var("this"),
                    subject: Term::var("subject"),
                    profile: Term::var("profile"),
                    kind: Term::var("kind"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .unwrap();
            rows.into_iter()
                .filter(|replica| replica.kind == Replica::repository_kind())
                .filter_map(|replica| replica.subject.0.to_string().parse().ok())
                .collect()
        }

        /// The exact transient the Hub's "New space" wizard commits.
        fn create_space_transient(name: &str) -> Changes {
            let mut changes = Changes::new();
            the!("xyz.tonk.command.create-space/name")
                .of("cmd:create".parse::<Entity>().unwrap())
                .is(name.to_string())
                .assert(&mut changes);
            changes
        }

        /// The exact transient the Hub row's delete confirm commits.
        fn remove_space_transient(subject: &dialog_varsig::Did) -> Changes {
            let mut changes = Changes::new();
            the!("xyz.tonk.command.remove-space/subject")
                .of("cmd:remove".parse::<Entity>().unwrap())
                .is(subject.this())
                .assert(&mut changes);
            changes
        }

        #[dialog_common::test]
        async fn it_dispatches_space_create_and_remove_natively_end_to_end() {
            let state = test_state().await;
            assert!(
                space_subjects(&state).await.is_empty(),
                "a fresh profile lists no spaces"
            );

            // Create: the same shape the Hub form posts, dispatched from
            // the profile origin (empty repo — `transact_profile` never
            // names one). The provider mints an identity, seeds the
            // standard library from the embedded assets, and records the
            // replica — all natively.
            invoke_commands(
                &state,
                CommandOrigin::default(),
                create_space_transient("Command Test Space"),
            )
            .await;
            let spaces = space_subjects(&state).await;
            assert_eq!(
                spaces.len(),
                1,
                "dispatching space/create natively must mint and record a space"
            );
            let subject = spaces[0].clone();

            // Containment: the same-shaped remove fact committed on a
            // content branch names the space by DID but must be ignored —
            // space A cannot delete space B (`CommandEnv::may_target_space`,
            // and `RemoveSpace`'s stricter profile-only rule).
            let foreign = CommandOrigin {
                repo: "did:key:zSomeOtherSpace".to_string(),
                branch: "main".to_string(),
                client: None,
            };
            invoke_commands(&state, foreign, remove_space_transient(&subject)).await;
            assert_eq!(
                space_subjects(&state).await.len(),
                1,
                "a content-branch origin must not remove a space by DID"
            );

            // From the profile origin the same transient removes it.
            invoke_commands(
                &state,
                CommandOrigin::default(),
                remove_space_transient(&subject),
            )
            .await;
            assert!(
                space_subjects(&state).await.is_empty(),
                "dispatching space/remove from the profile must remove the space"
            );
        }

        /// Serve `files` (path → body) over plain HTTP/1.1 on a loopback
        /// port, answering anything else 404, and return the base URL.
        fn serve(files: &'static [(&'static str, &'static str)]) -> String {
            use std::io::{BufRead as _, BufReader, Write as _};

            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    let _ = reader.read_line(&mut request);
                    let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                    // Drain the headers so the client sees a clean response.
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                        line.clear();
                    }
                    let mut stream = stream;
                    let response = match files.iter().find(|(file, _)| *file == path) {
                        Some((_, body)) => format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        ),
                        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned(),
                    };
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            base
        }

        /// `create_space_transient` carrying a seed URL, as the /seed
        /// page's dialog commits it.
        fn seeded_create_transient(name: &str, seed: &str) -> Changes {
            let mut changes = create_space_transient(name);
            the!("xyz.tonk.command.create-space/seed")
                .of("cmd:create".parse::<Entity>().unwrap())
                .is(seed.to_string())
                .assert(&mut changes);
            changes
        }

        #[dialog_common::test]
        async fn it_dispatches_duplicate_space_only_from_the_profile() {
            let state = test_state().await;
            invoke_commands(
                &state,
                CommandOrigin::default(),
                create_space_transient("Original"),
            )
            .await;
            let original = space_subjects(&state).await[0].clone();
            let copy_command = || {
                let mut changes = create_space_transient("Copy of Original");
                the!("xyz.tonk.command.create-space/copy-from")
                    .of("cmd:create".parse::<Entity>().unwrap())
                    .is(original.this())
                    .assert(&mut changes);
                changes
            };
            invoke_commands(
                &state,
                CommandOrigin {
                    repo: original.repo_key().to_owned(),
                    branch: "main".into(),
                    client: None,
                },
                copy_command(),
            )
            .await;
            assert_eq!(space_subjects(&state).await.len(), 1);
            invoke_commands(&state, CommandOrigin::default(), copy_command()).await;
            let spaces = space_subjects(&state).await;
            assert_eq!(spaces.len(), 2);
            assert!(spaces.contains(&original));
            assert_ne!(spaces[0], spaces[1]);

            // A malformed source is a failed copy, never a fallback to blank creation.
            let mut invalid = create_space_transient("Invalid copy");
            the!("xyz.tonk.command.create-space/copy-from")
                .of("cmd:create".parse::<Entity>().unwrap())
                .is(false)
                .assert(&mut invalid);
            invoke_commands(&state, CommandOrigin::default(), invalid).await;
            assert_eq!(space_subjects(&state).await.len(), 2);
        }

        const SEED_FILES: &[(&str, &str)] = &[
            (
                "/lib/seed.yaml",
                "xyz.test.seed!:\n  this: id:seeded\n  body: !include/text ./body.txt\n",
            ),
            ("/lib/body.txt", "hello from a seed\n"),
        ];

        /// SPACE-15: fetch the catalog and ordered required files before
        /// allocating a space. Optional files are deliberately unavailable.
        #[dialog_common::test]
        async fn it_creates_from_a_remote_catalog_and_refuses_changed_sources() {
            const CATALOG: &str = include_str!("../../tests/fixtures/discover/catalog.json");
            const MODEL: &str = include_str!("../../tests/fixtures/discover/model.yaml");
            const VIEW: &str = include_str!("../../tests/fixtures/discover/view.yaml");
            for (view, expected) in [(VIEW, 1), ("changed after catalog publication", 0)] {
                let files = Box::leak(Box::new([
                    ("/catalog.json", CATALOG),
                    ("/model.yaml", MODEL),
                    ("/view.yaml", view),
                ]));
                let base = serve(files);
                let state = test_state().await;
                let mut changes = create_space_transient("Remote template");
                dialog_query::the!("xyz.tonk.command.create-space/template")
                    .of("cmd:create".parse::<Entity>().unwrap())
                    .is(format!("{base}/catalog.json#remote-demo"))
                    .assert(&mut changes);
                invoke_commands(&state, CommandOrigin::default(), changes).await;
                assert_eq!(space_subjects(&state).await.len(), expected);
            }
        }

        /// A seeded `space/create` evaluates the document at the seed URL
        /// into the new space, on top of the standard library, with what it
        /// includes resolved beside it on the seed's server.
        #[dialog_common::test]
        async fn it_seeds_a_new_space_from_a_url() {
            let base = serve(SEED_FILES);
            let state = test_state().await;
            invoke_commands(
                &state,
                CommandOrigin::default(),
                seeded_create_transient("Seeded", &format!("{base}/lib/seed.yaml")),
            )
            .await;
            let spaces = space_subjects(&state).await;
            assert_eq!(spaces.len(), 1, "the seeded create must mint a space");

            let key = spaces[0].repo_key().to_owned();
            let tonk = state.read().await;
            let response = crate::router::evaluate::evaluate_body(
                &tonk,
                &key,
                "main",
                "xyz.test.seed:\n  this: id:seeded\n  body: ?body\n".to_owned(),
                false,
            )
            .await
            .unwrap();
            let body = response.matches_after[0].results[0]
                .fields
                .get("body")
                .cloned();
            assert_eq!(
                body,
                Some(serde_json::Value::String("hello from a seed\n".to_owned())),
                "the seed's included text must be in the new space"
            );
        }

        /// A seed that cannot be used fails the create before anything is
        /// created: no half-seeded space is left behind.
        #[dialog_common::test]
        async fn it_creates_nothing_when_the_seed_cannot_be_used() {
            let base = serve(SEED_FILES);
            let state = test_state().await;
            for seed in [
                format!("{base}/lib/missing.yaml"),
                "ftp://example.test/seed.yaml".to_owned(),
                "not a url".to_owned(),
            ] {
                invoke_commands(
                    &state,
                    CommandOrigin::default(),
                    seeded_create_transient("Refused", &seed),
                )
                .await;
                assert!(
                    space_subjects(&state).await.is_empty(),
                    "an unusable seed ({seed}) must not create a space"
                );
            }
        }
    }

    /// Every attribute of the key commands as `core.yaml` declares them is
    /// a key command attribute, so a space's own spelling of one is
    /// refused unconfirmed as the handler's is.
    #[dialog_common::test]
    fn the_key_command_attributes_cover_the_core_library() {
        let key = super::key_command_attributes();
        let core = include_str!("../../../tonk-core/assets/library/core.yaml");
        for attribute in [
            "xyz.tonk.command.invite/time",
            "xyz.tonk.command.expel-member/member",
            "xyz.tonk.rename-repository/subject",
            "xyz.tonk.rename-repository/name",
        ] {
            assert!(
                core.contains(&format!("the: {attribute}")),
                "core.yaml no longer declares {attribute}"
            );
            assert!(
                key.contains(&attribute.to_owned()),
                "{attribute} is not a key attribute"
            );
        }
    }
}
