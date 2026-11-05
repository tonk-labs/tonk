//! Command registration and post-commit dispatch.
//!
//! [`command_providers`] builds the two command vocabularies the worker
//! carries on [`TonkState`] — one for the profile branch, one for space
//! (content) branches — and [`dispatch`] runs the commands a
//! freshly-committed transient batch triggers against the vocabulary its
//! origin selects.
//!
//! A command is a [`dialog_capability::Command`] run by a
//! [`Provider<C>`](dialog_capability::Provider) — there is no handler
//! function. The provider is [`CommandEnv`], a cheap handle over
//! [`AppState`]; registering a command requires `CommandEnv: Provider<C>`,
//! so capability is a compile-time gate. A command is self-contained: its
//! `execute` does its own IO and commits its own outcomes through the env.
//!
//! [`TonkState`]: crate::worker::TonkState

use dialog_artifacts::{Changes, Entity};
use tonk_common::log;

use super::AppState;
use crate::reactor::CommandRegistry;

/// The environment commands run against — a cheap handle (clone of
/// [`AppState`]) that implements
/// [`Provider<C>`](dialog_capability::Provider) for each command `C` the
/// worker supports. The dispatcher hands a clone to the matched command;
/// `execute` does the work, reaching the operator/reactor by re-locking
/// through the `AppState`.
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
    task: super::invocation::Task,
}

/// The repository + branch a command was triggered in. Captured at the
/// dispatch site (the transact handler holds the committing
/// `BranchReference`, which knows both names) and carried on the env so
/// a handler can act on "the branch I fired in" without the command
/// re-carrying that context as a field.
#[derive(Clone, Debug, Default)]
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

impl CommandEnv {
    /// Build the env over a clone of the shared state, scoped to the
    /// `origin` the triggering commit happened in.
    pub fn new(state: AppState, origin: CommandOrigin) -> Self {
        Self {
            state,
            origin,
            task: Default::default(),
        }
    }

    /// The same env, reporting through `task`: what [`dispatch`] hands a
    /// command whose invoker asked to track it.
    pub fn with_task(mut self, task: super::invocation::Task) -> Self {
        self.task = task;
        self
    }

