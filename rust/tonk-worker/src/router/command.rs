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
    /// is likewise the origin space.
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
        .command::<tonk_schema::command::AgentHandoff>()
        .command::<super::repository::EnableSyncRequest>()
        .command::<tonk_schema::command::Load>()
        .command::<tonk_schema::command::PromoteMember>()
        .command::<tonk_schema::command::EnrollCustomer>()
        .command::<tonk_schema::command::ResendActivation>()
        .command::<tonk_schema::command::DeleteAccount>()
        .command::<super::ceremony::AuthorizeDeviceRequest>()
        .migrated::<tonk_schema::command::AddPasskey, tonk_schema::command::legacy::AddPasskey>()
        .migrated::<tonk_schema::command::ExpelMember, tonk_schema::command::legacy::ExpelMember>()
        .migrated::<tonk_schema::command::RemoveSpace, tonk_schema::command::legacy::RemoveSpace>()
        .migrated::<tonk_schema::command::Join, tonk_schema::command::legacy::Join>()
        .migrated::<tonk_schema::command::CheckEmail, tonk_schema::command::legacy::CheckEmail>()
        .migrated::<tonk_schema::command::RegisterAccount, tonk_schema::command::legacy::RegisterAccount>()
        .migrated::<tonk_schema::command::PauseSync, tonk_schema::command::legacy::PauseSync>()
        .migrated::<tonk_schema::command::ProfileRename, tonk_schema::command::legacy::ProfileRename>()
        .migrated::<tonk_schema::command::RenameRepository, tonk_schema::command::legacy::RenameRepository>()
}

/// A space branch's vocabulary — see [`CommandProviders`] for why each
/// of the three is here and nothing else is.
fn space_commands() -> CommandRegistry<CommandEnv> {
    CommandRegistry::new()
        .command::<tonk_schema::command::Load>()
        // A space may request an invite FOR ITSELF: the space view's
        // blank-canvas share (and the seeded `tonk:invite` descriptor)
        // dispatches on the space's own branch, and the refusal flow
        // ("sharing unavailable") publishes to that same branch's
        // overlay. `may_target_space` keeps it self-scoped — a space
        // cannot mint for another space. Profile-side is still the
        // destination once that surface moves.
        .command::<super::repository::InviteRequest>()
        .command::<tonk_schema::command::AgentHandoff>()
        .migrated::<tonk_schema::command::ExpelMember, tonk_schema::command::legacy::ExpelMember>()
        .migrated::<tonk_schema::command::RenameRepository, tonk_schema::command::legacy::RenameRepository>()
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
        let registry = tonk.commands.select(&origin);
        if registry.is_empty() {
            // No command providers for this origin — but the triggering
            // transact already committed and scheduled a poll, so still
            // drain below.
            Vec::new()
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
                        dialog_artifacts::Instruction::Assert(artifact)
                        | dialog_artifacts::Instruction::Replace(artifact)
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
            let env = CommandEnv::new(state.clone(), origin);
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
                tonk.commands = CommandProviders::new(
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
                tonk.commands = CommandProviders::new(
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
            use dialog_operator::Profile;
            use dialog_storage::provider::storage::Storage;

            let storage = Storage::<crate::worker::DefaultSpace>::default();
            let name = format!("command-dispatch-test-{}", rand::random::<u64>());
            let profile = Profile::open(&name).perform(&storage).await.unwrap();
            let session = crate::session::open(&profile, &storage).await.unwrap();
            let reactor = crate::Reactor::new(profile.clone());
            let state = TonkState {
                profile,
                operator: session.operator,
                storage,
                session_expires_at: session.expires_at,
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
                account_keys: Default::default(),
                registry: crate::device::Registry {
                    profile: name.clone(),
                    directory: dialog_effects::storage::Directory::Profile,
                },
                profile_transition: Default::default(),
                context_generation: Default::default(),
            };
            crate::router::repository::bootstrap_profile(&state)
                .await
                .unwrap();
            std::sync::Arc::new(tokio::sync::RwLock::new(state))
        }

        /// The subject DIDs of every user-space replica the profile
        /// lists — the Hub's source of truth, read the same way
        /// `require_real_space` reads it.
        async fn space_subjects(state: &AppState) -> Vec<dialog_varsig::Did> {
            let tonk = state.read().await;
            let meta = tonk
                .reactor
                .profile_repository()
                .branch("main")
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
    }
}
