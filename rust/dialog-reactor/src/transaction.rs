//! [`TransactionBuilder`] and [`Commit`] — accumulate
//! assert/retract pairs and commit them through the branch's
//! [`Stack`](dialog_repository::Stack).
//!
//! Routes call this instead of `Branch::transaction()`. A commit is one
//! invocation through the stack ([`Stack::assert`]): the durable writes
//! land in the primary; a command (a claim of a transient concept,
//! written `.at(control_layer())`) is the dispatch's stimulus and, once
//! the dispatch has published, lands on the outside end of the stack's
//! `control:` channel for one tick. The dispatch's report names that
//! tick's commands, and [`Commit::dispatch`] hands them to the caller
//! as [`Landed::effect`]: the stack's providers over exactly the
//! commands this dispatch landed, to perform when the caller chooses
//! ([`Effect::perform`](dialog_repository::Effect::perform)).
//!
//! A tick that carries commands may carry rows of the `control:` entity
//! itself beside them, written into the same share by
//! [`TransactionBuilder::stamp`]: the asking client, for one
//! ([`control::client`](crate::control::client)).
//!
//! On a successful commit, [`Commit::perform`] **schedules** a poll of
//! the affected branch ([`Reactor::schedule_poll`]) rather than
//! polling inline — so a commit and a session-overlay write on the
//! same branch in one turn coalesce into a single re-evaluation.
//! Every entry point that commits is therefore responsible for
//! draining the scheduled set once it's done
//! ([`Reactor::run_scheduled_polls`]); the command runner does
//! this for the transact path, and the create/migration/claim
//! routes drain explicitly. The builder is lazy; nothing touches
//! the branch until `Commit::perform`.
//!
//! [`Reactor::schedule_poll`]: crate::Reactor::schedule_poll
//! [`Reactor::run_scheduled_polls`]: crate::Reactor::run_scheduled_polls
//! [`Stack::assert`]: dialog_repository::Stack::assert

use dialog_corpus::{Changes, Pick, Relation, Statement, Update as _, Value};
use dialog_repository::{Effect, Revision, control_layer};
use tonk_schema::claim::{Claim, PredicateApplication};
use tonk_schema::transact::application_plan_from_predicate;

use super::BranchReference;
use super::env::{BranchOpenProvider, CommitProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// Builder — accumulates assertions and retractions into one
/// [`Changes`] batch. Chain off `TonkBranch::transaction`.
///
/// A claim of a transient concept is a command: it is written
/// `.at(control_layer())`, so the stack's commit takes it as the
/// dispatch's stimulus rather than a write to the primary, and it
/// reaches `control:` once the dispatch publishes. Routes that don't
/// carry transient classification (e.g. the legacy `/claim/*` path)
/// keep using the raw [`Self::assert`] / [`Self::retract`] entries,
/// which write to the primary.
pub struct TransactionBuilder<'a> {
    /// The branch the transaction will commit to.
    pub branch: BranchReference<'a>,
    /// Accumulated claims, durable and commands alike: a command's
    /// facts are in the `control:` share.
    pub changes: Changes,
    /// Rows of the `control:` entity for the tick, written beside the
    /// commands when there are any. See [`stamp`](Self::stamp).
    stamps: Vec<(Relation, Value)>,
    /// Whether a command has been applied, so the stamps have a tick to
    /// land in.
    commands: bool,
}

