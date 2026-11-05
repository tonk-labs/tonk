//! Invocations: keeping track of a command someone ran, so they can see
//! what became of it.
//!
//! An invoker that wants to know stamps its command with
//! `xyz.tonk.command/invoked-at` (and the command's concept). For every such
//! command [`super::dispatch`] keeps an *invocation* on the command's own
//! entity, in the session overlay of the branch the command was committed
//! on (see [`tonk_schema::invocation`] for the vocabulary):
//!
//! - when a provider takes it: the command's fields, `case:active` and when
//!   it started;
//! - while it runs: whatever [`Task`] the provider reports;
//! - when it finishes: `case:done` with what it produced, or `case:failed`
//!   with an error entity saying why;
//! - when nothing took it: `case:done` if a rule handled it in the commit
//!   and concluded a `command/completion`, else `case:unhandled`.
//!
//! A finished invocation gets an `expires` time and is retracted then: by
//! a timer started when it finished, by the next dispatch on its branch if
//! that timer never fired, and when a successor worker restores the
//! overlay ([`settle_inherited`]), which also turns invocations still
//! active in the worker that went away into `case:aborted`.
//!
//! Every write goes through one [`Reporter`] per dispatch, which coalesces
//! what its commands report and writes it in one overlay write and one
//! re-evaluation of the branch's subscriptions at most every
//! [`FLUSH_WINDOW`]. A provider that reports progress never waits for the
//! branch's subscribers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use dialog_artifacts::{Changes, Entity, Statement as _, Value};
use tonk_common::log;
use tonk_schema::invocation::{
    Case, Expires, Finished, Outcome, Output, Started, State, Status, case, command, completion,
    retention,
};

use super::{AppState, CommandOrigin};
use crate::worker::TonkState;

/// How long a [`Reporter`] gathers reports before it writes them.
pub const FLUSH_WINDOW: web_time::Duration = web_time::Duration::from_millis(100);

/// The worker's clock, in milliseconds since the epoch: what every
/// invocation time is measured on.
pub fn now() -> f64 {
    super::sync::now_millis()
}

/// How long finished invocations are kept, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retention {
    /// A successful one.
    pub done: f64,
    /// A failed, aborted or unhandled one.
    pub failed: f64,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            done: retention::DONE_SECONDS * 1000.0,
            failed: retention::FAILED_SECONDS * 1000.0,
        }
    }
}

impl Retention {
    /// How long an invocation that ended in `case` is kept.
    pub fn of(&self, case: &str) -> f64 {
        if case == case::DONE {
            self.done
        } else {
            self.failed
        }
    }

    /// The account's `command/retention`, read off the profile branch, with
    /// the defaults for whatever it does not set. A setting that is not a
    /// positive number of seconds is ignored.
    pub async fn read(tonk: &TonkState) -> Self {
        let mut retention = Self::default();
        let Ok(session) = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
        else {
            return retention;
        };
        let read = |the: dialog_query::Relation| async {
            use dialog_query::{AttributeQuery, Output as _, Term};
            let claims: Vec<dialog_query::Claim> = session
                .handle()
                .query()
                .select(AttributeQuery::from(
                    Term::from(the)
                        .of(Term::<Entity>::var("of"))
                        .is(Term::<f64>::var("seconds")),
                ))
                .perform(&tonk.operator)
                .try_vec()
                .await
                .unwrap_or_default();
            claims.into_iter().find_map(|claim| match claim.is {
                Value::Float(seconds) if seconds > 0.0 => Some(seconds * 1000.0),
                _ => None,
            })
        };
        if let Some(done) = read(retention::Done::the()).await {
            retention.done = done;
        }
        if let Some(failed) = read(retention::Failed::the()).await {
            retention.failed = failed;
        }
        retention
    }
}

/// The commands in `transients` whose invoker asked for an invocation,
/// each with its own facts: the entities carrying `invoked-at`.
pub fn invoked(transients: &Changes) -> BTreeMap<Entity, Changes> {
    let stamp = command::InvokedAt::the().to_string();
    let entities: BTreeSet<Entity> = transients
        .iter()
        .filter(|(_, the, _)| the.to_string() == stamp)
        .map(|(entity, _, _)| entity.clone())
        .collect();
    entities
        .into_iter()
        .map(|entity| {
            let mut facts = transients.clone();
            facts.retain_entities(|of| *of == entity);
            (entity, facts)
        })
        .collect()
}

