//! Web-local account attempt lifecycle and PostHog adapter.

#![cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use tonk_analytics::account::{
    AccountAction as AnalyticsAction, AccountEvent, AccountOutcome, AccountState, FailureKind,
    Journey, Stage, Surface, Trigger,
};

/// Start an attempt before an asynchronous operation and return the still-open
/// recorder beside its result. Callers retain the typed error evidence needed
/// to choose the terminal outcome at the presentation seam.
pub(crate) async fn observe<F, T, E>(
    action: AccountAction,
    surface: Surface,
    trigger: Trigger,
    account_state: AccountState,
    future: F,
) -> (WebAccountAttempt, Result<T, E>)
where
    F: std::future::Future<Output = Result<T, E>>,
{
    let attempt = WebAccountAttempt::start(action, surface, trigger, account_state);
    let result = future.await;
    (attempt, result)
}

use crate::user_error::AccountAction;

thread_local! {
    static AUTOMATIC_FAILURES: RefCell<HashSet<(AnalyticsAction, FailureKind)>> = RefCell::new(HashSet::new());
    static FALLBACK_ID: RefCell<u64> = const { RefCell::new(0) };
}

const PENDING_SETTLE: &str = "tonk:account-observability:pending-settle";

/// Remember that a ceremony parked on the emailed link still needs to
/// converge into a working account. Session storage survives the
/// activation-page detour but does not create a stable cross-session
/// identifier.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn mark_settle_pending() {
    let _ = web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .and_then(|storage| storage.set_item(PENDING_SETTLE, "1").ok());
}

/// Consume the page-local activation convergence marker exactly once.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn take_settle_pending() -> bool {
    web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .is_some_and(|storage| {
            let pending = storage.get_item(PENDING_SETTLE).ok().flatten().is_some();
            if pending {
                let _ = storage.remove_item(PENDING_SETTLE);
            }
            pending
        })
}

trait Recorder {
    fn capture(&self, event: AccountEvent);
}

#[derive(Clone, Copy)]
struct PostHogRecorder;