impl<'a> TransactionBuilder<'a> {
    /// Build a new transaction with no claims accumulated yet.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            changes: Changes::new(),
            stamps: Vec::new(),
            commands: false,
        }
    }

    /// Write the row `the is` on the `control:` entity, into the same
    /// `control:` share the transaction's commands go in, so it lands in
    /// their tick and whoever performs them reads it from the tick's
    /// rows: the asking client ([`control::client`](crate::control::client)),
    /// for one. Written only when the transaction asserts a command; a
    /// durable-only commit lands no tick to stamp.
    pub fn stamp(mut self, the: Relation, is: Value) -> Self {
        self.stamps.push((the, is));
        self
    }

    /// Add an assertion to the primary. Used by callers that operate
    /// on raw dialog [`Statement`] values (legacy `/claim/*` and tests)
    /// without a durability classification — assume durable.
    pub fn assert<S: Statement>(mut self, claim: S) -> Self {
        claim.assert(&mut self.changes);
        self
    }

    /// Add a retraction to the primary. Same fallback semantics as
    /// [`Self::assert`].
    pub fn retract<S: Statement>(mut self, claim: S) -> Self {
        claim.retract(&mut self.changes);
        self
    }

    /// Apply a typed [`Claim`] from the wire format. Each application
    /// is planned into raw EAV facts (same emitter the
    /// asserted-notation planner uses); a transient predicate's land
    /// in `control:`, a durable one's in the primary.
    pub fn apply(self, claim: Claim) -> Self {
        match claim {
            Claim::Assert(application) => self.apply_assert(application),
            Claim::Retract(application) => self.apply_retract(application),
        }
    }

    /// Assert a [`PredicateApplication`]. A transient predicate is a
    /// command, written `.at(control_layer())`.
    pub fn apply_assert(mut self, application: PredicateApplication) -> Self {
        if application.is_transient() {
            self.commands = true;
            application_plan_from_predicate(application)
                .at(control_layer())
                .assert(&mut self.changes);
        } else {
            application_plan_from_predicate(application).assert(&mut self.changes);
        }
        self
    }

    /// Retract a [`PredicateApplication`]. Routed by the same rule as
    /// [`Self::apply_assert`] — a retraction of a transient predicate
    /// goes to `control:` so it pairs with whatever assertion the same
    /// predicate emitted earlier in the document.
    pub fn apply_retract(mut self, application: PredicateApplication) -> Self {
        if application.is_transient() {
            application_plan_from_predicate(application)
                .at(control_layer())
                .retract(&mut self.changes);
        } else {
            application_plan_from_predicate(application).retract(&mut self.changes);
        }
        self
    }

    /// Wrap the accumulated changes into a [`Commit`] effect.
    #[track_caller]
    pub fn commit(self) -> Commit<'a> {
        let mut changes = self.changes;
        if self.commands {
            for (the, is) in self.stamps {
                stamp(&mut changes, the, is);
            }
        }
        Commit {
            origin: std::panic::Location::caller(),
            branch: self.branch,
            changes,
        }
    }
}

/// Write the row `the is` on the `control:` entity into `changes`'
/// `control:` share, where it lands in the tick of the commands written
/// there with it. What [`TransactionBuilder::stamp`] does at commit; for
/// a caller that invokes commands through the stack itself.
pub fn stamp(changes: &mut Changes, the: Relation, is: Value) {
    let mut row = Changes::new();
    row.associate(the, control_layer(), is, Pick::All);
    row.at(control_layer()).assert(changes);
}

/// Commit effect — performs one dispatch through the branch's stack,
/// then schedules a re-poll of every subscription on the branch so
/// changed query results fan out to subscribers.
pub struct Commit<'a> {
    /// Where `.commit()` was called, so the commit log names its caller.
    pub origin: &'static std::panic::Location<'static>,
    /// The branch the commit applies to.
    pub branch: BranchReference<'a>,
    /// The claims: durable ones for the primary, commands in the
    /// `control:` share.
    pub changes: Changes,
}

/// What a dispatch left behind: the head it left the primary at, and
/// the effect of the commands that landed on the stack's `control:`
/// channel for it.
///
/// The commands are the ones this dispatch wrote `.at(control_layer())`
/// and the ones its rules dispatched in any tick, as rows: a provider
/// decodes a command from the facts of its entity, since nothing of a
/// command stays anywhere a later read could find it. Nothing runs
/// until the effect is performed, so a route answers first and
/// performs it after.
#[derive(Debug, Clone)]
pub struct Landed {
    /// The revision the dispatch published, or the unchanged head when
    /// it wrote nothing durable.
    pub revision: Revision,
    /// The stack's providers over the commands that stood in `control:`
    /// for this dispatch; empty when it dispatched none.
    pub effect: Effect,
}