/// What a rule concluded about `entity` in the commit that dispatched it:
/// the output of the `command/completion` it asserted, if it asserted one.
/// `Some(None)` is a completion naming no output.
pub fn completion(concluded: &Changes, entity: &Entity) -> Option<Option<Entity>> {
    let output = completion::Output::the().to_string();
    let mut concluded_any = false;
    for (of, the, change) in concluded.iter() {
        if of != entity || the.to_string() != output {
            continue;
        }
        concluded_any = true;
        if let Value::Entity(produced) = change.value() {
            return Some(Some(produced.clone()));
        }
    }
    concluded_any.then_some(None)
}

/// The entity a failed invocation's error is recorded on. One per
/// invocation, derived from it, so it is swept with it.
pub fn error_entity(invocation: &Entity) -> Entity {
    let id = bs58::encode(invocation.to_string().as_bytes()).into_string();
    format!("error:{id}")
        .parse()
        .expect("an error entity is a valid URI")
}

fn case_entity(name: &str) -> Case {
    Case(case::entity(name))
}

/// The facts that open an invocation a provider took: the command's own
/// fields, `case:active` and when it started.
pub fn opened(entity: &Entity, fields: &Changes, now: f64) -> Changes {
    let mut changes = fields.clone();
    Case::of(entity.clone())
        .is(case_entity(case::ACTIVE))
        .assert(&mut changes);
    Started::of(entity.clone())
        .is(Started(now))
        .assert(&mut changes);
    changes
}

/// The facts that close an invocation: where it ended, what it produced
/// (or the error saying why it failed), when, and when it expires.
pub fn closed(entity: &Entity, outcome: &Outcome, now: f64, retention: &Retention) -> Changes {
    let mut changes = Changes::new();
    let name = match outcome {
        Ok(done) => {
            if let Some(output) = &done.output {
                Output::of(entity.clone())
                    .is(Output(output.clone()))
                    .assert(&mut changes);
            }
            case::DONE
        }
        Err(failure) => {
            let error = error_entity(entity);
            Status::of(error.clone())
                .is(Status(failure.0.clone()))
                .assert(&mut changes);
            Output::of(entity.clone())
                .is(Output(error))
                .assert(&mut changes);
            case::FAILED
        }
    };
    ended(&mut changes, entity, name, now, retention);
    changes
}

/// The facts recording an invocation no provider took: done when a rule
/// concluded a completion for it, unhandled otherwise. It never ran, so it
/// starts and ends at `now`.
pub fn settled(
    entity: &Entity,
    fields: &Changes,
    completed: Option<Option<Entity>>,
    now: f64,
    retention: &Retention,
) -> Changes {
    let mut changes = fields.clone();
    Started::of(entity.clone())
        .is(Started(now))
        .assert(&mut changes);
    let name = match completed {
        Some(output) => {
            if let Some(output) = output {
                Output::of(entity.clone())
                    .is(Output(output))
                    .assert(&mut changes);
            }
            case::DONE
        }
        None => case::UNHANDLED,
    };
    ended(&mut changes, entity, name, now, retention);
    changes
}

fn ended(changes: &mut Changes, entity: &Entity, name: &str, now: f64, retention: &Retention) {
    Case::of(entity.clone())
        .is(case_entity(name))
        .assert(changes);
    Finished::of(entity.clone())
        .is(Finished(now))
        .assert(changes);
    Expires::of(entity.clone())
        .is(Expires(now + retention.of(name)))
        .assert(changes);
}

/// Every entity `overlay` holds an invocation for that has expired by
/// `now`, with the error entities recorded for them.
pub fn expired(overlay: &Changes, now: f64) -> BTreeSet<Entity> {
    let expires = Expires::the().to_string();
    let mut gone = BTreeSet::new();
    for (entity, the, change) in overlay.iter() {
        if the.to_string() != expires {
            continue;
        }
        if let (Value::Float(at), dialog_artifacts::Change::Assert(..)) = (change.value(), change)
            && *at <= now
        {
            gone.insert(error_entity(entity));
            gone.insert(entity.clone());
        }
    }
    gone
}

/// Every invocation `overlay` holds that is still active.
pub fn active(overlay: &Changes) -> BTreeSet<Entity> {
    let case_attribute = Case::the().to_string();
    let active = Value::Entity(case::entity(case::ACTIVE));
    overlay
        .iter()
        .filter(|(_, the, change)| {
            the.to_string() == case_attribute
                && matches!(change, dialog_artifacts::Change::Assert(..))
                && *change.value() == active
        })
        .map(|(entity, _, _)| entity.clone())
        .collect()
}

