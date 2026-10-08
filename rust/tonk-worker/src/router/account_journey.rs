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

use std::cell::RefCell;
use std::collections::HashSet;

use tonk_analytics::account::{
    AccountAction, AccountEvent, AccountOutcome, AccountState, FailureKind, Journey, Stage,
    Surface, Trigger,
};
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
use tonk_identity::custody::CustodyDenial;
use tonk_schema::registration::kind;

use super::ClientId;
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
use super::registration::Stage as Panel;

/// One attempt at an account action, from what began it to the one outcome
/// it ends in, and the page that is told how it goes.
pub(crate) struct Attempt {
    action: AccountAction,
    trigger: Trigger,
    id: String,
    began: web_time::Instant,
    client: Option<ClientId>,
    /// What an attempt nobody asked for has to say, kept until it ends: one
    /// that never ends says nothing, and one that fails the way the last
    /// did says nothing again.
    held: Vec<AccountEvent>,
}

/// How an action is grouped, the stage it begins at, where it runs, and
/// what is known of the account when it begins.
fn classify(action: AccountAction) -> (Journey, Stage, Surface, AccountState) {
    use AccountAction as Action;
    let dialog = Surface::RegistrationDialog;
    match action {
        Action::OpenRegistration => (
            Journey::Onboarding,
            Stage::Input,
            dialog,
            AccountState::Unknown,
        ),
        Action::LoadRegistration => (
            Journey::Onboarding,
            Stage::AccountLoad,
            dialog,
            AccountState::Unknown,
        ),
        Action::CheckEmail | Action::CreateAccount => (
            Journey::Onboarding,
            Stage::Input,
            dialog,
            AccountState::None,
        ),
        Action::SaveInitialDisplayName => (
            Journey::Onboarding,
            Stage::Input,
            dialog,
            AccountState::Ready,
        ),
        Action::Login => (Journey::Login, Stage::Input, dialog, AccountState::Unknown),
        Action::WatchActivation => (
            Journey::Activation,
            Stage::ActivationWait,
            dialog,
            AccountState::PendingActivation,
        ),
        Action::ActivateAccount => (
            Journey::Activation,
            Stage::Input,
            Surface::ActivationPage,
            AccountState::PendingActivation,
        ),
        _ => (
            Journey::AccountManagement,
            Stage::Input,
            Surface::Settings,
            AccountState::Unknown,
        ),
    }
}

thread_local! {
    /// The failures attempts nobody asked for last ended in, by action. One
    /// that fails the same way again is not told; a success clears them.
    static REPEATED: RefCell<HashSet<(AccountAction, FailureKind)>> = RefCell::new(HashSet::new());
}

impl Attempt {
    /// Begin an attempt at `action`, for the page behind `client`.
    pub(crate) fn begin(
        client: Option<&ClientId>,
        action: AccountAction,
        trigger: Trigger,
    ) -> Self {
        let id = hex::encode(rand::random::<[u8; 16]>());
        let (journey, stage, surface, state) = classify(action);
        let began =
            AccountEvent::started(journey, action, stage, surface, trigger, state, id.clone());
        let mut attempt = Self {
            action,
            trigger,
            id,
            began: web_time::Instant::now(),
            client: client.cloned(),
            held: Vec::new(),
        };
        attempt.say(began);
        attempt
    }

    /// The page this attempt is told to.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn client(&self) -> Option<&ClientId> {
        self.client.as_ref()
    }

    fn say(&mut self, event: AccountEvent) {
        if self.trigger == Trigger::Automatic {
            self.held.push(event);
        } else {
            tell(self.client.as_ref(), event);
        }
    }

    /// The attempt reached `stage`.
    pub(crate) fn reached(&mut self, stage: Stage) {
        let (journey, _, surface, state) = classify(self.action);
        let reached = AccountEvent::checkpoint(
            journey,
            self.action,
            stage,
            surface,
            self.trigger,
            state,
            self.id.clone(),
        );
        self.say(reached);
    }

    /// The attempt ended at `stage`, with `outcome`.
    pub(crate) fn end(mut self, stage: Stage, outcome: AccountOutcome) {
        if self.trigger == Trigger::Automatic {
            let action = self.action;
            let repeated = REPEATED.with_borrow_mut(|repeated| match outcome.failure_kind() {
                Some(kind) => !repeated.insert((action, kind)),
                None => {
                    repeated.retain(|(failed, _)| *failed != action);
                    false
                }
            });
            if repeated {
                return;
            }
        }
        let (journey, _, surface, state) = classify(self.action);
        let lasted = self.began.elapsed().as_millis();
        for held in std::mem::take(&mut self.held) {
            tell(self.client.as_ref(), held);
        }
        tell(
            self.client.as_ref(),
            AccountEvent::finished(
                journey,
                self.action,
                stage,
                surface,
                self.trigger,
                state,
                self.id,
                u64::try_from(lasted).unwrap_or(u64::MAX),
                outcome,
            ),
        );
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

/// How a passkey hand-off ended, from where it left the panel: put away,
/// waiting for the emailed link, or at a failure the service's `code` may
/// explain. A hand-off that failed is told at the passkey assertion,
/// whichever ceremony it was.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn handed_off(panel: Option<&Panel>, code: Option<&str>) -> (Stage, AccountOutcome) {
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
                Some(CustodyDenial::Other(_)) => {
                    AccountOutcome::terminal_failure(FailureKind::AccessDenied)
                }
                _ => AccountOutcome::retryable(FailureKind::Unknown),
            };
            (Stage::PasskeyAssert, outcome)
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
pub(crate) fn refused(name: &str) -> (Stage, AccountOutcome) {
    let outcome = match name {
        "NotAllowedError" => AccountOutcome::cancelled(),
        "InvalidStateError" => AccountOutcome::terminal_failure(FailureKind::CredentialExists),
        "NotSupportedError" => AccountOutcome::terminal_failure(FailureKind::PasskeyUnsupported),
        "SecurityError" => AccountOutcome::terminal_failure(FailureKind::SecurityContext),
        "NoPrfError" => AccountOutcome::terminal_failure(FailureKind::PrfUnsupported),
        _ => AccountOutcome::retryable(FailureKind::Unknown),
    };
    (Stage::PasskeyAssert, outcome)
}