impl Recorder for PostHogRecorder {
    fn capture(&self, event: AccountEvent) {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let _ = tonk_analytics::web::capture_account(&event);
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let _ = event;
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MemoryRecorder(Rc<RefCell<Vec<AccountEvent>>>);

#[cfg(test)]
impl Recorder for MemoryRecorder {
    fn capture(&self, event: AccountEvent) {
        self.0.borrow_mut().push(event);
    }
}

/// One page-local account attempt. Capture is best-effort and terminal calls
/// are idempotent.
pub(crate) struct WebAccountAttempt {
    recorder: Rc<dyn Recorder>,
    now: Rc<dyn Fn() -> u64>,
    started_ms: u64,
    journey: Journey,
    action: AnalyticsAction,
    surface: Surface,
    trigger: Trigger,
    account_state: AccountState,
    attempt_id: String,
    deferred_events: Vec<AccountEvent>,
    finished: bool,
}

impl WebAccountAttempt {
    /// Start and immediately record an attempt.
    pub(crate) fn start(
        action: AccountAction,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
    ) -> Self {
        Self::start_with(
            action,
            surface,
            trigger,
            account_state,
            Rc::new(PostHogRecorder),
            Rc::new(now_ms),
        )
    }

    fn start_with(
        action: AccountAction,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
        recorder: Rc<dyn Recorder>,
        now: Rc<dyn Fn() -> u64>,
    ) -> Self {
        let (journey, action, stage) = classification(action);
        let attempt_id = attempt_id();
        let started_ms = now();
        let start = AccountEvent::started(
            journey,
            action,
            stage,
            surface,
            trigger,
            account_state,
            attempt_id.clone(),
        );
        let deferred_events = if trigger == Trigger::Automatic {
            vec![start]
        } else {
            recorder.capture(start);
            Vec::new()
        };
        Self {
            recorder,
            now,
            started_ms,
            journey,
            action,
            surface,
            trigger,
            account_state,
            attempt_id,
            deferred_events,
            finished: false,
        }
    }

    /// Record a stage reached before the terminal outcome.
    pub(crate) fn checkpoint(&mut self, stage: Stage) {
        if self.finished {
            return;
        }
        let event = AccountEvent::checkpoint(
            self.journey,
            self.action,
            stage,
            self.surface,
            self.trigger,
            self.account_state,
            self.attempt_id.clone(),
        );
        if self.trigger == Trigger::Automatic {
            self.deferred_events.push(event);
        } else {
            self.recorder.capture(event);
        }
    }

    /// Record exactly one terminal outcome.
    pub(crate) fn finish(&mut self, stage: Stage, outcome: AccountOutcome) {
        if self.finished {
            return;
        }
        self.finished = true;

        if self.trigger == Trigger::Automatic {
            let suppress = AUTOMATIC_FAILURES.with(|streaks| {
                let mut streaks = streaks.borrow_mut();
                if let Some(kind) = outcome.failure_kind() {
                    !streaks.insert((self.action, kind))
                } else {
                    streaks.retain(|(action, _)| *action != self.action);
                    false
                }
            });
            if suppress {
                return;
            }
        }

        let duration = (self.now)().saturating_sub(self.started_ms);
        for event in self.deferred_events.drain(..) {
            self.recorder.capture(event);
        }
        self.recorder.capture(AccountEvent::finished(
            self.journey,
            self.action,
            stage,
            self.surface,
            self.trigger,
            self.account_state,
            self.attempt_id.clone(),
            duration,
            outcome,
        ));
    }
}

/// Record a synchronous operation whose complete lifetime is this call.
pub(crate) fn record_instant_success(
    action: AccountAction,
    surface: Surface,
    trigger: Trigger,
    account_state: AccountState,
    stage: Stage,
) {
    let mut attempt = WebAccountAttempt::start(action, surface, trigger, account_state);
    attempt.finish(stage, AccountOutcome::success());
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn now_ms() -> u64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now().max(0.0) as u64)
        .unwrap_or_default()
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn attempt_id() -> String {
    let random = rand::random::<[u8; 16]>();
    if random.iter().any(|byte| *byte != 0) {
        return hex::encode(random);
    }
    FALLBACK_ID.with(|next| {
        let mut next = next.borrow_mut();
        *next = next.saturating_add(1);
        format!("page-{next}")
    })
}

fn classification(action: AccountAction) -> (Journey, AnalyticsAction, Stage) {
    use AccountAction as Ui;
    use AnalyticsAction as Event;
    match action {
        Ui::OpenRegistration => (Journey::Onboarding, Event::OpenRegistration, Stage::Input),
        Ui::LoadAccount => (
            Journey::AccountManagement,
            Event::LoadAccount,
            Stage::AccountLoad,
        ),
        Ui::LoadRegistration => (
            Journey::Onboarding,
            Event::LoadRegistration,
            Stage::AccountLoad,
        ),
        Ui::CheckEmail => (Journey::Onboarding, Event::CheckEmail, Stage::Input),
        Ui::CreateAccount => (Journey::Onboarding, Event::CreateAccount, Stage::Input),
        Ui::LogIn => (Journey::Login, Event::Login, Stage::Input),
        Ui::AddPasskey => (Journey::Passkey, Event::AddPasskey, Stage::Input),
        Ui::ChangeDisplayName => (
            Journey::AccountManagement,
            Event::ChangeDisplayName,
            Stage::Input,
        ),
        Ui::ResendActivation => (Journey::Activation, Event::ResendActivation, Stage::Input),
        Ui::LoadDevices => (
            Journey::AccountManagement,
            Event::LoadDevices,
            Stage::AccountLoad,
        ),
        Ui::LoadProfiles => (
            Journey::AccountManagement,
            Event::LoadProfiles,
            Stage::AccountLoad,
        ),
        Ui::LinkCli => (Journey::CliHandoff, Event::LinkCli, Stage::WorkerHandoff),
        Ui::SwitchProfile => (
            Journey::AccountManagement,
            Event::SwitchProfile,
            Stage::LocalPreflight,
        ),
        Ui::SignOut => (
            Journey::AccountManagement,
            Event::SignOut,
            Stage::LocalPreflight,
        ),
        Ui::LoadDeletionPlan => (
            Journey::AccountDeletion,
            Event::LoadDeletionPlan,
            Stage::AccountLoad,
        ),
        Ui::DeleteAccount => (Journey::AccountDeletion, Event::DeleteAccount, Stage::Input),
        Ui::DeleteSpace => (Journey::AccountDeletion, Event::DeleteSpace, Stage::Input),
        Ui::RevokeDevice => (
            Journey::AccountManagement,
            Event::RevokeDevice,
            Stage::Input,
        ),
        Ui::FinishAccountBackup => (
            Journey::AccountManagement,
            Event::FinishAccountBackup,
            Stage::PasskeyAssert,
        ),
        Ui::ActivateAccount => (Journey::Activation, Event::ActivateAccount, Stage::Input),
        Ui::WatchActivation => (
            Journey::Activation,
            Event::WatchActivation,
            Stage::ActivationWait,
        ),
        Ui::SaveInitialDisplayName => (
            Journey::Onboarding,
            Event::SaveInitialDisplayName,
            Stage::Input,
        ),
        Ui::CopyInvite => (
            Journey::AccountManagement,
            Event::CopyInvite,
            Stage::CallbackDelivery,
        ),
        Ui::FinishPreviousAction => (
            Journey::AccountManagement,
            Event::FinishPreviousAction,
            Stage::LocalCommit,
        ),
        Ui::SettleAccount => (
            Journey::Activation,
            Event::SettleAccount,
            Stage::AccountSync,
        ),
        Ui::LoadAccountSpaces => (
            Journey::AccountManagement,
            Event::LoadAccountSpaces,
            Stage::AccountLoad,
        ),
        Ui::PullAccountSpace => (
            Journey::AccountManagement,
            Event::PullAccountSpace,
            Stage::AccountSync,
        ),
        Ui::OpenAccountDeletion => (
            Journey::AccountDeletion,
            Event::OpenAccountDeletion,
            Stage::Input,
        ),
        Ui::OpenSpaceDeletion => (
            Journey::AccountDeletion,
            Event::OpenSpaceDeletion,
            Stage::Input,
        ),
        Ui::SyncAccount => (
            Journey::AccountManagement,
            Event::SyncAccount,
            Stage::AccountSync,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn event_json(recorder: &MemoryRecorder) -> Vec<serde_json::Value> {
        recorder
            .0
            .borrow()
            .iter()
            .map(|event| serde_json::Value::Object(event.validated_properties().unwrap()))
            .collect()
    }

    #[test]
    fn it_records_one_start_and_one_terminal_outcome() {
        let recorder = MemoryRecorder::default();
        let time = Rc::new(Cell::new(0_u64));
        let clock: Rc<dyn Fn() -> u64> = {
            let time = time.clone();
            Rc::new(move || time.get())
        };
        let mut attempt = WebAccountAttempt::start_with(
            AccountAction::LogIn,
            Surface::Settings,
            Trigger::User,
            AccountState::None,
            Rc::new(recorder.clone()),
            clock,
        );
        attempt.checkpoint(Stage::PasskeyAssert);
        time.set(700_000);
        attempt.finish(Stage::Complete, AccountOutcome::success());
        attempt.finish(Stage::Complete, AccountOutcome::success());
        attempt.checkpoint(Stage::AccountLoad);
        let events = event_json(&recorder);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["phase"], "started");
        assert_eq!(events[2]["duration_ms"], 600_000);
    }

    #[test]
    fn it_gives_each_attempt_an_opaque_non_content_id() {
        let recorder = MemoryRecorder::default();
        let clock: Rc<dyn Fn() -> u64> = Rc::new(|| 0);
        let _first = WebAccountAttempt::start_with(
            AccountAction::LogIn,
            Surface::Settings,
            Trigger::User,
            AccountState::None,
            Rc::new(recorder.clone()),
            clock.clone(),
        );
        let _second = WebAccountAttempt::start_with(
            AccountAction::LogIn,
            Surface::Settings,
            Trigger::User,
            AccountState::None,
            Rc::new(recorder.clone()),
            clock,
        );
        let events = event_json(&recorder);
        let first = events[0]["attempt_id"].as_str().unwrap();
        let second = events[1]["attempt_id"].as_str().unwrap();
        assert_ne!(first, second);
        assert!(first.len() <= 36 && first.is_ascii());
        assert!(!first.contains("login"));
    }

    #[test]
    fn it_reports_one_automatic_failure_per_streak() {
        AUTOMATIC_FAILURES.with(|value| value.borrow_mut().clear());
        let recorder = MemoryRecorder::default();
        let clock: Rc<dyn Fn() -> u64> = Rc::new(|| 0);
        for outcome in [
            AccountOutcome::retryable(FailureKind::Network),
            AccountOutcome::retryable(FailureKind::Network),
            AccountOutcome::success(),
            AccountOutcome::retryable(FailureKind::Network),
        ] {
            let mut attempt = WebAccountAttempt::start_with(
                AccountAction::LoadAccount,
                Surface::Settings,
                Trigger::Automatic,
                AccountState::Ready,
                Rc::new(recorder.clone()),
                clock.clone(),
            );
            attempt.finish(Stage::AccountLoad, outcome);
        }
        let terminals = event_json(&recorder)
            .into_iter()
            .filter(|event| event["phase"] == "finished")
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 3);
        assert_eq!(terminals[1]["result"], "success");
        assert_eq!(event_json(&recorder).len(), 6);
    }

    #[test]
    fn automatic_attempts_flush_ordered_checkpoints_or_suppress_the_whole_attempt() {
        AUTOMATIC_FAILURES.with(|value| value.borrow_mut().clear());
        let recorder = MemoryRecorder::default();
        let clock: Rc<dyn Fn() -> u64> = Rc::new(|| 0);
        for _ in 0..2 {
            let mut attempt = WebAccountAttempt::start_with(
                AccountAction::LoadAccount,
                Surface::Settings,
                Trigger::Automatic,
                AccountState::Ready,
                Rc::new(recorder.clone()),
                clock.clone(),
            );
            attempt.checkpoint(Stage::AccountLoad);
            attempt.finish(
                Stage::AccountLoad,
                AccountOutcome::retryable(FailureKind::Network),
            );
        }
        let events = event_json(&recorder);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["phase"], "started");
        assert_eq!(events[1]["phase"], "checkpoint");
        assert_eq!(events[2]["phase"], "finished");
    }

    #[test]
    fn every_ui_action_has_a_stable_classification() {
        let actions = [
            AccountAction::OpenRegistration,
            AccountAction::LoadAccount,
            AccountAction::LoadRegistration,
            AccountAction::CheckEmail,
            AccountAction::CreateAccount,
            AccountAction::LogIn,
            AccountAction::AddPasskey,
            AccountAction::ChangeDisplayName,
            AccountAction::ResendActivation,
            AccountAction::LoadDevices,
            AccountAction::LoadProfiles,
            AccountAction::LinkCli,
            AccountAction::SwitchProfile,
            AccountAction::SignOut,
            AccountAction::LoadDeletionPlan,
            AccountAction::DeleteAccount,
            AccountAction::DeleteSpace,
            AccountAction::RevokeDevice,
            AccountAction::FinishAccountBackup,
            AccountAction::ActivateAccount,
            AccountAction::WatchActivation,
            AccountAction::SaveInitialDisplayName,
            AccountAction::CopyInvite,
            AccountAction::FinishPreviousAction,
            AccountAction::SettleAccount,
            AccountAction::LoadAccountSpaces,
            AccountAction::PullAccountSpace,
            AccountAction::OpenAccountDeletion,
            AccountAction::OpenSpaceDeletion,
            AccountAction::SyncAccount,
        ];
        for action in actions {
            let _ = classification(action);
        }
    }
}