/// Settle the overlay a successor worker inherits: drop the invocations
/// that have expired, and close the ones still active as `case:aborted`,
/// since the worker that was running them is gone and nothing will report
/// how they ended.
pub fn settle_inherited(overlay: &mut Changes, now: f64, retention: &Retention) {
    let gone = expired(overlay, now);
    overlay.retain_entities(|entity| !gone.contains(entity));
    let mut aborted = Changes::new();
    for entity in active(overlay) {
        ended(&mut aborted, &entity, case::ABORTED, now, retention);
    }
    overlay.merge(aborted);
}

/// Retract the invocations on `tonk`'s `origin` branch that have expired
/// by `now`. Returns whether any were.
pub async fn sweep(tonk: &TonkState, origin: &CommandOrigin, now: f64) -> bool {
    let Ok(session) = branch(tonk, origin).acquire(&tonk.operator).await else {
        return false;
    };
    let gone = expired(&session.handle().overlay().export(), now);
    if gone.is_empty() {
        return false;
    }
    let removed = session
        .state
        .retain_overlay_entities(|entity| !gone.contains(entity));
    if removed {
        tonk.reactor.schedule_poll(Arc::clone(&session.state));
    }
    removed
}

/// The branch a command fired on: the profile's for a profile origin
/// (whose branch name a test may leave empty: the active one), else the
/// named space's.
fn branch<'a>(tonk: &'a TonkState, origin: &'a CommandOrigin) -> crate::BranchReference<'a> {
    if origin.repo.is_empty() {
        let name = if origin.branch.is_empty() {
            tonk.active_branch.as_str()
        } else {
            origin.branch.as_str()
        };
        tonk.reactor.profile_repository().branch(name)
    } else {
        tonk.reactor.repository(&origin.repo).branch(&origin.branch)
    }
}

/// Write `changes` to the overlay of the branch `origin` names, and tell
/// its subscribers. Logs and drops the write when the branch cannot be
/// opened (a transient-only commit on a space this device has not pulled).
async fn publish(state: &AppState, origin: &CommandOrigin, changes: Changes) {
    let tonk = state.read().await;
    if let Err(error) = branch(&tonk, origin)
        .overlay()
        .assert(changes)
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("invocations: not recorded on {origin:?}: {error}");
        return;
    }
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Gathers what one dispatch's commands report and writes it to their
/// branch's overlay, coalesced.
///
/// The first report after a write starts a flush that waits
/// [`FLUSH_WINDOW`] and writes everything reported by then in one overlay
/// write, then re-evaluates the branch's subscriptions once. Writes are
/// serialized, so a later report never lands before an earlier one.
#[derive(Clone)]
pub struct Reporter {
    inner: Arc<ReporterInner>,
}

struct ReporterInner {
    state: AppState,
    origin: CommandOrigin,
    pending: parking_lot::Mutex<Option<Changes>>,
    writing: tokio::sync::Mutex<()>,
}

