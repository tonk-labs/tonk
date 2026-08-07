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

use std::collections::VecDeque;
use std::sync::Arc;

use dialog_artifacts::{Changes, Entity};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::AppState;
use crate::reactor::{CommandRegistry, ScheduledHandler};

const INVOCATION_LEDGER_CAPACITY: usize = 256;

/// Asynchronous state of one scheduled native handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerState {
    /// The triggering transaction committed and work is queued.
    Scheduled,
    /// The handler completed successfully.
    Completed,
    /// The handler returned a structured failure.
    Failed,
}

/// Diagnostic outcome for one native handler registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerOutcome {
    /// Stable diagnostic handler name.
    pub handler: String,
    /// Current asynchronous state.
    pub state: HandlerState,
    /// Sanitized failure detail. Command arguments are never stored here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Bounded diagnostic record for one nominal invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRecord {
    /// Correlation identifier returned by `/transact`.
    pub correlation: String,
    /// Stable nominal command kind.
    pub command: Entity,
    /// Native handlers scheduled for this occurrence.
    pub handlers: Vec<HandlerOutcome>,
}

/// Cloneable FIFO ledger retaining the newest nominal invocation records.
#[derive(Clone, Default)]
pub struct InvocationLedger {
    records: Arc<RwLock<VecDeque<InvocationRecord>>>,
}

impl InvocationLedger {
    /// Insert a newly scheduled invocation, evicting the oldest at capacity.
    pub async fn insert(&self, record: InvocationRecord) {
        let mut records = self.records.write().await;
        if records.len() == INVOCATION_LEDGER_CAPACITY {
            records.pop_front();
        }
        records.push_back(record);
    }

    /// Resolve one record by correlation identifier.
    pub async fn get(&self, correlation: &str) -> Option<InvocationRecord> {
        self.records
            .read()
            .await
            .iter()
            .find(|record| record.correlation == correlation)
            .cloned()
    }

    async fn finish(
        &self,
        correlation: &str,
        handler_index: usize,
        result: Result<(), crate::reactor::CommandFailure>,
    ) {
        let mut records = self.records.write().await;
        let Some(record) = records
            .iter_mut()
            .find(|record| record.correlation == correlation)
        else {
            return;
        };
        let Some(handler) = record.handlers.get_mut(handler_index) else {
            return;
        };
        match result {
            Ok(()) => handler.state = HandlerState::Completed,
            Err(failure) => {
                handler.state = HandlerState::Failed;
                handler.message = Some(format!("{}: {}", failure.code, failure.message));
            }
        }
    }
}

/// Nominal native work prepared only after its triggering commit succeeded.
pub struct PendingInvocation {
    /// Correlation identifier shared with the response and ledger.
    pub correlation: String,
    /// Stable kind retained for diagnostics.
    pub command: Entity,
    /// Already decoded, post-commit native work.
    pub handlers: Vec<ScheduledHandler>,
}

