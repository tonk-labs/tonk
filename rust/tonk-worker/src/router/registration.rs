//! Signing in, as this profile asks for it.
//!
//! The host runs the sign-in: the page that boots the kernels asks the
//! person who they are and runs the passkey. This worker asks for one, by
//! having the provisional persona ask to merge, records that it asked
//! ([`tonk_schema::SigningIn`]), and puts that away once the host merges
//! the persona or says the person did not sign in.

use std::sync::{Mutex, PoisonError};

use dialog_artifacts::Entity;
use tonk_analytics::account;
use tonk_common::log;
use tonk_schema::SigningIn;
use tonk_schema::domain::sign_in::Asked;
use tonk_schema::registration::ENTITY;
#[cfg(any(target_arch = "wasm32", test))]
use tonk_schema::registration::kind;

use super::account_journey;
#[cfg(any(target_arch = "wasm32", test))]
use super::account_journey::Attempt;
use crate::router::ClientId;
use crate::worker::TonkState;

/// The page that asked for the sign-in under way, told how it went.
static ASKED_BY: Mutex<Option<ClientId>> = Mutex::new(None);

/// Ask the host to sign the person in, for the page behind `client`.
pub(crate) async fn sign_in(tonk: &TonkState, client: Option<&ClientId>) {
    account_journey::done_at_once(
        client,
        account::AccountAction::OpenRegistration,
        account::Stage::Input,
    );
    *ASKED_BY.lock().unwrap_or_else(PoisonError::into_inner) = client.cloned();
    record(tonk, true).await;
    #[cfg(target_arch = "wasm32")]
    let asked = super::merge::ask();
    #[cfg(not(target_arch = "wasm32"))]
    let asked: Result<(), String> = Err("Only a kernel's host signs anyone in.".to_owned());
    if let Err(message) = asked {
        log!("registration: no sign-in was asked for: {message}");
        record(tonk, false).await;
    }
}

/// Record whether a sign-in is under way on the profile's overlay.
async fn record(tonk: &TonkState, asked: bool) {
    let Ok(this) = ENTITY.parse::<Entity>() else {
        return;
    };
    let branch = match tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("registration: the profile branch did not open: {error}");
            return;
        }
    };
    branch
        .state
        .retain_overlay_entities(|overlaid| overlaid != &this);
    if asked {
        let overlay = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .overlay()
            .assert(SigningIn {
                this,
                asked: Asked(now()),
            });
        if let Err(error) = overlay.write().perform(&tonk.operator).await {
            log!("registration: the sign-in was not recorded: {error}");
        }
    } else {
        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&branch.state));
    }
    // The sign-in ends with a message from the host, which nothing after
    // tells the views of, so this does.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

fn now() -> f64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as f64)
        .unwrap_or_default()
}

/// The sign-in the passkey ceremony `kind` ran is over: the persona
/// merged (`Ok`, `true` when the account it reached is served), or the
/// custody service refused it with `code`.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn settle(
    tonk: &TonkState,
    kind: &'static str,
    outcome: Result<bool, Option<&str>>,
) {
    let client = ASKED_BY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let mut attempt = Attempt::begin(
        client.as_ref(),
        account_journey::action_of(kind),
        account::Trigger::User,
    );
    // The host's panel looks the address up before it asks for a passkey.
    attempt.reached(account::Stage::EmailLookup);
    attempt.reached(account_journey::passkey_stage(kind));
    let (ended, told) = account_journey::handed_off(outcome);
    attempt.end(ended, told);
    if outcome.is_ok() && kind == kind::CREATE {
        crate::router::navigate::notify_analytics(
            client.as_ref(),
            tonk_worker_api::AnalyticsEvent::AccountCreated,
        );
    }
    // An account that waits for its emailed link is not done signing in:
    // what waits on the sign-in, such as a space's bar, goes on once the
    // account is served (see [`activated`]).
    let awaiting = matches!(outcome, Ok(false) | Err(Some("awaiting-activation")));
    if !awaiting {
        record(tonk, false).await;
    }
}

/// The account is served: a sign-in that waited for its emailed link is
/// over.
pub(crate) async fn activated(tonk: &TonkState) {
    record(tonk, false).await;
}

/// The person did not sign in: the browser refused the passkey ceremony
/// `kind`, as it names the refusal, `name`, or they put the host's panel
/// away.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) async fn refused(tonk: &TonkState, kind: &str, name: &str) {
    let client = ASKED_BY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    log!("registration: the {kind} sign-in was not finished ({name})");
    let attempt = Attempt::begin(
        client.as_ref(),
        account_journey::action_of(kind),
        account::Trigger::User,
    );
    let (ended, told) = account_journey::refused(name);
    attempt.end(ended, told);
    record(tonk, false).await;
}

/// Run `account/sign-in`.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::SignIn> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::SignIn) {
        let tonk = self.state().read().await;
        sign_in(&tonk, self.client()).await;
    }
}

#[cfg(test)]
mod tests {
    use dialog_capability::Provider;
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::command::SignIn;
    use tonk_schema::domain::command::current::sign_in;

    use super::*;
    use crate::router::{CommandEnv, CommandOrigin};

    async fn env() -> CommandEnv {
        let state = crate::router::command::tests::native::test_state().await;
        CommandEnv::new(state, CommandOrigin::default())
    }

    async fn signing_in(tonk: &TonkState) -> bool {
        let this: Entity = ENTITY.parse().unwrap();
        let branch = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        branch
            .handle()
            .query()
            .select(Query::<SigningIn> {
                this: Term::from(this),
                asked: Term::var("asked"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .is_ok_and(|rows| !rows.is_empty())
    }

    /// A worker no host signs anyone in for does not leave a sign-in
    /// standing that nothing will end.
    #[dialog_common::test]
    async fn it_leaves_no_sign_in_standing_where_none_could_be_asked_for() {
        let env = env().await;
        Provider::<SignIn>::execute(
            &env,
            SignIn {
                this: "cmd:sign-in".parse().expect("entity"),
                time: sign_in::Time(1.0),
            },
        )
        .await;
        let tonk = env.state().read().await;
        assert!(!signing_in(&tonk).await);
    }

    /// A sign-in the person did not finish is put away.
    #[dialog_common::test]
    async fn it_puts_away_a_sign_in_the_person_did_not_finish() {
        let env = env().await;
        let tonk = env.state().read().await;
        record(&tonk, true).await;
        assert!(signing_in(&tonk).await, "the sign-in is recorded");
        refused(&tonk, kind::LOG_IN, "AbortError").await;
        assert!(!signing_in(&tonk).await, "and put away");
    }
}