    /// How a provider reports how its command is going, for the person
    /// who ran it. Does nothing for a command nobody tracks, so a provider
    /// can always report. See [`super::invocation`].
    pub fn task(&self) -> &super::invocation::Task {
        &self.task
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

/// The worker's two command vocabularies, selected per dispatch by the
/// triggering commit's [`CommandOrigin`].
///
/// This is the registry-level expression of the containment rule
/// [`CommandEnv::may_target_space`] enforces per provider: WHERE a
/// transient committed decides WHAT it can mean. The profile branch —
/// the Hub/FAB evaluation surface, the one the user actually drives —
/// carries the full vocabulary. A space (content) branch carries only
/// the commands that are genuinely that space's to run; everything else
/// (space lifecycle, joins, account/passkey ceremonies, member
/// promotion) simply does not exist there, so a same-shaped fact
/// committed on a joined space's branch matches nothing instead of
/// relying on each provider to notice and refuse.
pub struct CommandProviders {
    /// The full vocabulary — every command the worker supports.
    profile: CommandRegistry<CommandEnv>,
    /// What a space branch may run on itself: loading a site
    /// ([`Load`](tonk_schema::command::Load), whose target IS the
    /// origin), renaming itself
    /// ([`RenameRepository`](tonk_schema::command::RenameRepository),
    /// which names its space and is refused cross-space by
    /// `may_target_space`), requesting an invite for itself (the space
    /// view's share surface dispatches on the space branch and its
    /// refusal publishes there), and — until membership moves fully
    /// profile-side —
    /// [`ExpelMember`](tonk_schema::command::ExpelMember), whose target
    /// is likewise the origin space, and installing a library component
    /// ([`InstallComponent`](tonk_schema::command::InstallComponent)) into
    /// the origin space, which the profile has none of.
    space: CommandRegistry<CommandEnv>,
}

impl CommandProviders {
    /// Build the pair explicitly. Production uses
    /// [`command_providers`]; tests use this to install doubles.
    pub fn new(profile: CommandRegistry<CommandEnv>, space: CommandRegistry<CommandEnv>) -> Self {
        Self { profile, space }
    }

    /// The vocabulary for a commit that landed at `origin`: the full
    /// set for the profile branch, the space set for a content branch.
    pub fn select(&self, origin: &CommandOrigin) -> &CommandRegistry<CommandEnv> {
        if origin.repo.is_empty() {
            &self.profile
        } else {
            &self.space
        }
    }
}

impl Default for CommandProviders {
    fn default() -> Self {
        Self::new(CommandRegistry::new(), CommandRegistry::new())
    }
}

/// Build both command vocabularies. Registration is just the type — the
/// behaviour is the `Provider<C>` impl on [`CommandEnv`].
///
/// The same vocabularies build for EVERY target — the browser, the CLI,
/// a TUI, a test. A command whose effect needs a page (a passkey
/// ceremony, a redirect) still registers everywhere; its provider
/// refuses visibly on a host without one rather than silently not
/// existing there. Where a chain genuinely needs a browser API, the
/// `cfg` sits on that leaf (`delete_space_storage_for`,
/// `worker_origin`, the `navigate` client messaging), never on a
/// registration.
pub fn command_providers() -> CommandProviders {
    CommandProviders::new(profile_commands(), space_commands())
}

/// The profile branch's vocabulary: everything.
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
/// [`CreateSpaceRequest`]: super::repository::CreateSpaceRequest
/// [`InviteRequest`]: super::repository::InviteRequest
/// [`EnableSyncRequest`]: super::repository::EnableSyncRequest
fn profile_commands() -> CommandRegistry<CommandEnv> {
    CommandRegistry::new()
        .command::<super::repository::CreateSpaceRequest>()
        .command::<super::repository::InviteRequest>()
        .command::<super::repository::AgentHandoffRequest>()
        .command::<super::repository::EnableSyncRequest>()
        .command::<tonk_schema::command::Load>()
        .command::<tonk_schema::command::PromoteMember>()
        .command::<tonk_schema::command::PublishSpace>()
        .command::<tonk_schema::command::UnpublishSpace>()
        .command::<tonk_schema::command::EnrollCustomer>()
        .command::<tonk_schema::command::ResendActivation>()
        .command::<tonk_schema::command::DeleteAccount>()
        .command::<tonk_schema::command::SendEmailInvitation>()
        .command::<tonk_schema::command::ClaimEmailInvitation>()
        // The plaintext shapes of the commands whose secret is now sealed.
        // Each runs the same work as its sealed sibling; an invocation
        // copies none of its secret (`secret::PLAINTEXT_LEGACY`). Retire
        // once no page sends them.
        .command::<tonk_schema::command::legacy::SendEmailInvitation>()
        .command::<tonk_schema::command::legacy::ClaimEmailInvitation>()
        .command::<tonk_schema::command::legacy::FinishSignInVia>()
        .migrated::<tonk_schema::command::legacy::JoinUrl, tonk_schema::command::legacy::Join>()
        .command::<super::ceremony::AuthorizeDeviceRequest>()
        // Finishing a sign-in has a plaintext shape (registered above)
        // from before its url was sealed; starting one carries no secret.
        .command::<tonk_schema::command::SignInVia>()
        .command::<tonk_schema::command::FinishSignInVia>()
        .migrated::<tonk_schema::command::AddPasskey, tonk_schema::command::legacy::AddPasskey>()
        .migrated::<tonk_schema::command::ExpelMember, tonk_schema::command::legacy::ExpelMember>()
        .migrated::<tonk_schema::command::RemoveSpace, tonk_schema::command::legacy::RemoveSpace>()
        .command::<tonk_schema::command::Join>()
        .migrated::<tonk_schema::command::CheckEmail, tonk_schema::command::legacy::CheckEmail>()
        .migrated::<tonk_schema::command::RegisterAccount, tonk_schema::command::legacy::RegisterAccount>()
        .migrated::<tonk_schema::command::PauseSync, tonk_schema::command::legacy::PauseSync>()
        .migrated::<tonk_schema::command::ProfileRename, tonk_schema::command::legacy::ProfileRename>()
        .migrated::<tonk_schema::command::RenameRepository, tonk_schema::command::legacy::RenameRepository>()
        // Replication and update checks are the Hub's to ask for: it is
        // the surface that lists spaces this device may not hold, and
        // `ForgetInvite` clears a row that lives on this branch anyway.
        // Switching profiles is new, so it has no legacy shape to migrate.
        .command::<tonk_schema::command::AddProfile>()
        .command::<tonk_schema::command::SwitchProfile>()
        .command::<tonk_schema::command::SignOut>()
        .command::<tonk_schema::command::Home>()
        // The bar's own acts: each records a request on the asking tab's
        // site, which that tab's bar performs (see `site_request`).
        .command::<tonk_schema::command::AddAccount>()
        .command::<tonk_schema::command::ShareLink>()
        .command::<tonk_schema::command::ViewMembers>()
        .command::<tonk_schema::command::ConnectAgent>()
        .command::<tonk_schema::command::ConnectSpace>()
        // Opens the introspection overlay; the same site-request shape,
        // answered by the overlay rather than the bar.
        .command::<tonk_schema::command::Inspect>()
        // What the page in a tab has selected, recorded on its site.
        .command::<tonk_schema::command::SiteSelect>()
        // What was typed in the command palette, interpreted.
        .command::<tonk_schema::command::Interpret>()
        .command::<tonk_schema::command::ReplicateSpace>()
        .command::<tonk_schema::command::ForgetInvite>()
        .command::<tonk_schema::command::CheckUpdate>()
}

/// A space branch's vocabulary — see [`CommandProviders`] for why each
/// of the three is here and nothing else is.
fn space_commands() -> CommandRegistry<CommandEnv> {
    CommandRegistry::new()
        .command::<tonk_schema::command::Load>()
        // What the space's page has selected, recorded on the tab's site.
        .command::<tonk_schema::command::SiteSelect>()
        // What was typed in the command palette, against this space's commands.
        .command::<tonk_schema::command::Interpret>()
        // A space may request an invite FOR ITSELF: the space view's
        // blank-canvas share (and the seeded `tonk:invite` descriptor)
        // dispatches on the space's own branch, and the refusal flow
        // ("sharing unavailable") publishes to that same branch's
        // overlay. `may_target_space` keeps it self-scoped — a space
        // cannot mint for another space. Profile-side is still the
        // destination once that surface moves.
        .command::<super::repository::InviteRequest>()
        // A space may ask whether ITS OWN seed is behind: the check
        // targets the origin, and `may_target_space` keeps it there.
        // Replication is deliberately absent — a space cannot ask to be
        // pulled onto a device that does not have it yet, because the
        // request would have to arrive on the branch it is asking for.
        .command::<tonk_schema::command::CheckUpdate>()
        .command::<super::repository::AgentHandoffRequest>()
        // A space installs a library component into itself; the profile
        // has no components to install.
        .command::<tonk_schema::command::InstallComponent>()
        .migrated::<tonk_schema::command::ExpelMember, tonk_schema::command::legacy::ExpelMember>()
        .migrated::<tonk_schema::command::RenameRepository, tonk_schema::command::legacy::RenameRepository>()
}

/// Run every command the just-committed `transients` triggered.
///
/// [`dispatch_committed`] for a commit whose rules concluded nothing, or
/// whose conclusions the caller does not have.
pub async fn dispatch(state: &AppState, origin: CommandOrigin, transients: Changes) {
    dispatch_committed(state, origin, transients, Changes::new()).await;
}

/// Run every command the just-committed `transients` triggered, keeping an
/// invocation for each one its invoker asked to track.
///
/// Called by a mutation path (e.g. `/transact`) after its commit, with
/// `AppState` in hand. The transients have already been swept from
/// durable storage by the commit; we matched them from the pre-commit
/// buffer, so the trigger fired exactly once. `concluded` is what the
/// commit's rules concluded: a rule that handled a command says so with a
/// `command/completion` there.
///
/// Each command's `Provider::execute` runs *concurrently and
/// independently* — they don't block one another, and a slow or failing
/// one doesn't hold up the rest. (Concurrent, not parallel: they share
/// the single SW task, interleaving at await points.) A command is
/// self-contained: it does its own IO and commits through the
/// [`CommandEnv`], so there's no outcome buffer to commit here. What each
/// one returns is its [`Outcome`](tonk_schema::invocation::Outcome): for a
/// tracked command it closes the invocation (see [`super::invocation`]),
/// and for any other a failure is logged.
///
/// TODO(stm): commands have no transactional isolation. A command reads
/// durable state through the env, decides, and commits — but between the
/// read and the commit another commit may have changed what it read, and
/// concurrent commands in the same batch can both read and write the same
/// state. The goal is STM-like optimistic concurrency: track the observed
/// revision/read-set and commit-or-conflict, re-running on conflict. See
/// the `TODO(stm)` notes on `reactor::command::TypedCommand::run`.
pub async fn dispatch_committed(
    state: &AppState,
    origin: CommandOrigin,
    transients: Changes,
    concluded: Changes,
) {
    use super::invocation::{self, Reporter, Retention, Task};

    let invoked = invocation::invoked(&transients);
    let reporter = (!invoked.is_empty()).then(|| Reporter::new(state.clone(), origin.clone()));
    let started = super::invocation::now();
    // Match commands and build their `'static` run-futures while holding
    // the read lock — each future owns its decoded command and an env
    // clone, so we can drop the lock before awaiting them. That keeps
    // command IO from ever running under a held lock (a command re-locks
    // through its env).
    let (tracked, untracked, retention, taken, refused) = {
        let tonk = state.read().await;
        // A command whose sealed field does not open never runs: plain
        // text where a secret belongs, a blob for an earlier session, or
        // one altered or moved from another field.
        let refused = unopened(&tonk, &transients).await;
        let retention = if invoked.is_empty() {
            Retention::default()
        } else {
            // Invocations whose own timers never fired (a worker that slept
            // through them) go before new ones are added.
            invocation::sweep(&tonk, &origin, started).await;
            Retention::read(&tonk).await
        };
        let registry = tonk.commands.select(&origin);
        let mut tracked: std::collections::BTreeMap<Entity, Vec<crate::reactor::RunFuture>> =
            invoked
                .keys()
                .map(|entity| (entity.clone(), Vec::new()))
                .collect();
        let mut untracked = Vec::new();
        // The current-shape attributes of the handlers that took each
        // tracked command: what its invocation may copy.
        let mut taken: std::collections::BTreeMap<Entity, std::collections::BTreeSet<String>> =
            Default::default();
        for entity in refused.keys() {
            tracked.remove(entity);
        }
        if registry.is_empty() {
            // No command providers for this origin — but the triggering
            // transact already committed and scheduled a poll, so still
            // drain below.
        } else {
            let fired = registry.match_transients(&transients);
            // The one place a command that decodes as nothing can be
            // seen: the transient committed, so the page believes it
            // asked, and nothing else says which attributes reached
            // the registry. On a space origin this is also where a
            // profile-only shape (a join, a create, a ceremony)
            // surfaces as contained rather than run.
            if fired.is_empty() {
                let attributes: std::collections::BTreeSet<String> = transients
                    .clone()
                    .into_instructions()
                    .into_iter()
                    .map(|instruction| match instruction {
                        dialog_artifacts::Instruction::Assert(artifact, _)
                        | dialog_artifacts::Instruction::Retract(artifact) => {
                            artifact.the.to_string()
                        }
                    })
                    .collect();
                if !attributes.is_empty() {
                    let surface = if origin.repo.is_empty() {
                        "profile".to_string()
                    } else {
                        format!("space '{}'", origin.repo)
                    };
                    log!(
                        "commands: no handler in the {surface} vocabulary matched a \
                         transient over {attributes:?}"
                    );
                }
            }
            let env = CommandEnv::new(state.clone(), origin.clone());
            for (handler, facts) in fired {
                let entity = facts.first().map(|artifact| artifact.of.clone());
                if entity
                    .as_ref()
                    .is_some_and(|entity| refused.contains_key(entity))
                {
                    continue;
                }
                match (entity, &reporter) {
                    (Some(entity), Some(reporter)) if tracked.contains_key(&entity) => {
                        let task = Task::tracked(entity.clone(), reporter.clone());
                        let run = handler.run(&facts, &env.clone().with_task(task));
                        taken
                            .entry(entity.clone())
                            .or_default()
                            .extend(handler.fields().iter().cloned());
                        tracked.entry(entity).or_default().push(run);
                    }
                    _ => untracked.push(handler.run(&facts, &env)),
                }
            }
        }
        (tracked, untracked, retention, taken, refused)
    };

    // A refused command reports why, where its invoker follows it, and is
    // logged either way.
    for (entity, error) in &refused {
        let failure = super::secret::refusal(error);
        log!("commands: refused {entity}: {failure}");
        if let (Some(reporter), Some(fields)) = (&reporter, invoked.get(entity)) {
            reporter.report(invocation::refused(
                entity, fields, &failure, started, &retention,
            ));
            reporter.expire_after(retention.of(tonk_schema::invocation::case::FAILED));
        }
    }

    // Open an invocation for each tracked command a provider took, and
    // settle the rest now: a rule handled it in the commit, or nothing did.
    if let Some(reporter) = &reporter {
        for (entity, runs) in &tracked {
            let fields = &invoked[entity];
            if runs.is_empty() {
                let fields = &invocation::copyable_unrun(fields);
                let completed = invocation::completion(&concluded, entity);
                let case = if completed.is_some() {
                    tonk_schema::invocation::case::DONE
                } else {
                    tonk_schema::invocation::case::UNHANDLED
                };
                reporter.report(invocation::settled(
                    entity, fields, completed, started, &retention,
                ));
                reporter.expire_after(retention.of(case));
            } else {
                let current = taken.get(entity).cloned().unwrap_or_default();
                let fields = invocation::copyable(fields, &current);
                reporter.report(invocation::opened(entity, &fields, started));
            }
        }
    }

    // Drive every command concurrently. `join_all` interleaves them so
    // independent effects make progress together rather than in sequence.
    let tracked_runs = tracked
        .into_iter()
        .filter(|(_, runs)| !runs.is_empty())
        .map(|(entity, runs)| {
            let reporter = reporter.clone();
            async move {
                let outcome = combine(futures_util::future::join_all(runs).await);
                if let Some(reporter) = reporter {
                    let case = if outcome.is_ok() {
                        tonk_schema::invocation::case::DONE
                    } else {
                        tonk_schema::invocation::case::FAILED
                    };
                    let finished = super::invocation::now();
                    reporter.report(invocation::closed(&entity, &outcome, finished, &retention));
                    reporter.expire_after(retention.of(case));
                }
            }
        });
    let untracked_runs = untracked.into_iter().map(|run| async move {
        if let Err(failure) = run.await {
            log!("commands: a command failed: {failure}");
        }
    });
    futures_util::future::join(
        futures_util::future::join_all(tracked_runs),
        futures_util::future::join_all(untracked_runs),
    )
    .await;

    // Everything the commands reported lands before the drain below, so a
    // caller that awaited the dispatch reads the invocations closed.
    if let Some(reporter) = &reporter {
        reporter.flush().await;
    }

    // Drain every poll the request scheduled — the triggering commit plus
    // anything its providers committed or wrote to the overlay — in one
    // pass. This is the single point that turns scheduled writes into
    // subscription broadcasts; coalesced by branch identity.
    let tonk = state.read().await;
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// The commands among `transients` whose sealed fields do not open with
/// this worker's key, each with why. Derives the key only when some
/// transient carries a sealed attribute at all.
async fn unopened(
    tonk: &crate::worker::TonkState,
    transients: &Changes,
) -> std::collections::BTreeMap<Entity, tonk_schema::secret::SecretError> {
    let mut sealed: std::collections::BTreeMap<Entity, Vec<(String, dialog_artifacts::Value)>> =
        Default::default();
    for (of, the, change) in transients.iter() {
        let relation = the.to_string();
        if tonk_schema::secret::is_sealed(&relation) {
            sealed
                .entry(of.clone())
                .or_default()
                .push((relation, change.value().clone()));
        }
    }
    if sealed.is_empty() {
        return Default::default();
    }
    let key = super::secret::key(tonk).await;
    sealed
        .into_iter()
        .filter_map(|(entity, facts)| {
            tonk_schema::secret::verify(
                key.as_ref(),
                facts
                    .iter()
                    .map(|(relation, value)| (relation.clone(), value)),
            )
            .err()
            .map(|error| (entity, error))
        })
        .collect()
}

/// What the providers that took one command came to, together: the first
/// failure, or else success with the first output any of them produced.
fn combine(outcomes: Vec<tonk_schema::invocation::Outcome>) -> tonk_schema::invocation::Outcome {
    let mut done = tonk_schema::invocation::Done::default();
    for outcome in outcomes {
        let finished = outcome?;
        if done.output.is_none() {
            done.output = finished.output;
        }
    }
    Ok(done)
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(missing_docs)]

    use super::*;
    use dialog_artifacts::Statement;
    use dialog_query::{Attribute, Concept, Entity, the};
    use std::sync::Mutex;

    // A test command whose provider RECORDS each invocation's tag, so a
    // test can observe whether (and with what) `execute` ran. The
    // provider does no IO, so `run`-level tests don't need a real
    // service-worker scope — only `dispatch` (which builds a real
    // `CommandEnv` from `AppState`) is browser-gated.
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
        type Output = tonk_schema::invocation::Outcome;
    }

    /// Tags passed to `Ping`'s provider, in invocation order. The `run`
    /// tests `drain` it; serialized within the single-threaded test run.
    static PING_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Take and clear the recorded tags. Only the wasm-gated `run` tests
    /// read it; the provider writes it on every target.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn drain_ping_log() -> Vec<String> {
        std::mem::take(&mut *PING_LOG.lock().unwrap())
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl dialog_capability::Provider<Ping> for CommandEnv {
        async fn execute(&self, command: Ping) -> tonk_schema::invocation::Outcome {
            PING_LOG.lock().unwrap().push(command.tag.0);
            tonk_schema::invocation::Done::nothing()
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

    #[dialog_common::test]
    fn it_registers_a_command_type_by_its_provider() {
        // `command::<Ping>()` compiles only because `CommandEnv:
        // Provider<Ping>` — the capability gate. The registered type
        // matches its trigger.
        let registry = profile_commands().command::<Ping>();
        let changes = ping_transient("did:key:zPing", "hi");
        assert_eq!(
            registry.match_transients(&changes).len(),
            1,
            "the registered Ping command should match its trigger"
        );
    }

    /// The vocabulary split itself: every transient here DECODES as its
    /// command (each shape matches at least once on the profile), so a
    /// zero-match on the space vocabulary means the command is absent
    /// there — not that the probe was malformed.
    #[dialog_common::test]
    fn a_space_origin_selects_a_vocabulary_without_profile_only_commands() {
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
                !providers
                    .select(&profile)
                    .match_transients(changes)
                    .is_empty(),
                "`{name}` must match on the profile — a zero here means the \
                 probe shape no longer decodes and this test proves nothing"
            );
            assert!(
                providers
                    .select(&space)
                    .match_transients(changes)
                    .is_empty(),
                "`{name}` must not exist in a space branch's vocabulary"
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
                    !providers
                        .select(origin)
                        .match_transients(changes)
                        .is_empty(),
                    "`{name}` must be dispatchable from the {surface}"
                );
            }
        }
    }

