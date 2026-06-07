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

/// Build the registry of command handlers the worker installs at
/// startup. Real handlers register here via the chainable
/// [`command`](CommandRegistry::command) builder, e.g.
/// `CommandRegistry::new().command(create_repo)`.
pub fn command_registry() -> CommandRegistry {
    CommandRegistry::new()
}

/// Run every command handler the just-committed `transients` triggered,
/// committing each handler's outcome to the same `(repo, branch)` the
/// command arrived on.
///
/// Called by a mutation path (e.g. `/transact`) after its commit, with
/// `AppState` and the branch identity in hand. The transients have
/// already been swept from durable storage by the commit; we matched
/// them from the pre-commit buffer, so the trigger fired exactly once.
///
/// Each handler runs and its outcome commits independently and
/// *concurrently* — handlers don't block one another, and a slow or
/// failing one doesn't hold up the rest. (Concurrent, not parallel:
/// they share the single SW task, interleaving at await points.)
pub async fn dispatch(state: &AppState, repo: RepoTarget, branch: String, transients: Changes) {
    // Match handlers and build their `'static` run-futures while
    // holding the read lock — the futures own everything (decoded
    // trigger, extracted capabilities), so we can drop the lock before
    // awaiting them. That keeps handler IO from ever running under a
    // held lock (a handler that extracts `State<AppState>` may re-lock).
    let run_futures = {
        let tonk = state.read().await;
        if tonk.commands.is_empty() {
            return;
        }
        tonk.commands
            .match_transients(&transients)
            .into_iter()
            .map(|(handler, facts)| handler.run(&facts, state))
            .collect::<Vec<_>>()
    };

    // Drive every (run → commit) chain concurrently. Each chain awaits
    // its handler, then commits its outcome to the command's branch;
    // `join_all` interleaves them so independent effects make progress
    // together rather than strictly in sequence.
    let chains = run_futures.into_iter().map(|run| async {
        if let Some(changes) = run.await {
            commit_outcome(state, &repo, &branch, changes).await;
        }
    });
    futures_util::future::join_all(chains).await;
}

/// Which repository a command's branch belongs to — the profile
/// repository, or a named one. Mirrors the reactor's repository
/// addressing so the outcome commit re-acquires the right branch.
#[derive(Clone, Debug)]
pub enum RepoTarget {
    /// The profile-as-repository (the Hub's meta branch lives here).
    Profile,
    /// A named repository.
    Named(String),
}

/// Commit one handler's outcome `Changes` to `(repo, branch)` through
/// the reactor, so the write fans out to subscribers like any other.
/// Logs and swallows errors: an outcome that fails to commit must not
/// take down the dispatch loop (the command already succeeded).
async fn commit_outcome(state: &AppState, repo: &RepoTarget, branch: &str, changes: Changes) {
    let tonk = state.read().await;
    let repository = match repo {
        RepoTarget::Profile => tonk.reactor.profile_repository(),
        RepoTarget::Named(name) => tonk.reactor.repository(name),
    };

    // `Changes` implements `Statement`, so the whole outcome batch
    // asserts in one call.
    let result = repository
        .branch(branch)
        .transaction()
        .assert(changes)
        .commit()
        .perform(&tonk.operator)
        .await;

    if let Err(error) = result {
        tonk_common::log!("[command] outcome commit failed on branch '{branch}': {error}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(missing_docs)]

    use super::*;
    use crate::reactor::{CommandTx, State};
    use dialog_artifacts::{Changes, Statement};
    use dialog_query::{Attribute, Concept, Entity, the};

    // A command that declares a capability: it reads `State<AppState>`,
    // proving a non-pure handler registers and triggers. The handler
    // body doesn't touch the state here (a real one would re-lock to
    // reach the operator/reactor) — the point is that declaring the
    // capability compiles and the dispatcher supplies it.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.command")]
    pub struct PingTag(pub String);

    #[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Ping {
        pub this: Entity,
        pub tag: PingTag,
    }

    async fn handle_ping(ping: Ping, _state: State<AppState>, mut tx: CommandTx) -> CommandTx {
        // A real handler would use `_state` to do IO; here we just echo.
        tx.assert(ping);
        tx
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
    fn it_registers_a_capability_declaring_handler() {
        // `handle_ping` declares `State<AppState>`; the chainable
        // builder infers `C = Ping` and `Args = (State<AppState>,)`.
        let registry = command_registry().command(handle_ping);
        let changes = ping_transient("did:key:zPing", "hi");
        assert_eq!(
            registry.match_transients(&changes).len(),
            1,
            "the capability-declaring Ping handler should match its trigger"
        );
    }
}