#[cfg(test)]
thread_local! {
    /// What a test hears in place of a page.
    static HEARD: RefCell<Option<Vec<AccountEvent>>> = const { RefCell::new(None) };
}

/// Hear what is told from here on, in place of a page.
#[cfg(test)]
pub(crate) fn listen() {
    HEARD.set(Some(Vec::new()));
    REPEATED.with_borrow_mut(HashSet::clear);
}

/// What was told since [`listen`], as the properties a page would capture.
#[cfg(test)]
pub(crate) fn heard() -> Vec<serde_json::Map<String, serde_json::Value>> {
    HEARD
        .take()
        .unwrap_or_default()
        .iter()
        .map(|event| event.validated_properties().expect("a valid event"))
        .collect()
}

/// Tell the page behind `client` of `event`.
pub(crate) fn tell(client: Option<&ClientId>, event: AccountEvent) {
    #[cfg(test)]
    if HEARD
        .with_borrow_mut(|heard| heard.as_mut().map(|heard| heard.push(event.clone())))
        .is_some()
    {
        return;
    }
    super::navigate::notify_analytics(client, tonk_worker_api::AnalyticsEvent::Account { event });
}

/// An action whose whole life is the command that ran it, ending at
/// `stage`.
pub(crate) fn done_at_once(client: Option<&ClientId>, action: AccountAction, stage: Stage) {
    Attempt::begin(client, action, Trigger::User).end(stage, AccountOutcome::success());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts of each heard event a funnel is built on.
    fn steps() -> Vec<String> {
        heard()
            .iter()
            .map(|event| {
                let part = |key: &str| {
                    event
                        .get(key)
                        .and_then(|value| value.as_str())
                        .unwrap_or("-")
                };
                format!(
                    "{} {} {} {} {}",
                    part("action"),
                    part("phase"),
                    part("stage"),
                    part("result"),
                    part("failure_kind")
                )
            })
            .collect()
    }

    #[dialog_common::test]
    fn it_tells_an_attempt_from_its_start_to_its_one_end() {
        listen();
        let mut attempt = Attempt::begin(None, AccountAction::CreateAccount, Trigger::User);
        attempt.reached(Stage::EmailLookup);
        attempt.reached(Stage::PasskeyCreate);
        let (stage, outcome) = handed_off(Some(&Panel::Confirming { kind: kind::CREATE }), None);
        attempt.end(stage, outcome);

        let told = heard();
        assert_eq!(told.len(), 4);
        assert!(
            told.iter()
                .all(|event| event["attempt_id"] == told[0]["attempt_id"])
        );
        assert_eq!(told[0]["journey"], "onboarding");
        assert_eq!(told[0]["account_state"], "none");
        assert_eq!(told[0]["trigger"], "user");
    }

    #[dialog_common::test]
    fn it_tells_signing_up_as_the_page_used_to() {
        listen();
        done_at_once(None, AccountAction::OpenRegistration, Stage::Input);
        let mut attempt = Attempt::begin(None, AccountAction::CreateAccount, Trigger::User);
        attempt.reached(Stage::EmailLookup);
        attempt.reached(passkey_stage(kind::CREATE));
        let (stage, outcome) = handed_off(Some(&Panel::Confirming { kind: kind::CREATE }), None);
        attempt.end(stage, outcome);

        assert_eq!(
            steps(),
            [
                "open_registration started input - -",
                "open_registration finished input success -",
                "create_account started input - -",
                "create_account checkpoint email_lookup - -",
                "create_account checkpoint passkey_create - -",
                "create_account finished activation_wait blocked awaiting_activation",
            ]
        );
    }

    #[dialog_common::test]
    fn it_groups_each_action_as_the_page_did() {
        let of = |action| {
            listen();
            drop(Attempt::begin(None, action, Trigger::User));
            let began = heard().remove(0);
            let part = |key: &str| began[key].as_str().unwrap_or_default().to_owned();
            (
                part("journey"),
                part("stage"),
                part("surface"),
                part("account_state"),
            )
        };
        let dialog = "registration_dialog";
        let row = |journey: &str, stage: &str, surface: &str, state: &str| {
            (
                journey.to_owned(),
                stage.to_owned(),
                surface.to_owned(),
                state.to_owned(),
            )
        };
        assert_eq!(
            of(AccountAction::OpenRegistration),
            row("onboarding", "input", dialog, "unknown")
        );
        assert_eq!(
            of(AccountAction::LoadRegistration),
            row("onboarding", "account_load", dialog, "unknown")
        );
        assert_eq!(
            of(AccountAction::CheckEmail),
            row("onboarding", "input", dialog, "none")
        );
        assert_eq!(
            of(AccountAction::CreateAccount),
            row("onboarding", "input", dialog, "none")
        );
        assert_eq!(
            of(AccountAction::Login),
            row("login", "input", dialog, "unknown")
        );
        assert_eq!(
            of(AccountAction::WatchActivation),
            row(
                "activation",
                "activation_wait",
                dialog,
                "pending_activation"
            )
        );
        assert_eq!(
            of(AccountAction::SaveInitialDisplayName),
            row("onboarding", "input", dialog, "ready")
        );
        assert_eq!(
            of(AccountAction::ActivateAccount),
            row(
                "activation",
                "input",
                "activation_page",
                "pending_activation"
            )
        );
    }

    #[dialog_common::test]
    fn it_says_nothing_of_an_attempt_nobody_asked_for_until_it_ends() {
        listen();
        let waiting = Attempt::begin(None, AccountAction::WatchActivation, Trigger::Automatic);
        assert!(steps().is_empty(), "nothing is told while it waits");

        listen();
        waiting.end(Stage::Complete, AccountOutcome::success());
        assert_eq!(
            steps(),
            [
                "watch_activation started activation_wait - -",
                "watch_activation finished complete success -",
            ]
        );

        // One that never ends says nothing at all.
        listen();
        drop(Attempt::begin(
            None,
            AccountAction::WatchActivation,
            Trigger::Automatic,
        ));
        assert!(steps().is_empty());
    }

    #[dialog_common::test]
    fn it_tells_a_repeated_failure_nobody_asked_for_once() {
        listen();
        let cancel = || {
            Attempt::begin(None, AccountAction::LoadRegistration, Trigger::Automatic)
                .end(Stage::Complete, AccountOutcome::cancelled());
        };
        cancel();
        cancel();
        assert_eq!(
            steps().len(),
            2,
            "the first is told, start and end; the second is not"
        );

        // A success clears it, and the next failure is told again.
        HEARD.set(Some(Vec::new()));
        Attempt::begin(None, AccountAction::LoadRegistration, Trigger::Automatic)
            .end(Stage::AccountLoad, AccountOutcome::success());
        cancel();
        assert_eq!(steps().len(), 4);

        // One a person asked for is told every time.
        listen();
        for _ in 0..2 {
            Attempt::begin(None, AccountAction::CheckEmail, Trigger::User).end(
                Stage::EmailLookup,
                AccountOutcome::retryable(FailureKind::Unknown),
            );
        }
        assert_eq!(steps().len(), 4);
    }

    #[dialog_common::test]
    fn it_reads_how_a_hand_off_ended_off_where_it_left_the_panel() {
        assert_eq!(
            handed_off(None, None),
            (Stage::Complete, AccountOutcome::success())
        );
        let failed = Panel::Failed {
            kind: kind::CREATE,
            message: "no".into(),
        };
        assert_eq!(
            handed_off(Some(&failed), None),
            (
                Stage::PasskeyAssert,
                AccountOutcome::retryable(FailureKind::Unknown)
            ),
            "a failed creation is told at the assertion too, as it was"
        );
        assert_eq!(
            handed_off(Some(&failed), Some("suspended")).1,
            AccountOutcome::blocked(FailureKind::Suspended)
        );
        assert_eq!(
            handed_off(Some(&failed), Some("denied")).1,
            AccountOutcome::terminal_failure(FailureKind::AccessDenied)
        );
    }

    #[dialog_common::test]
    fn it_tells_a_closed_prompt_from_a_browser_that_cannot() {
        assert_eq!(
            refused("NotAllowedError"),
            (Stage::PasskeyAssert, AccountOutcome::cancelled())
        );
        assert_eq!(
            refused("NotSupportedError").1,
            AccountOutcome::terminal_failure(FailureKind::PasskeyUnsupported)
        );
        assert_eq!(
            refused("NoPrfError").1,
            AccountOutcome::terminal_failure(FailureKind::PrfUnsupported)
        );
        assert_eq!(
            refused("anything else").1,
            AccountOutcome::retryable(FailureKind::Unknown)
        );
    }
}
