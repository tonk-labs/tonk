//! The account attempts this worker runs, told to the page that captures
//! them.
//!
//! Adding an account, logging in and confirming an address are the
//! worker's to run: the panel asserts a command and reads back a stage. So
//! the worker is where an attempt starts and ends, and it says so in the
//! account journey's closed vocabulary (see [`tonk_analytics::account`]).
//! Each event goes to the page that asked, over the message the worker
//! already tells it of a created or joined space with; the page validates
//! and captures it. Nothing here names an account, an address or a space.

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
use tonk_analytics::account::FailureKind;
use tonk_analytics::account::{
    AccountAction, AccountEvent, AccountOutcome, AccountState, Journey, Stage, Surface, Trigger,
};
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
use tonk_identity::custody::CustodyDenial;
use tonk_schema::registration::kind;

use super::ClientId;
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
use super::registration::Stage as Panel;

/// One attempt at an account action, from the command that began it to
/// the one outcome it ends in.
pub(crate) struct Attempt {
    action: AccountAction,
    id: String,
    began: web_time::Instant,
}

/// How an action is grouped, where it runs, and what is known of the
/// account when it begins.
fn classify(action: AccountAction) -> (Journey, Surface, AccountState) {
    match action {
        AccountAction::Login => (
            Journey::Login,
            Surface::RegistrationDialog,
            AccountState::Unknown,
        ),
        AccountAction::ActivateAccount => (
            Journey::Activation,
            Surface::ActivationPage,
            AccountState::PendingActivation,
        ),
        _ => (
            Journey::Onboarding,
            Surface::RegistrationDialog,
            AccountState::None,
        ),
    }
}

impl Attempt {
    /// Begin an attempt at `action`, with the event that says it began.
    pub(crate) fn begin(action: AccountAction) -> (Self, AccountEvent) {
        let id = hex::encode(rand::random::<[u8; 16]>());
        let (journey, surface, state) = classify(action);
        let began = AccountEvent::started(
            journey,
            action,
            Stage::Input,
            surface,
            Trigger::User,
            state,
            id.clone(),
        );
        (
            Self {
                action,
                id,
                began: web_time::Instant::now(),
            },
            began,
        )
    }

    /// The action this is an attempt at.
    pub(crate) fn action(&self) -> AccountAction {
        self.action
    }

    /// The attempt reached `stage`.
    pub(crate) fn reached(&self, stage: Stage) -> AccountEvent {
        let (journey, surface, state) = classify(self.action);
        AccountEvent::checkpoint(
            journey,
            self.action,
            stage,
            surface,
            Trigger::User,
            state,
            self.id.clone(),
        )
    }

    /// The attempt ended at `stage`, with `outcome`.
    pub(crate) fn ended(self, stage: Stage, outcome: AccountOutcome) -> AccountEvent {
        let (journey, surface, state) = classify(self.action);
        let lasted = self.began.elapsed().as_millis();
        AccountEvent::finished(
            journey,
            self.action,
            stage,
            surface,
            Trigger::User,
            state,
            self.id,
            u64::try_from(lasted).unwrap_or(u64::MAX),
            outcome,
        )
    }
}

/// The action a passkey ceremony of `kind` is an attempt at.
pub(crate) fn action_of(kind: &str) -> AccountAction {
    match kind {
        kind::CREATE => AccountAction::CreateAccount,
        _ => AccountAction::Login,
    }
}

/// The stage at which a ceremony of `kind` waits on the passkey.
pub(crate) fn passkey_stage(kind: &str) -> Stage {
    match kind {
        kind::CREATE => Stage::PasskeyCreate,
        _ => Stage::PasskeyAssert,
    }
}

/// How a passkey hand-off for `kind` ended, from where it left the panel:
/// put away, waiting for the emailed link, or at a failure the service's
/// `code` may explain.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn handed_off(
    kind: &str,
    panel: Option<&Panel>,
    code: Option<&str>,
) -> (Stage, AccountOutcome) {
    match panel {
        None => (Stage::Complete, AccountOutcome::success()),
        Some(Panel::Failed { .. }) => {
            let outcome = match code.and_then(|code| CustodyDenial::from_code(code, "")) {
                Some(CustodyDenial::Suspended(_)) => {
                    AccountOutcome::blocked(FailureKind::Suspended)
                }
                Some(CustodyDenial::NotProvisioned(_)) => {
                    AccountOutcome::blocked(FailureKind::NotProvisioned)
                }
                _ => AccountOutcome::retryable(FailureKind::Unknown),
            };
            (passkey_stage(kind), outcome)
        }
        Some(_) => (
            Stage::ActivationWait,
            AccountOutcome::blocked(FailureKind::AwaitingActivation),
        ),
    }
}

