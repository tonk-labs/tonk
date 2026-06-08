//! Command registration and post-commit dispatch.
//!
//! [`command_registry`] builds the registry of typed-Rust command
//! handlers the worker carries on [`TonkState`]. [`dispatch`] runs the
//! handlers a freshly-committed transient batch triggers, committing
//! each handler's outcome back to the branch the command arrived on.
//!
//! The split honours the sandbox: a handler declares its capability in
//! its signature (axum-style `State<…>`), so a pure handler
//! (`async fn(C, Transaction) -> Transaction`) gets nothing, while one
//! that needs IO names `State<AppState>` and the dispatcher supplies it.
//! Either way the handler's only fact-write path is the returned
//! [`Transaction`](crate::reactor::CommandTx); the privileged commit
//! lives here, in the worker layer, and always targets the command's
//! own branch — so a handler can never "commit whatever whenever".
//!
//! [`TonkState`]: crate::worker::TonkState

use dialog_artifacts::Changes;

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
}

impl CommandEnv {
    /// Build the env over a clone of the shared state.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Borrow the underlying state — `Provider` impls re-lock through
    /// this to reach the operator and reactor.
    pub fn state(&self) -> &AppState {
        &self.state
    }
}

/// Build the registry of supported command *types*. Registration is just
/// the type — the behaviour is the `Provider<C>` impl on [`CommandEnv`].
///
/// `CreateSpace` is gated to wasm because its provider seeds from a
/// served asset that only exists in the service-worker scope. Native
/// builds get an empty registry (tests register their own).
pub fn command_registry() -> CommandRegistry {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        CommandRegistry::new().command::<tonk_schema::command::CreateSpace>()
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
pub async fn dispatch(state: &AppState, transients: Changes) {
    // Match commands and build their `'static` run-futures while holding
    // the read lock — each future owns its decoded command and an env
    // clone, so we can drop the lock before awaiting them. That keeps
    // command IO from ever running under a held lock (a command re-locks
    // through its env).
    let run_futures = {
        let tonk = state.read().await;
        if tonk.commands.is_empty() {
            return;
        }
        let env = CommandEnv::new(state.clone());
        tonk.commands
            .match_transients(&transients)
            .into_iter()
            .map(|(handler, facts)| handler.run(&facts, &env))
            .collect::<Vec<_>>()
    };

    // Drive every command concurrently. `join_all` interleaves them so
    // independent effects make progress together rather than in sequence.
    futures_util::future::join_all(run_futures).await;
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
            let env = CommandEnv::new(state.clone());
            (state, env)
        }

        fn one_match<'a>(
            registry: &'a CommandRegistry,
            changes: &Changes,
        ) -> (&'a dyn crate::reactor::CommandHandler, EntityFacts) {
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

            dispatch(&state, changes).await;

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

            dispatch(&state, changes).await;
            assert!(drain_ping_log().is_empty());
        }
    }
}