impl Commit<'_> {
    /// Execute the commit. On success, every subscription on
    /// the branch is scheduled for re-evaluation. The commands the
    /// dispatch landed are dropped: a caller that runs providers wants
    /// [`dispatch`](Self::dispatch).
    pub async fn perform<Env>(self, env: &Env) -> Result<Revision, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider + SelectProvider,
    {
        self.dispatch(env).await.map(|landed| landed.revision)
    }

    /// Execute the commit as one dispatch through the branch's stack and
    /// read back what it landed in `control:`.
    ///
    /// The stack's commit runs induction over the user's claims and the
    /// branch's rules, routes each write to its layer, publishes the
    /// primary's share with one compare-and-swap, and re-runs the whole
    /// dispatch from the new head when a sync moved it meanwhile (see
    /// [`Invocation`](dialog_repository::Invocation)). Once it has
    /// published, its commands land on `control:`'s outside end as one
    /// tick, which its report names; [`Landed::effect`] is the stack's
    /// providers over exactly those. Rows, not one concept's
    /// conclusions: a provider decodes a command from the shape of its
    /// entity's facts, so it wants every row.
    pub async fn dispatch<Env>(self, env: &Env) -> Result<Landed, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider + SelectProvider,
    {
        let repo = self.branch.repository.name().to_owned();
        let name = self.branch.name.to_owned();
        let origin = self.origin;
        let cached = self.branch.acquire(env).await?;
        // A tree laid out the way an earlier release wrote is read, never
        // written into: move it first, once per branch per process. A
        // no-op after the first commit or read that ran it.
        self.branch.migrate_once(env).await?;

        // Serialize every commit on this branch through the per-branch
        // transactor lock — so concurrent writers (the transact route AND the
        // direct handler commits: invite, join, pause, …) line up here rather
        // than racing the head CAS. The lock lives on `.commit()` so no caller
        // can sidestep it. Held across the whole induce+commit, and across
        // the read of `control:` after it, so the commands read are this
        // dispatch's alone. Sync does NOT take it — it coordinates via the
        // CAS and retries — so a commit never waits on a sync.
        let _transacting = cached.state.transactor().lock().await;
        let stack = cached.state.stack();

        let changes = self.changes;
        let (claims, attributes) = describe(&changes);
        let before = stack.primary().revision();

        let t_commit = web_time::Instant::now();
        // The stack re-runs the dispatch from the new head when a sync
        // advanced it between the snapshot and the publish: the changes
        // are idempotent asserts/retracts, so replaying them on the
        // advanced head is safe. Nothing here is computed from a read, so
        // the one-statement `assert` serves; a route that must read
        // first would go through `Stack::transact` with a body.
        let report = stack.assert(changes).perform(env).await?;
        let revision = report.revision().clone();
        let commit_ms = t_commit.elapsed().as_millis();
        let effect = report.effect();

        // Name the commit: the branch, the claim count, and whether the
        // tree moved. A commit that lands an identical tree still mints a
        // revision and publishes it, so a caller re-asserting what is
        // already there reads as "tree unchanged" here, and that is the
        // one to go and find.
        let moved = if before.as_ref().map(|before| &before.tree) == Some(&revision.tree) {
            "unchanged"
        } else {
            "moved"
        };
        let head = if before.as_ref().map(|before| before.version()) == Some(revision.version()) {
            "same"
        } else {
            "advanced"
        };
        dialog_common::log!(
            "reactor commit timing: commit {commit_ms}ms on {name}@{repo}, \
             {claims} claims ({attributes}), tree {moved}, head {head}, from {origin}"
        );

        // Schedule the poll rather than running it inline: the request's
        // command runner drains the scheduled set once after its providers
        // run (`Reactor::run_scheduled_polls`), so a commit and a
        // session-overlay write on the same branch in one turn coalesce
        // into a single re-evaluation. Every entry point that commits must
        // drain.
        self.branch
            .reactor()
            .schedule_poll(std::sync::Arc::clone(&cached.state));

        Ok(Landed { revision, effect })
    }
}

/// How many claims a batch carries and the attributes they touch,
/// deduplicated: enough to name the concept a commit carries without
/// printing its values. Counts the writes to every layer, the
/// commands included.
fn describe(changes: &Changes) -> (usize, String) {
    let mut names: Vec<String> = changes
        .iter()
        .map(|(_, the, _)| the.to_string())
        .chain(changes.iter_in().map(|(_, _, the, _)| the.to_string()))
        .collect();
    let claims = names.len();
    names.sort();
    names.dedup();
    (claims, names.join(","))
}