impl Reporter {
    /// A reporter writing to `origin`'s branch.
    pub fn new(state: AppState, origin: CommandOrigin) -> Self {
        Self {
            inner: Arc::new(ReporterInner {
                state,
                origin,
                pending: parking_lot::Mutex::new(None),
                writing: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Queue `changes`, starting a flush if none is waiting.
    pub fn report(&self, changes: Changes) {
        let start = {
            let mut pending = self.inner.pending.lock();
            match pending.as_mut() {
                Some(queued) => {
                    queued.merge(changes);
                    false
                }
                None => {
                    *pending = Some(changes);
                    true
                }
            }
        };
        if start {
            let reporter = self.clone();
            crate::detach(async move {
                let _ = crate::sleep(FLUSH_WINDOW).await;
                reporter.flush().await;
            });
        }
    }

    /// Write whatever is queued now, without waiting for the window.
    pub async fn flush(&self) {
        let _writing = self.inner.writing.lock().await;
        let Some(changes) = self.inner.pending.lock().take() else {
            return;
        };
        if changes.is_empty() {
            return;
        }
        publish(&self.inner.state, &self.inner.origin, changes).await;
    }

    /// Retract the invocations on this reporter's branch that have expired
    /// once `after` has passed. Detached: the sweep outlives the dispatch.
    pub fn expire_after(&self, after: f64) {
        let state = self.inner.state.clone();
        let origin = self.inner.origin.clone();
        crate::detach(async move {
            let wait = web_time::Duration::from_millis(after.max(0.0) as u64);
            let _ = crate::sleep(wait).await;
            let tonk = state.read().await;
            if sweep(&tonk, &origin, now()).await {
                tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            }
        });
    }
}

/// What a provider uses to report how its command is going.
///
/// [`super::CommandEnv::task`] hands one to every provider. For a command
/// nobody asked to track it does nothing, so a provider never needs to
/// check.
#[derive(Clone, Default)]
pub struct Task {
    tracked: Option<(Entity, Reporter)>,
}

impl Task {
    /// The task of the invocation on `entity`, reported through `reporter`.
    pub fn tracked(entity: Entity, reporter: Reporter) -> Self {
        Self {
            tracked: Some((entity, reporter)),
        }
    }

    /// The command entity this task reports on, when it is tracked.
    pub fn entity(&self) -> Option<&Entity> {
        self.tracked.as_ref().map(|(entity, _)| entity)
    }

    /// Say the command is in `state`: an entity that already has a
    /// `xyz.tonk.state/status` (a domain entity a view knows how to show).
    pub fn state(&self, state: Entity) {
        let Some((entity, reporter)) = &self.tracked else {
            return;
        };
        let mut changes = Changes::new();
        State::of(entity.clone())
            .is(State(state))
            .assert(&mut changes);
        reporter.report(changes);
    }

    /// Say the command is in `state`, and that `state` says `status`: for a
    /// state entity with nothing else to it, such as `status:minting`.
    pub fn status(&self, state: Entity, status: impl Into<String>) {
        let Some((entity, reporter)) = &self.tracked else {
            return;
        };
        let mut changes = Changes::new();
        Status::of(state.clone())
            .is(Status(status.into()))
            .assert(&mut changes);
        State::of(entity.clone())
            .is(State(state))
            .assert(&mut changes);
        reporter.report(changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_query::the;
    use tonk_schema::invocation::{Done, Failure};
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    fn entity(uri: &str) -> Entity {
        uri.parse().expect("entity URI")
    }

    fn values(changes: &Changes, of: &Entity, the: impl ToString) -> Vec<Value> {
        let the = the.to_string();
        changes
            .iter()
            .filter(|(entity, relation, change)| {
                *entity == of
                    && relation.to_string() == the
                    && matches!(change, dialog_artifacts::Change::Assert(..))
            })
            .map(|(_, _, change)| change.value().clone())
            .collect()
    }

    fn retention() -> Retention {
        Retention {
            done: 1_000.0,
            failed: 5_000.0,
        }
    }

    #[dialog_common::test]
    fn it_tracks_only_the_commands_stamped_with_invoked_at() {
        let mut transients = Changes::new();
        the!("xyz.tonk.rename-repository/name")
            .of(entity("id:tracked"))
            .is("Plans".to_owned())
            .assert(&mut transients);
        command::InvokedAt::of(entity("id:tracked"))
            .is(command::InvokedAt(10.0))
            .assert(&mut transients);
        the!("xyz.tonk.rename-repository/name")
            .of(entity("id:untracked"))
            .is("Other".to_owned())
            .assert(&mut transients);

        let invoked = invoked(&transients);
        assert_eq!(
            invoked.keys().cloned().collect::<Vec<_>>(),
            vec![entity("id:tracked")]
        );
        let fields = &invoked[&entity("id:tracked")];
        assert_eq!(
            values(
                fields,
                &entity("id:tracked"),
                "xyz.tonk.rename-repository/name"
            ),
            vec![Value::String("Plans".into())],
            "an invocation carries its command's own fields"
        );
        assert!(
            values(
                fields,
                &entity("id:untracked"),
                "xyz.tonk.rename-repository/name"
            )
            .is_empty(),
            "and no other command's"
        );
    }

    #[dialog_common::test]
    fn it_opens_an_invocation_active_with_its_fields() {
        let this = entity("id:run");
        let mut fields = Changes::new();
        the!("xyz.tonk.rename-repository/name")
            .of(this.clone())
            .is("Plans".to_owned())
            .assert(&mut fields);
        let opened = opened(&this, &fields, 42.0);
        assert_eq!(
            values(&opened, &this, Case::the()),
            vec![Value::Entity(case::entity(case::ACTIVE))]
        );
        assert_eq!(
            values(&opened, &this, Started::the()),
            vec![Value::Float(42.0)]
        );
        assert_eq!(
            values(&opened, &this, "xyz.tonk.rename-repository/name"),
            vec![Value::String("Plans".into())]
        );
    }

    #[dialog_common::test]
    fn it_closes_a_success_with_its_output_and_a_short_expiry() {
        let this = entity("id:run");
        let produced = entity("did:key:zSpace");
        let closed = closed(&this, &Done::with(produced.clone()), 100.0, &retention());
        assert_eq!(
            values(&closed, &this, Case::the()),
            vec![Value::Entity(case::entity(case::DONE))]
        );
        assert_eq!(
            values(&closed, &this, Output::the()),
            vec![Value::Entity(produced)]
        );
        assert_eq!(
            values(&closed, &this, Finished::the()),
            vec![Value::Float(100.0)]
        );
        assert_eq!(
            values(&closed, &this, Expires::the()),
            vec![Value::Float(1_100.0)]
        );
    }

    #[dialog_common::test]
    fn it_closes_a_failure_with_an_error_entity_saying_why() {
        let this = entity("id:run");
        let failure: Outcome = Err(Failure::new("the space is not yours to rename"));
        let closed = closed(&this, &failure, 100.0, &retention());
        let error = error_entity(&this);
        assert_eq!(
            values(&closed, &this, Case::the()),
            vec![Value::Entity(case::entity(case::FAILED))]
        );
        assert_eq!(
            values(&closed, &this, Output::the()),
            vec![Value::Entity(error.clone())]
        );
        assert_eq!(
            values(&closed, &error, Status::the()),
            vec![Value::String("the space is not yours to rename".into())]
        );
        assert_eq!(
            values(&closed, &this, Expires::the()),
            vec![Value::Float(5_100.0)],
            "a failure is kept longer, so it can be found"
        );
    }

    #[dialog_common::test]
    fn it_settles_a_command_no_provider_took() {
        let this = entity("id:run");
        let notebook = entity("id:notebook");
        let done = settled(
            &this,
            &Changes::new(),
            Some(Some(notebook.clone())),
            7.0,
            &retention(),
        );
        assert_eq!(
            values(&done, &this, Case::the()),
            vec![Value::Entity(case::entity(case::DONE))],
            "a rule concluded a completion: done"
        );
        assert_eq!(
            values(&done, &this, Output::the()),
            vec![Value::Entity(notebook)]
        );
        let unhandled = settled(&this, &Changes::new(), None, 7.0, &retention());
        assert_eq!(
            values(&unhandled, &this, Case::the()),
            vec![Value::Entity(case::entity(case::UNHANDLED))],
            "nothing concluded anything: unhandled"
        );
        assert!(values(&unhandled, &this, Output::the()).is_empty());
    }

    #[dialog_common::test]
    fn it_reads_a_completion_a_rule_concluded() {
        let this = entity("id:run");
        let mut concluded = Changes::new();
        completion::Output::of(this.clone())
            .is(completion::Output(entity("id:notebook")))
            .assert(&mut concluded);
        assert_eq!(
            completion(&concluded, &this),
            Some(Some(entity("id:notebook")))
        );
        assert_eq!(completion(&concluded, &entity("id:other")), None);
        assert_eq!(completion(&Changes::new(), &this), None);
    }

    #[dialog_common::test]
    fn it_finds_what_has_expired_with_its_error() {
        let old = entity("id:old");
        let fresh = entity("id:fresh");
        let mut overlay = closed(&old, &Err(Failure::new("no")), 0.0, &retention());
        overlay.merge(closed(&fresh, &Done::nothing(), 4_500.0, &retention()));
        let gone = expired(&overlay, 5_000.0);
        assert!(gone.contains(&old), "{gone:?}");
        assert!(gone.contains(&error_entity(&old)), "{gone:?}");
        assert!(!gone.contains(&fresh), "{gone:?}");
    }

    #[dialog_common::test]
    fn a_successor_aborts_what_was_active_and_drops_what_expired() {
        let running = entity("id:running");
        let expired_one = entity("id:expired");
        let kept = entity("id:kept");
        let mut overlay = opened(&running, &Changes::new(), 0.0);
        overlay.merge(closed(&expired_one, &Done::nothing(), 0.0, &retention()));
        overlay.merge(closed(&kept, &Done::nothing(), 9_500.0, &retention()));

        settle_inherited(&mut overlay, 10_000.0, &retention());

        assert_eq!(
            values(&overlay, &running, Case::the()),
            vec![Value::Entity(case::entity(case::ABORTED))],
            "nothing will finish what the old worker was running"
        );
        assert_eq!(
            values(&overlay, &running, Expires::the()),
            vec![Value::Float(15_000.0)]
        );
        assert!(
            values(&overlay, &expired_one, Case::the()).is_empty(),
            "an expired invocation is not carried over"
        );
        assert_eq!(
            values(&overlay, &kept, Case::the()),
            vec![Value::Entity(case::entity(case::DONE))],
            "one that has not expired is kept as it was"
        );
    }

    #[dialog_common::test]
    fn an_untracked_task_reports_nothing() {
        let task = Task::default();
        assert!(task.entity().is_none());
        task.status(entity("status:working"), "working");
        task.state(entity("status:working"));
    }
}
