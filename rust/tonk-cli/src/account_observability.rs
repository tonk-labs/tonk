//! Native CLI account-attempt lifecycle.

use std::time::Instant;

use tonk_analytics::account::{
    AccountAction, AccountEvent, AccountOutcome, AccountState, DegradationKind, Journey, Stage,
    Surface, Trigger,
};

/// The narrow stage interface used by native account workflows.
pub trait CliAccountObserver {
    /// Record a native-owned stage.
    fn checkpoint(&mut self, stage: Stage);
    /// Record a non-transactional follow-up that did not complete.
    fn degraded(&mut self, kind: DegradationKind);
    /// Finish at a deep seam that has stronger outcome evidence than the
    /// top-level exit code.
    fn finish(&mut self, stage: Stage, outcome: AccountOutcome);
}

/// Observer for library callers that do not own a telemetry attempt.
#[derive(Default)]
pub struct NoopAccountObserver;

impl CliAccountObserver for NoopAccountObserver {
    fn checkpoint(&mut self, _stage: Stage) {}
    fn degraded(&mut self, _kind: DegradationKind) {}
    fn finish(&mut self, _stage: Stage, _outcome: AccountOutcome) {}
}

/// Exhaustive CLI account operation descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountCommandKind {
    /// Read account state.
    Status,
    /// Browser handoff login.
    Login,
    /// Sign out locally.
    Logout,
    /// Open account deletion review.
    Delete,
    /// List account-owned spaces.
    SpaceList,
    /// Pull one account space.
    SpacePull,
    /// Open one space's deletion review.
    SpaceDelete,
    /// Synchronize account state.
    Sync,
    /// List devices.
    Devices,
    /// Revoke a device.
    Revoke,
}

impl AccountCommandKind {
    fn classification(self) -> (Journey, AccountAction, Stage) {
        match self {
            Self::Status => (
                Journey::AccountManagement,
                AccountAction::LoadAccount,
                Stage::AccountLoad,
            ),
            Self::Login => (Journey::Login, AccountAction::Login, Stage::LocalPreflight),
            Self::Logout => (
                Journey::AccountManagement,
                AccountAction::SignOut,
                Stage::LocalPreflight,
            ),
            Self::Delete => (
                Journey::AccountDeletion,
                AccountAction::OpenAccountDeletion,
                Stage::BrowserOpen,
            ),
            Self::SpaceList => (
                Journey::AccountManagement,
                AccountAction::LoadAccountSpaces,
                Stage::AccountLoad,
            ),
            Self::SpacePull => (
                Journey::AccountManagement,
                AccountAction::PullAccountSpace,
                Stage::AccountSync,
            ),
            Self::SpaceDelete => (
                Journey::AccountDeletion,
                AccountAction::OpenSpaceDeletion,
                Stage::BrowserOpen,
            ),
            Self::Sync => (
                Journey::AccountManagement,
                AccountAction::SyncAccount,
                Stage::AccountSync,
            ),
            Self::Devices => (
                Journey::AccountManagement,
                AccountAction::LoadDevices,
                Stage::AccountLoad,
            ),
            Self::Revoke => (
                Journey::AccountManagement,
                AccountAction::RevokeDevice,
                Stage::LocalPreflight,
            ),
        }
    }
}

/// Process-local buffer for one CLI account attempt.
pub struct CliAccountAttempt {
    started: Instant,
    journey: Journey,
    action: AccountAction,
    account_state: AccountState,
    attempt_id: String,
    events: Vec<AccountEvent>,
    finished: bool,
    degradation: Option<DegradationKind>,
    last_stage: Stage,
}

