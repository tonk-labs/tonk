//! Command registration and post-commit dispatch.
//!
//! [`command_registry`] builds the registry of supported command *types*
//! the worker carries on [`TonkState`]. [`dispatch`] runs the commands a
//! freshly-committed transient batch triggers.
//!
//! A command is a [`dialog_capability::Command`] run by a
//! [`Provider<C>`](dialog_capability::Provider) — there is no handler
//! function. The provider is [`CommandEnv`], a cheap handle over
//! [`AppState`]; registering a command requires `CommandEnv: Provider<C>`,
//! so capability is a compile-time gate. A command is self-contained: its
//! `execute` does its own IO and commits its own outcomes through the env.
//!
//! [`TonkState`]: crate::worker::TonkState

use dialog_artifacts::Changes;
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
}

/// Build the registry of supported command *types*. Registration is just
/// the type — the behaviour is the `Provider<C>` impl on [`CommandEnv`].
///
/// Gated to wasm because the handler does service-worker-scoped IO
/// (seeding from a served asset, opening a remote branch over the
/// network). Native builds get an empty registry (tests register their
/// own).
///
/// One custom [`CreateSpaceHandler`] serves both the Hub "New space"
/// (`space/create`) and topbar "Enable sync" (`space/enable-sync`) forms:
/// both post the same `name`(+`remote`) shape, the handler keys on the
/// shared `name` attribute, and it reads the optional remote from the
/// transient's facts — which a typed `Provider`, receiving only the
/// decoded command, can't do.
///
/// [`RenameRepositoryHandler`] serves the FAB's repository-name chip
/// (`tonk/rename-repository`): a profile-branch command carrying its
/// target `space`, since a claim dispatched from the profile branch has
/// no space-side rule to consume it (see
/// [`tonk_schema::command::RenameRepository`]).
///
/// [`RemoveSpaceHandler`] serves the Hub's per-row delete confirm
/// (`space/remove`): replica retraction, reactor eviction, storage
/// cleanup.
///
/// [`EnableSyncHandler`] is deliberately its own command
/// ([`tonk_schema::command::EnableSync`]), not a second handler on
/// `space/enable-sync`: that trigger attribute belongs to `CreateSpace`, and
/// `CreateSpaceHandler` always mints a fresh identity first, so anything
/// registered against it would attach the remote to a brand-new space rather
/// than the existing one the FAB names.
///
/// [`CreateSpaceHandler`]: super::repository::CreateSpaceHandler
/// [`RemoveSpaceHandler`]: super::repository::RemoveSpaceHandler
/// [`RenameRepositoryHandler`]: super::repository::RenameRepositoryHandler
/// [`EnableSyncHandler`]: super::repository::EnableSyncHandler
pub fn command_registry() -> CommandRegistry<CommandEnv> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let mut registry = CommandRegistry::new();
        registry.register(Box::new(super::repository::CreateSpaceHandler::new()));
        registry.register(Box::new(super::repository::CreateNotebookHandler::new()));
        registry.register(Box::new(super::repository::RemoveSpaceHandler::new()));
        registry.register(Box::new(super::repository::InviteHandler::new()));
        registry.register(Box::new(super::repository::EnableSyncHandler::new()));
        registry.register(Box::new(super::repository::PauseSyncHandler::new()));
        registry.register(Box::new(super::repository::ProfileRenameHandler::new()));
        registry.register(Box::new(super::repository::RenameRepositoryHandler::new()));
        registry.register(Box::new(super::members::PromoteMemberHandler::new()));
        registry.register(Box::new(super::members::ExpelMemberHandler::new()));
        registry.register(Box::new(super::join::JoinHandler::new()));
        registry.register(Box::new(super::email_status::CheckEmailHandler::new()));
        registry.register(Box::new(super::email_status::RegisterAccountHandler::new()));
        registry.register(Box::new(super::customer::EnrollCustomerHandler::new()));
        registry.register(Box::new(super::customer::ResendActivationHandler::new()));
        registry.register(Box::new(super::session::LoadHandler::new()));
        registry.register(Box::new(
            super::account_deletion::DeleteAccountHandler::new(),
        ));
        registry.register(Box::new(super::ceremony::AuthorizeDeviceHandler::new()));
        registry.register(Box::new(super::ceremony::AddPasskeyHandler::new()));
        registry
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        CommandRegistry::new()
    }
}