/// `GET /api/invocations/{correlation}` diagnostic status lookup.
pub async fn invocation_status(
    ::axum::extract::State(state): ::axum::extract::State<AppState>,
    ::axum::extract::Path(correlation): ::axum::extract::Path<String>,
) -> Result<::axum::Json<InvocationRecord>, crate::TonkWorkerError> {
    let ledger = state.read().await.invocations.clone();
    ledger
        .get(&correlation)
        .await
        .map(::axum::Json)
        .ok_or_else(|| {
            crate::TonkWorkerError::NotFound(format!(
                "invocation correlation {correlation:?} is unknown or expired"
            ))
        })
}

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
/// registered against it would attach the remote to a brand-new spot rather
/// than the existing one the FAB names.
///
/// [`CreateSpaceHandler`]: super::repository::CreateSpaceHandler
/// [`RemoveSpaceHandler`]: super::repository::RemoveSpaceHandler
/// [`RenameRepositoryHandler`]: super::repository::RenameRepositoryHandler
/// [`EnableSyncHandler`]: super::repository::EnableSyncHandler
pub fn command_registry() -> CommandRegistry<CommandEnv> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use crate::reactor::CompatibilityNominalAdapter;

        let mut registry = CommandRegistry::new();
        let attribute = |value: &str| value.parse().expect("command attribute URI");

        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::CreateSpace>(
                "id:space/create".parse().expect("command kind"),
                "CreateSpaceHandler",
                Arc::new(super::repository::CreateSpaceHandler::new()),
            )
            .argument(
                "remote",
                attribute("dom.event.current-target.elements.remote/value"),
            )
            .argument(
                "revocation",
                attribute("dom.event.current-target.elements.revocation/value"),
            )
            .argument(
                "template",
                attribute("dom.event.current-target.elements.template/value"),
            ),
        ));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::CreateSpace>(
                "id:space/enable-sync".parse().expect("command kind"),
                "CreateSpaceHandler",
                Arc::new(super::repository::CreateSpaceHandler::new()),
            )
            .argument(
                "remote",
                attribute("dom.event.current-target.elements.remote/value"),
            ),
        ));
        registry.register_nominal(Box::new(CompatibilityNominalAdapter::for_concept::<
            tonk_schema::command::RemoveSpace,
        >(
            "id:space/remove".parse().expect("command kind"),
            "RemoveSpaceHandler",
            Arc::new(super::repository::RemoveSpaceHandler::new()),
        )));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::Invite>(
                "tonk:invite".parse().expect("command kind"),
                "InviteHandler",
                Arc::new(super::repository::InviteHandler::new()),
            )
            .argument("space", attribute("xyz.tonk.invite/space"))
            .constant(
                attribute("dom.event.current-target.dataset/invite"),
                dialog_artifacts::Value::Entity("tonk:invite".parse().expect("marker")),
            ),
        ));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::EnableSync>(
                "tonk:enable-sync".parse().expect("command kind"),
                "EnableSyncHandler",
                Arc::new(super::repository::EnableSyncHandler::new()),
            )
            .argument("space", attribute("xyz.tonk.enable-sync/space"))
            .argument("remote", attribute("xyz.tonk.enable-sync/remote"))
            .argument(
                "revocation",
                attribute("xyz.tonk.enable-sync/revocation-url"),
            )
            .argument("share", attribute("xyz.tonk.enable-sync/share"))
            .constant(
                attribute("dom.event.current-target.dataset/enable-sync"),
                dialog_artifacts::Value::Entity("tonk:enable-sync".parse().expect("marker")),
            ),
        ));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::PauseSync>(
                "tonk:pause-sync".parse().expect("command kind"),
                "PauseSyncHandler",
                Arc::new(super::repository::PauseSyncHandler::new()),
            )
            .constant(
                attribute("dom.event/time-stamp"),
                dialog_artifacts::Value::Float(0.0),
            )
            .constant(
                attribute("dom.event.current-target.dataset/pause-sync"),
                dialog_artifacts::Value::Entity("tonk:pause-sync".parse().expect("marker")),
            ),
        ));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::ProfileRename>(
                "id:profile/rename".parse().expect("command kind"),
                "ProfileRenameHandler",
                Arc::new(super::repository::ProfileRenameHandler::new()),
            )
            .constant(
                attribute("dom.event.current-target.dataset/rename"),
                dialog_artifacts::Value::Entity("tonk:profile".parse().expect("marker")),
            ),
        ));
        registry.register_nominal(Box::new(
            CompatibilityNominalAdapter::for_concept::<tonk_schema::command::RenameRepository>(
                "tonk:rename-repository".parse().expect("command kind"),
                "RenameRepositoryHandler",
                Arc::new(super::repository::RenameRepositoryHandler::new()),
            )
            .constant(
                attribute("dom.event.current-target.dataset/rename-repository"),
                dialog_artifacts::Value::Entity("tonk:repository".parse().expect("marker")),
            ),
        ));
        registry.register_nominal(Box::new(CompatibilityNominalAdapter::for_concept::<
            tonk_schema::command::Join,
        >(
            "tonk:join".parse().expect("command kind"),
            "JoinHandler",
            Arc::new(super::join::JoinHandler::new()),
        )));
        registry.register_nominal(Box::new(super::session::NominalLoadHandler::new()));
        registry.register_legacy(Box::new(super::repository::CreateSpaceHandler::new()));
        registry.register_legacy(Box::new(super::repository::RemoveSpaceHandler::new()));
        registry.register_legacy(Box::new(super::repository::InviteHandler::new()));
        registry.register_legacy(Box::new(super::repository::EnableSyncHandler::new()));
        registry.register_legacy(Box::new(super::repository::PauseSyncHandler::new()));
        registry.register_legacy(Box::new(super::repository::ProfileRenameHandler::new()));
        registry.register_legacy(Box::new(super::repository::RenameRepositoryHandler::new()));
        registry.register_legacy(Box::new(super::join::JoinHandler::new()));
        registry.register_legacy(Box::new(super::session::LoadHandler::new()));
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
/// the `TODO(stm)` notes on `reactor::command::LegacyTypedCommand::run`.
pub async fn dispatch(state: &AppState, origin: CommandOrigin, transients: Changes) {
    dispatch_with_nominal(state, origin, transients, Vec::new()).await;
}