impl CliAccountAttempt {
    /// Start an account command with native surface ownership.
    pub fn start(command: AccountCommandKind, account_state: AccountState) -> Self {
        let (journey, action, stage) = command.classification();
        let attempt_id = hex::encode(rand::random::<[u8; 16]>());
        Self {
            started: Instant::now(),
            journey,
            action,
            account_state,
            events: vec![AccountEvent::started(
                journey,
                action,
                stage,
                Surface::NativeCli,
                Trigger::User,
                account_state,
                attempt_id.clone(),
            )],
            attempt_id,
            finished: false,
            degradation: None,
            last_stage: stage,
        }
    }

    /// Record a native-owned stage.
    pub fn checkpoint(&mut self, stage: Stage) {
        if self.finished {
            return;
        }
        self.last_stage = stage;
        self.events.push(AccountEvent::checkpoint(
            self.journey,
            self.action,
            stage,
            Surface::NativeCli,
            Trigger::User,
            self.account_state,
            self.attempt_id.clone(),
        ));
    }

    /// Retain the earliest incomplete follow-up as the terminal degradation.
    pub fn degraded(&mut self, kind: DegradationKind) {
        if !self.finished && self.degradation.is_none() {
            self.degradation = Some(kind);
        }
    }

    /// Success outcome, preserving a previously reported degradation.
    pub fn success_outcome(&self) -> AccountOutcome {
        self.degradation
            .map(AccountOutcome::degraded)
            .unwrap_or_else(AccountOutcome::success)
    }

    /// Whether a deep seam already supplied the terminal classification.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Most recent deep stage, for classifying an error before display text.
    pub fn last_stage(&self) -> Stage {
        self.last_stage
    }

    /// Finish once. Later finishes and checkpoints are ignored.
    pub fn finish(&mut self, stage: Stage, outcome: AccountOutcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.events.push(AccountEvent::finished(
            self.journey,
            self.action,
            stage,
            Surface::NativeCli,
            Trigger::User,
            self.account_state,
            self.attempt_id.clone(),
            self.started.elapsed().as_millis() as u64,
            outcome,
        ));
    }

    /// Consume the buffered shared-schema events.
    pub fn into_events(self) -> Vec<AccountEvent> {
        self.events
    }
}

impl CliAccountObserver for CliAccountAttempt {
    fn checkpoint(&mut self, stage: Stage) {
        Self::checkpoint(self, stage);
    }

    fn degraded(&mut self, kind: DegradationKind) {
        Self::degraded(self, kind);
    }

    fn finish(&mut self, stage: Stage, outcome: AccountOutcome) {
        Self::finish(self, stage, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonk_analytics::account::{AccountResult, DegradationKind};

    #[test]
    fn it_records_native_account_start_checkpoints_and_one_terminal() {
        let mut attempt = CliAccountAttempt::start(AccountCommandKind::Login, AccountState::None);
        attempt.checkpoint(Stage::CallbackBind);
        attempt.degraded(DegradationKind::AccountSync);
        attempt.finish(Stage::AccountSync, attempt.success_outcome());
        attempt.finish(Stage::Complete, AccountOutcome::success());
        attempt.checkpoint(Stage::Complete);
        assert_eq!(attempt.last_stage(), Stage::CallbackBind);
        let events = attempt.into_events();
        assert_eq!(events.len(), 3);
        let values = events
            .iter()
            .map(|event| serde_json::Value::Object(event.validated_properties().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values[0]["surface"], "native_cli");
        assert_eq!(values[2]["result"], "degraded_success");
        assert_eq!(values[2]["degradation_kind"], "account_sync");
        let _ = AccountResult::Success;
    }

    #[test]
    fn every_account_command_maps_to_the_shared_vocabulary() {
        for command in [
            AccountCommandKind::Status,
            AccountCommandKind::Login,
            AccountCommandKind::Logout,
            AccountCommandKind::Delete,
            AccountCommandKind::SpaceList,
            AccountCommandKind::SpacePull,
            AccountCommandKind::SpaceDelete,
            AccountCommandKind::Sync,
            AccountCommandKind::Devices,
            AccountCommandKind::Revoke,
        ] {
            let _ = command.classification();
        }
    }
}