    // The `run` and `dispatch` tests build a real `CommandEnv` from an
    // `AppState`, which needs the service-worker scope (`test_state`).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    mod run {
        use super::*;
        use crate::reactor::{CommandRegistry, EntityFacts};
        use crate::router::AppState;
        use crate::router::tests::test_state;
        use std::sync::Arc;

        async fn env() -> (AppState, CommandEnv) {
            let state: AppState = Arc::new(crate::StateLock::new(test_state().await));
            let env = CommandEnv::new(state.clone(), CommandOrigin::default());
            (state, env)
        }

        fn one_match<'a>(
            registry: &'a CommandRegistry<CommandEnv>,
            changes: &Changes,
        ) -> (
            &'a dyn crate::reactor::CommandHandler<CommandEnv>,
            EntityFacts,
        ) {
            let mut fired = registry.match_transients(changes);
            assert_eq!(fired.len(), 1, "expected exactly one matched command");
            fired.pop().unwrap()
        }

        #[dialog_common::test]
        async fn it_runs_the_provider_with_the_decoded_command() {
            let _ = drain_ping_log();
            let (_state, env) = env().await;
            let registry = CommandRegistry::new().command::<Ping>();
            let changes = ping_transient("did:key:zPing", "hello");

            let (handler, facts) = one_match(&registry, &changes);
            handler.run(&facts, &env).await;

            assert_eq!(
                drain_ping_log(),
                vec!["hello".to_string()],
                "the provider should run once with the decoded tag"
            );
        }

        #[dialog_common::test]
        async fn it_runs_the_provider_for_a_non_decoding_entity_as_a_noop() {
            // `TypedCommand::run`'s own decode guard: handed an entity's
            // facts that don't decode as `Ping`, `run` is a no-op (the
            // provider is never called). We get a handler from a real
            // match, then run it against unrelated facts directly.
            let _ = drain_ping_log();
            let (_state, env) = env().await;
            let registry = CommandRegistry::new().command::<Ping>();
            let matched = registry.match_transients(&ping_transient("did:key:zP", "t"));
            let handler = matched[0].0;

            let unrelated: EntityFacts = {
                let mut changes = Changes::new();
                the!("xyz.tonk.unrelated/noise")
                    .of("did:key:zNoise".parse::<Entity>().unwrap())
                    .is("x".to_string())
                    .assert(&mut changes);
                // One entity → its facts.
                match changes.into_instructions().into_iter().next().unwrap() {
                    dialog_artifacts::Instruction::Assert(a, _)
                    | dialog_artifacts::Instruction::Retract(a) => vec![a],
                }
            };
            handler.run(&unrelated, &env).await;

            assert!(
                drain_ping_log().is_empty(),
                "a non-decoding entity must not run the provider"
            );
        }

        #[dialog_common::test]
        async fn it_dispatches_every_matched_command_in_a_batch() {
            let _ = drain_ping_log();
            let (state, _env) = env().await;
            // Install the registry on the state so `dispatch` sees it.
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands = CommandProviders::new(
                    CommandRegistry::new().command::<Ping>(),
                    CommandRegistry::new(),
                );
            }

            // Two distinct Ping entities in one batch → two invocations.
            let mut changes = ping_transient("did:key:zA", "alpha");
            the!("xyz.tonk.command/ping-tag")
                .of("did:key:zB".parse::<Entity>().unwrap())
                .is("beta".to_string())
                .assert(&mut changes);

            dispatch(&state, CommandOrigin::default(), changes).await;

            let mut tags = drain_ping_log();
            tags.sort();
            assert_eq!(
                tags,
                vec!["alpha".to_string(), "beta".to_string()],
                "dispatch runs each matched command's provider"
            );
        }

        #[dialog_common::test]
        async fn it_dispatches_nothing_when_no_command_matches() {
            let _ = drain_ping_log();
            let (state, _env) = env().await;
            {
                let mut tonk = state.write().await;
                tonk.shared_mut().commands = CommandProviders::new(
                    CommandRegistry::new().command::<Ping>(),
                    CommandRegistry::new(),
                );
            }
            let mut changes = Changes::new();
            the!("xyz.tonk.unrelated/noise")
                .of("did:key:zNoise".parse::<Entity>().unwrap())
                .is("x".to_string())
                .assert(&mut changes);

            dispatch(&state, CommandOrigin::default(), changes).await;
            assert!(drain_ping_log().is_empty());
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
            test_state_with(crate::router::command_providers()).await
        }

        /// [`test_state`] running `commands` instead of the real
        /// vocabularies, for a test that needs a provider of its own.
        pub(crate) async fn test_state_with(commands: CommandProviders) -> AppState {
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
                    commands,
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
            dispatch(
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
            dispatch(&state, foreign, remove_space_transient(&subject)).await;
            assert_eq!(
                space_subjects(&state).await.len(),
                1,
                "a content-branch origin must not remove a space by DID"
            );

            // From the profile origin the same transient removes it.
            dispatch(
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
            dispatch(
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
            dispatch(
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
            dispatch(&state, CommandOrigin::default(), copy_command()).await;
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
            dispatch(&state, CommandOrigin::default(), invalid).await;
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
                dispatch(&state, CommandOrigin::default(), changes).await;
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
            dispatch(
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
                dispatch(
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

        /// A test command whose provider reports a progress state, then
        /// succeeds producing `id:produced` or, for the tag `fail`, fails.
        #[derive(dialog_query::Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[domain("xyz.tonk.command")]
        pub struct ReportTag(pub String);

        #[derive(dialog_query::Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
        pub struct Report {
            pub this: Entity,
            pub tag: ReportTag,
        }

        impl dialog_capability::Command for Report {
            type Input = Self;
            type Output = tonk_schema::invocation::Outcome;
        }

        #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
        impl dialog_capability::Provider<Report> for CommandEnv {
            async fn execute(&self, command: Report) -> tonk_schema::invocation::Outcome {
                self.task()
                    .status("status:reporting".parse().unwrap(), "Reporting");
                if command.tag.0 == "fail" {
                    return Err("the report was refused".into());
                }
                tonk_schema::invocation::Done::with("id:produced".parse().unwrap())
            }
        }

        async fn reporting_state() -> AppState {
            test_state_with(CommandProviders::new(
                profile_commands().command::<Report>(),
                space_commands(),
            ))
            .await
        }

        /// A `Report` transient on `of`, stamped as the palette stamps
        /// what it runs when `invoked` is set.
        fn report(of: &str, tag: &str, invoked: bool) -> Changes {
            let mut changes = Changes::new();
            the!("xyz.tonk.command/report-tag")
                .of(of.parse::<Entity>().unwrap())
                .is(tag.to_string())
                .assert(&mut changes);
            if invoked {
                stamp(&mut changes, of);
            }
            changes
        }

        fn stamp(changes: &mut Changes, of: &str) {
            use tonk_schema::invocation::command::{Concept, InvokedAt};
            let of: Entity = of.parse().unwrap();
            InvokedAt::of(of.clone()).is(InvokedAt(1.0)).assert(changes);
            Concept::of(of)
                .is(Concept("tonk:test/report".parse().unwrap()))
                .assert(changes);
        }

        /// The profile branch's session overlay.
        async fn overlay(state: &AppState) -> Changes {
            let tonk = state.read().await;
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .overlay()
                .export()
        }

        /// What the overlay holds for `of`'s `the`.
        fn held(overlay: &Changes, of: &str, the: impl ToString) -> Vec<dialog_artifacts::Value> {
            let of: Entity = of.parse().unwrap();
            let the = the.to_string();
            overlay
                .iter()
                .filter(|(entity, relation, change)| {
                    **entity == of
                        && relation.to_string() == the
                        && matches!(change, dialog_artifacts::Change::Assert(..))
                })
                .map(|(_, _, change)| change.value().clone())
                .collect()
        }

        fn entity_value(uri: &str) -> dialog_artifacts::Value {
            dialog_artifacts::Value::Entity(uri.parse().unwrap())
        }

        fn float(held: &[dialog_artifacts::Value]) -> f64 {
            match held {
                [dialog_artifacts::Value::Float(value)] => *value,
                other => panic!("expected one float, got {other:?}"),
            }
        }

        #[dialog_common::test]
        async fn it_keeps_an_invocation_for_a_tracked_command_that_succeeds() {
            use tonk_schema::invocation::{
                Case, Expires, Finished, Output, Started, State, Status, case,
            };
            let state = reporting_state().await;
            dispatch(
                &state,
                CommandOrigin::default(),
                report("id:run", "ok", true),
            )
            .await;

            let overlay = overlay(&state).await;
            assert_eq!(
                held(&overlay, "id:run", Case::the()),
                vec![entity_value(case::DONE)]
            );
            assert_eq!(
                held(&overlay, "id:run", Output::the()),
                vec![entity_value("id:produced")],
                "the output is what the provider produced"
            );
            assert_eq!(
                held(&overlay, "id:run", "xyz.tonk.command/report-tag"),
                vec![dialog_artifacts::Value::String("ok".into())],
                "the invocation carries the command's own fields"
            );
            assert_eq!(
                held(&overlay, "id:run", State::the()),
                vec![entity_value("status:reporting")],
                "what the provider reported is the invocation's state"
            );
            assert_eq!(
                held(&overlay, "status:reporting", Status::the()),
                vec![dialog_artifacts::Value::String("Reporting".into())],
                "and the state says what it is"
            );
            let started = float(&held(&overlay, "id:run", Started::the()));
            let finished = float(&held(&overlay, "id:run", Finished::the()));
            let expires = float(&held(&overlay, "id:run", Expires::the()));
            assert!(started <= finished, "{started} <= {finished}");
            assert_eq!(
                expires,
                finished + tonk_schema::invocation::retention::DONE_SECONDS * 1000.0,
                "a success is kept for the default retention"
            );
        }

        #[dialog_common::test]
        async fn it_records_why_a_tracked_command_failed() {
            use tonk_schema::invocation::{Case, Expires, Finished, Output, Status, case};
            let state = reporting_state().await;
            dispatch(
                &state,
                CommandOrigin::default(),
                report("id:refused", "fail", true),
            )
            .await;

            let overlay = overlay(&state).await;
            assert_eq!(
                held(&overlay, "id:refused", Case::the()),
                vec![entity_value(case::FAILED)]
            );
            let error = crate::router::invocation::error_entity(&"id:refused".parse().unwrap());
            assert_eq!(
                held(&overlay, "id:refused", Output::the()),
                vec![dialog_artifacts::Value::Entity(error.clone())],
                "a failure's output is its error"
            );
            assert_eq!(
                held(&overlay, &error.to_string(), Status::the()),
                vec![dialog_artifacts::Value::String(
                    "the report was refused".into()
                )],
                "and the error says why"
            );
            let finished = float(&held(&overlay, "id:refused", Finished::the()));
            assert_eq!(
                float(&held(&overlay, "id:refused", Expires::the())),
                finished + tonk_schema::invocation::retention::FAILED_SECONDS * 1000.0,
                "a failure is kept for longer"
            );
        }

        #[dialog_common::test]
        async fn it_keeps_no_invocation_for_a_command_nobody_tracks() {
            use tonk_schema::invocation::{Case, Started, State};
            let state = reporting_state().await;
            dispatch(
                &state,
                CommandOrigin::default(),
                report("id:quiet", "ok", false),
            )
            .await;

            let overlay = overlay(&state).await;
            assert!(held(&overlay, "id:quiet", Case::the()).is_empty());
            assert!(held(&overlay, "id:quiet", Started::the()).is_empty());
            assert!(
                held(&overlay, "id:quiet", State::the()).is_empty(),
                "a provider reporting on an untracked command writes nothing"
            );
            assert!(
                held(&overlay, "id:quiet", "xyz.tonk.command/report-tag").is_empty(),
                "nor is the command copied"
            );
        }

        #[dialog_common::test]
        async fn it_records_a_tracked_command_nothing_took_as_unhandled() {
            use tonk_schema::invocation::{Case, case};
            let state = reporting_state().await;
            let mut changes = Changes::new();
            the!("xyz.tonk.command.nobody/handles")
                .of("id:orphan".parse::<Entity>().unwrap())
                .is("x".to_string())
                .assert(&mut changes);
            stamp(&mut changes, "id:orphan");
            dispatch(&state, CommandOrigin::default(), changes).await;

            assert_eq!(
                held(&overlay(&state).await, "id:orphan", Case::the()),
                vec![entity_value(case::UNHANDLED)]
            );
        }

        #[dialog_common::test]
        async fn it_records_a_command_a_rule_completed_as_done() {
            use tonk_schema::invocation::{Case, Output, case, completion};
            let state = reporting_state().await;
            let mut changes = Changes::new();
            the!("xyz.tonk.notebook.retitle/title")
                .of("id:retitle".parse::<Entity>().unwrap())
                .is("Plans".to_string())
                .assert(&mut changes);
            stamp(&mut changes, "id:retitle");
            let mut concluded = Changes::new();
            completion::Output::of("id:retitle".parse::<Entity>().unwrap())
                .is(completion::Output("id:notebook".parse().unwrap()))
                .assert(&mut concluded);
            dispatch_committed(&state, CommandOrigin::default(), changes, concluded).await;

            let overlay = overlay(&state).await;
            assert_eq!(
                held(&overlay, "id:retitle", Case::the()),
                vec![entity_value(case::DONE)]
            );
            assert_eq!(
                held(&overlay, "id:retitle", Output::the()),
                vec![entity_value("id:notebook")],
                "the output is what the rule concluded"
            );
        }

        #[dialog_common::test]
        async fn a_tracked_dispatch_sweeps_what_has_expired() {
            use tonk_schema::invocation::{Case, Done};
            let state = reporting_state().await;
            {
                let tonk = state.read().await;
                let stale = crate::router::invocation::closed(
                    &"id:stale".parse().unwrap(),
                    &Done::nothing(),
                    0.0,
                    &crate::router::invocation::Retention::default(),
                );
                tonk.reactor
                    .profile_repository()
                    .branch(&tonk.active_branch)
                    .overlay()
                    .assert(stale)
                    .write()
                    .perform(&tonk.operator)
                    .await
                    .unwrap();
            }
            assert!(!held(&overlay(&state).await, "id:stale", Case::the()).is_empty());

            dispatch(
                &state,
                CommandOrigin::default(),
                report("id:next", "ok", true),
            )
            .await;

            let overlay = overlay(&state).await;
            assert!(
                held(&overlay, "id:stale", Case::the()).is_empty(),
                "an invocation that expired long ago is swept"
            );
            assert!(!held(&overlay, "id:next", Case::the()).is_empty());
        }

        /// A real provider: creating a space from the hub, tracked, ends
        /// done with the new space as its output, and says what it was
        /// doing on the way.
        #[dialog_common::test]
        async fn a_tracked_space_create_ends_done_with_the_new_space() {
            use tonk_schema::invocation::{Case, Output, State, case};
            let state = test_state().await;
            let mut changes = create_space_transient("Tracked");
            stamp(&mut changes, "cmd:create");
            dispatch(&state, CommandOrigin::default(), changes).await;

            let spaces = space_subjects(&state).await;
            assert_eq!(spaces.len(), 1);
            let overlay = overlay(&state).await;
            assert_eq!(
                held(&overlay, "cmd:create", Case::the()),
                vec![entity_value(case::DONE)]
            );
            assert_eq!(
                held(&overlay, "cmd:create", Output::the()),
                vec![dialog_artifacts::Value::Entity(
                    spaces[0].to_string().parse().unwrap()
                )]
            );
            assert_eq!(
                held(&overlay, "cmd:create", State::the()),
                vec![entity_value("status:creating-space")]
            );
        }
    }
}