/// Run every command the just-committed `transients` triggered.
///
/// Called by a mutation path (e.g. `/transact`) after its commit, with
/// `AppState` in hand. The transients have already been swept from
/// durable storage by the commit; we matched them from the pre-commit
/// buffer, so the trigger fired exactly once.
///
/// Each command's `Provider::execute` runs *concurrently and
/// independently* — they don't block one another, and a slow or failing
/// one doesn't hold up the rest. (Concurrent, not parallel: they share
/// the single SW task, interleaving at await points.) A command is
/// self-contained: it does its own IO and commits through the
/// [`CommandEnv`], so there's no outcome buffer to commit here.
///
/// TODO(stm): commands have no transactional isolation. A command reads
/// durable state through the env, decides, and commits — but between the
/// read and the commit another commit may have changed what it read, and
/// concurrent commands in the same batch can both read and write the same
/// state. The goal is STM-like optimistic concurrency: track the observed
/// revision/read-set and commit-or-conflict, re-running on conflict. See
/// the `TODO(stm)` notes on `reactor::command::TypedCommand::run`.
pub async fn dispatch(state: &AppState, origin: CommandOrigin, transients: Changes) {
    // Match commands and build their `'static` run-futures while holding
    // the read lock — each future owns its decoded command and an env
    // clone, so we can drop the lock before awaiting them. That keeps
    // command IO from ever running under a held lock (a command re-locks
    // through its env).
    let run_futures = {
        let tonk = state.read().await;
        if tonk.commands.is_empty() {
            // No command providers — but the triggering transact already
            // committed and scheduled a poll, so still drain below.
            Vec::new()
        } else {
            let env = CommandEnv::new(state.clone(), origin);
            let fired = tonk.commands.match_transients(&transients);
            // The one place a command that decodes as nothing can be
            // seen: the transient committed, so the page believes it
            // asked, and nothing else says which attributes reached
            // the registry.
            if fired.is_empty() {
                let attributes: std::collections::BTreeSet<String> = transients
                    .clone()
                    .into_instructions()
                    .into_iter()
                    .map(|instruction| match instruction {
                        dialog_artifacts::Instruction::Assert(artifact)
                        | dialog_artifacts::Instruction::Replace(artifact)
                        | dialog_artifacts::Instruction::Retract(artifact) => {
                            artifact.the.to_string()
                        }
                    })
                    .collect();
                if !attributes.is_empty() {
                    log!("commands: no handler matched a transient over {attributes:?}");
                }
            }
            fired
                .into_iter()
                .map(|(handler, facts)| handler.run(&facts, &env))
                .collect::<Vec<_>>()
        }
    };

    // Drive every command concurrently. `join_all` interleaves them so
    // independent effects make progress together rather than in sequence.
    futures_util::future::join_all(run_futures).await;

    // Drain every poll the request scheduled — the triggering commit plus
    // anything its providers committed or wrote to the overlay — in one
    // pass. This is the single point that turns scheduled writes into
    // subscription broadcasts; coalesced by branch identity.
    let tonk = state.read().await;
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

#[cfg(test)]
mod tests {
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
        type Output = ();
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
        async fn execute(&self, command: Ping) {
            PING_LOG.lock().unwrap().push(command.tag.0);
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
        let registry = command_registry().command::<Ping>();
        let changes = ping_transient("did:key:zPing", "hi");
        assert_eq!(
            registry.match_transients(&changes).len(),
            1,
            "the registered Ping command should match its trigger"
        );
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
        use tokio::sync::RwLock;

        async fn env() -> (AppState, CommandEnv) {
            let state: AppState = Arc::new(RwLock::new(test_state().await));
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
                    dialog_artifacts::Instruction::Assert(a)
                    | dialog_artifacts::Instruction::Replace(a)
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
                tonk.commands = CommandRegistry::new().command::<Ping>();
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
                tonk.commands = CommandRegistry::new().command::<Ping>();
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
}