/// How a passkey ceremony the page refused ended: `name` is the browser's
/// name for the refusal.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn refused(kind: &str, name: &str) -> (Stage, AccountOutcome) {
    let outcome = match name {
        "NotAllowedError" => AccountOutcome::cancelled(),
        "InvalidStateError" => AccountOutcome::terminal_failure(FailureKind::CredentialExists),
        "NotSupportedError" => AccountOutcome::terminal_failure(FailureKind::PasskeyUnsupported),
        "SecurityError" => AccountOutcome::terminal_failure(FailureKind::SecurityContext),
        _ => AccountOutcome::retryable(FailureKind::Unknown),
    };
    (passkey_stage(kind), outcome)
}

/// Tell the page behind `client` of `event`.
pub(crate) fn tell(client: Option<&ClientId>, event: AccountEvent) {
    super::navigate::notify_analytics(client, tonk_worker_api::AnalyticsEvent::Account { event });
}

/// An action whose whole life is the command that ran it.
pub(crate) fn done_at_once(client: Option<&ClientId>, action: AccountAction) {
    let (attempt, began) = Attempt::begin(action);
    tell(client, began);
    tell(
        client,
        attempt.ended(Stage::Input, AccountOutcome::success()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(event: &AccountEvent) -> serde_json::Map<String, serde_json::Value> {
        event.validated_properties().expect("the event is valid")
    }

    #[dialog_common::test]
    fn it_tells_an_attempt_from_its_start_to_its_one_end() {
        let (attempt, began) = Attempt::begin(AccountAction::CreateAccount);
        let waiting = attempt.reached(Stage::PasskeyCreate);
        let (stage, outcome) = handed_off(
            kind::CREATE,
            Some(&Panel::Confirming { kind: kind::CREATE }),
            None,
        );
        let ended = attempt.ended(stage, outcome);

        let (began, waiting, ended) = (read(&began), read(&waiting), read(&ended));
        assert_eq!(
            (&began["action"], &began["phase"], &began["stage"]),
            (&"create_account".into(), &"started".into(), &"input".into())
        );
        assert_eq!(began["journey"], "onboarding");
        assert_eq!(
            (&waiting["phase"], &waiting["stage"]),
            (&"checkpoint".into(), &"passkey_create".into())
        );
        assert_eq!(
            (&ended["phase"], &ended["stage"], &ended["result"]),
            (
                &"finished".into(),
                &"activation_wait".into(),
                &"blocked".into()
            )
        );
        assert_eq!(ended["failure_kind"], "awaiting_activation");
        assert_eq!(began["attempt_id"], waiting["attempt_id"]);
        assert_eq!(began["attempt_id"], ended["attempt_id"]);
    }

    #[dialog_common::test]
    fn it_groups_a_log_in_and_an_activation_apart_from_signing_up() {
        let (_, log_in) = Attempt::begin(action_of(kind::LOG_IN));
        let log_in = read(&log_in);
        assert_eq!(
            (
                &log_in["action"],
                &log_in["journey"],
                &log_in["account_state"]
            ),
            (&"login".into(), &"login".into(), &"unknown".into())
        );

        let (_, activation) = Attempt::begin(AccountAction::ActivateAccount);
        let activation = read(&activation);
        assert_eq!(
            (&activation["journey"], &activation["surface"]),
            (&"activation".into(), &"activation_page".into())
        );
    }

    #[dialog_common::test]
    fn it_reads_how_a_hand_off_ended_off_where_it_left_the_panel() {
        assert_eq!(
            handed_off(kind::LOG_IN, None, None),
            (Stage::Complete, AccountOutcome::success())
        );
        let failed = Panel::Failed {
            kind: kind::LOG_IN,
            message: "no".into(),
        };
        assert_eq!(
            handed_off(kind::LOG_IN, Some(&failed), None),
            (
                Stage::PasskeyAssert,
                AccountOutcome::retryable(FailureKind::Unknown)
            )
        );
    }

    #[dialog_common::test]
    fn it_tells_a_closed_prompt_from_a_browser_that_cannot() {
        assert_eq!(
            refused(kind::CREATE, "NotAllowedError"),
            (Stage::PasskeyCreate, AccountOutcome::cancelled())
        );
        assert_eq!(
            refused(kind::CREATE, "NotSupportedError"),
            (
                Stage::PasskeyCreate,
                AccountOutcome::terminal_failure(FailureKind::PasskeyUnsupported)
            )
        );
        assert_eq!(
            refused(kind::LOG_IN, "anything else"),
            (
                Stage::PasskeyAssert,
                AccountOutcome::retryable(FailureKind::Unknown)
            )
        );
    }
}