/// Dispatch legacy structural work plus already-prepared nominal native work.
/// Nominal handlers are never derived from `transients`; the two lanes meet
/// only at this post-commit concurrency/drain boundary.
pub async fn dispatch_with_nominal(
    state: &AppState,
    origin: CommandOrigin,
    transients: Changes,
    nominal: Vec<PendingInvocation>,
) {
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
            tonk.commands
                .match_legacy_transients(&transients)
                .into_iter()
                .map(|(handler, facts)| handler.run(&facts, &env))
                .collect::<Vec<_>>()
        }
    };

    // Drive every command concurrently. `join_all` interleaves them so
    // independent effects make progress together rather than in sequence.
    let ledger = state.read().await.invocations.clone();
    let nominal_futures = nominal
        .into_iter()
        .flat_map(|invocation| {
            let correlation = invocation.correlation;
            invocation
                .handlers
                .into_iter()
                .enumerate()
                .map(|(handler_index, handler)| {
                    let ledger = ledger.clone();
                    let correlation = correlation.clone();
                    async move {
                        let result = handler.perform().await;
                        ledger.finish(&correlation, handler_index, result).await;
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    futures_util::future::join(
        futures_util::future::join_all(run_futures),
        futures_util::future::join_all(nominal_futures),
    )
    .await;

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
        let registry = command_registry().legacy::<Ping>();
        let changes = ping_transient("did:key:zPing", "hi");
        assert_eq!(
            registry.match_legacy_transients(&changes).len(),
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
            &'a dyn crate::reactor::LegacyCommandHandler<CommandEnv>,
            EntityFacts,
        ) {
            let mut fired = registry.match_legacy_transients(changes);
            assert_eq!(fired.len(), 1, "expected exactly one matched command");
            fired.pop().unwrap()
        }

        #[dialog_common::test]
        async fn it_runs_the_provider_with_the_decoded_command() {
            let _ = drain_ping_log();
            let (_state, env) = env().await;
            let registry = CommandRegistry::new().legacy::<Ping>();
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
            // `LegacyTypedCommand::run`'s own decode guard: handed an entity's
            // facts that don't decode as `Ping`, `run` is a no-op (the
            // provider is never called). We get a handler from a real
            // match, then run it against unrelated facts directly.
            let _ = drain_ping_log();
            let (_state, env) = env().await;
            let registry = CommandRegistry::new().legacy::<Ping>();
            let matched = registry.match_legacy_transients(&ping_transient("did:key:zP", "t"));
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
                tonk.commands = CommandRegistry::new().legacy::<Ping>();
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
                tonk.commands = CommandRegistry::new().legacy::<Ping>();
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

    fn invocation_record(correlation: impl Into<String>) -> InvocationRecord {
        InvocationRecord {
            correlation: correlation.into(),
            command: "id:test/command".parse().unwrap(),
            handlers: vec![HandlerOutcome {
                handler: "test-handler".into(),
                state: HandlerState::Scheduled,
                message: None,
            }],
        }
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    fn production_registry_has_every_nominal_native_kind() {
        let registry = command_registry();
        for kind in [
            "id:space/create",
            "id:space/enable-sync",
            "id:space/remove",
            "tonk:invite",
            "tonk:enable-sync",
            "tonk:pause-sync",
            "id:profile/rename",
            "tonk:rename-repository",
            "tonk:join",
            "tonk:load",
        ] {
            let kind = kind.parse().expect("stable command kind");
            assert_eq!(
                registry.registrations(&kind),
                1,
                "{kind} must have exactly one nominal native handler"
            );
        }
    }

    #[dialog_common::test]
    async fn invocation_ledger_tracks_sibling_completion_independently() {
        let ledger = InvocationLedger::default();
        let mut record = invocation_record("invoke:siblings");
        record.handlers.push(HandlerOutcome {
            handler: "second-handler".into(),
            state: HandlerState::Scheduled,
            message: None,
        });
        ledger.insert(record).await;

        ledger.finish("invoke:siblings", 0, Ok(())).await;
        ledger
            .finish(
                "invoke:siblings",
                1,
                Err(crate::reactor::CommandFailure {
                    code: "offline".into(),
                    message: "provider unavailable".into(),
                }),
            )
            .await;

        let record = ledger.get("invoke:siblings").await.unwrap();
        assert_eq!(record.handlers[0].state, HandlerState::Completed);
        assert_eq!(record.handlers[0].message, None);
        assert_eq!(record.handlers[1].state, HandlerState::Failed);
        assert_eq!(
            record.handlers[1].message.as_deref(),
            Some("offline: provider unavailable")
        );
    }

    #[dialog_common::test]
    async fn invocation_ledger_evicts_the_oldest_of_257_records() {
        let ledger = InvocationLedger::default();
        for index in 0..=INVOCATION_LEDGER_CAPACITY {
            ledger
                .insert(invocation_record(format!("invoke:{index:03}")))
                .await;
        }

        assert!(ledger.get("invoke:000").await.is_none());
        assert!(ledger.get("invoke:001").await.is_some());
        assert!(ledger.get("invoke:256").await.is_some());
    }
}
