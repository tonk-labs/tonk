//! The panel that adds an account to this profile.
//!
//! The panel is views of the stages in [`tonk_schema::registration`], and
//! this is what moves it from one to the next: the commands its views
//! assert, and the custody hand-off that ends a passkey ceremony. The
//! ceremony itself runs in the top-level page, the one place WebAuthn
//! can, which the worker asks for it; everything else is decided here.

use std::cell::{Cell, RefCell};

use dialog_artifacts::Entity;
use tonk_analytics::account::{self, AccountOutcome};
use tonk_common::log;
use tonk_schema::registration::{
    ENTITY, RegistrationAddress, RegistrationCeremony, RegistrationConfirming, RegistrationFailed,
    RegistrationNaming, RegistrationVia, kind,
};

use super::account_journey::{self, Attempt};
use crate::worker::TonkState;
use tonk_schema::email_state;

/// The stage the panel is at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stage {
    /// See [`RegistrationAddress`].
    Address {
        /// The address typed so far.
        email: String,
    },
    /// See [`RegistrationVia`].
    Via {
        /// The address typed so far.
        origin: String,
    },
    /// See [`RegistrationNaming`].
    Naming {
        /// The address the account will have.
        email: String,
    },
    /// See [`RegistrationCeremony`].
    Ceremony {
        /// One of [`kind`].
        kind: &'static str,
    },
    #[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
    /// See [`RegistrationConfirming`].
    Confirming {
        /// One of [`kind`].
        kind: &'static str,
    },
    /// See [`RegistrationFailed`].
    Failed {
        /// One of [`kind`].
        kind: &'static str,
        /// What went wrong.
        message: String,
    },
}

thread_local! {
    /// Whether a passkey ceremony is out with the page. Claimed before the
    /// first await, so two commands in one turn cannot both ask for one;
    /// recording any other stage gives it up.
    static ASKING: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    /// The attempt the passkey ceremony now out is part of.
    static CEREMONY: RefCell<Option<Attempt>> = const { RefCell::new(None) };
    /// The panel's wait for its first answer about an address, begun when
    /// it opens.
    static LOADING: RefCell<Option<Attempt>> = const { RefCell::new(None) };
    /// The panel's wait for the emailed link, begun when a ceremony leaves
    /// an account that is not yet served.
    static WATCHING: RefCell<Option<Attempt>> = const { RefCell::new(None) };
}

/// The panel opened for the page behind `client`: told as an action done at
/// once, and the wait for its first answer begins.
pub(crate) fn opened(client: Option<&crate::router::ClientId>) {
    account_journey::done_at_once(
        client,
        account::AccountAction::OpenRegistration,
        account::Stage::Input,
    );
    LOADING.set(Some(Attempt::begin(
        client,
        account::AccountAction::LoadRegistration,
        account::Trigger::Automatic,
    )));
}

/// The panel begins its wait for the emailed link, for the page behind
/// `client`.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn watch(client: Option<&crate::router::ClientId>) {
    WATCHING.set(Some(Attempt::begin(
        client,
        account::AccountAction::WatchActivation,
        account::Trigger::Automatic,
    )));
}

/// The panel has its first answer about an address.
pub(crate) fn answered() {
    if let Some(loading) = LOADING.take() {
        loading.end(account::Stage::AccountLoad, AccountOutcome::success());
    }
}

/// Claim the one passkey ceremony the panel may have out, or `false` when
/// it already has one.
fn claim_ceremony() -> bool {
    !ASKING.replace(true)
}

/// What a typed address leads to, given what the lookup said about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    /// The address is free: name the account to create.
    Name,
    /// An account has the address: log in to it.
    LogIn,
    /// Neither is on offer (an invalid or suspended address, or a service
    /// that did not answer). The panel stays at the address, where the
    /// lookup's own answer says why.
    Stay,
}

/// What `answer`, the lookup's word for an address, leads to.
pub(crate) fn next(answer: &str) -> Next {
    match answer {
        email_state::UNREGISTERED => Next::Name,
        email_state::ACTIVE | email_state::PENDING => Next::LogIn,
        _ => Next::Stay,
    }
}

/// Record `stage` as where the panel is, in place of whichever stage was
/// there. `None` puts the panel away.
pub(crate) async fn record(tonk: &TonkState, stage: Option<Stage>) {
    ASKING.set(matches!(stage, Some(Stage::Ceremony { .. })));
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
    let overlay = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .overlay();
    let overlay = match stage {
        None => {
            tonk.reactor
                .schedule_poll(std::sync::Arc::clone(&branch.state));
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            return;
        }
        Some(Stage::Address { email }) => overlay.assert(RegistrationAddress {
            this,
            email: tonk_schema::domain::registration::address::Email(email),
        }),
        Some(Stage::Via { origin }) => overlay.assert(RegistrationVia {
            this,
            origin: tonk_schema::domain::registration::via::Origin(origin),
        }),
        Some(Stage::Naming { email }) => overlay.assert(RegistrationNaming {
            this,
            email: tonk_schema::domain::registration::naming::Email(email),
        }),
        Some(Stage::Ceremony { kind }) => overlay.assert(RegistrationCeremony {
            this,
            kind: tonk_schema::domain::registration::ceremony::Kind(kind.to_owned()),
        }),
        #[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
        Some(Stage::Confirming { kind }) => overlay.assert(RegistrationConfirming {
            this,
            kind: tonk_schema::domain::registration::confirming::Kind(kind.to_owned()),
        }),
        Some(Stage::Failed { kind, message }) => overlay.assert(RegistrationFailed {
            this,
            kind: tonk_schema::domain::registration::failed::Kind(kind.to_owned()),
            message: tonk_schema::domain::registration::failed::Message(message),
        }),
    };
    if let Err(error) = overlay.write().perform(&tonk.operator).await {
        log!("registration: the stage was not recorded: {error}");
    }
    // A stage is recorded from more than a command: a passkey's result and
    // a refusal arrive as messages, and activation from a sync. Nothing
    // after those tells the panel, so this does.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Whether the panel waits for an account's emailed link.
pub(crate) async fn confirming(tonk: &TonkState) -> bool {
    use dialog_query::{Output as _, Query, Term};

    let Ok(this) = ENTITY.parse::<Entity>() else {
        return false;
    };
    let Ok(branch) = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
    else {
        return false;
    };
    branch
        .handle()
        .query()
        .select(Query::<RegistrationConfirming> {
            this: Term::from(this),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .is_ok_and(|rows| !rows.is_empty())
}

/// The account's address is confirmed: a panel waiting for that is done.
pub(crate) async fn activated(tonk: &TonkState) {
    if confirming(tonk).await {
        if let Some(watching) = WATCHING.take() {
            watching.end(account::Stage::Complete, AccountOutcome::success());
        }
        record(tonk, None).await;
    }
}

/// What a refused custody hand-off says to the person, and whether it is
/// a wait for the emailed link rather than a failure.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn refusal(code: Option<&str>, message: &str) -> Result<(), String> {
    use tonk_identity::custody::CustodyDenial;

    match code.and_then(|code| CustodyDenial::from_code(code, message)) {
        Some(CustodyDenial::AwaitingActivation) => Ok(()),
        Some(CustodyDenial::Suspended(_)) => Err(
            "This account is suspended, so it cannot be used on this device. Contact support to restore it."
                .to_owned(),
        ),
        Some(CustodyDenial::NotProvisioned(_)) => Err(
            "This account is not set up for syncing yet. Finish creating it on the browser that holds its passkey, then try again."
                .to_owned(),
        ),
        Some(CustodyDenial::Other(_)) | None => Err(message.to_owned()),
    }
}

/// What a passkey ceremony the page refused says to the person: the
/// browser's `name` for the refusal.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn refused(name: &str) -> String {
    match name {
        "NotAllowedError" => "The passkey prompt was closed. Try again when you are ready.",
        "InvalidStateError" => {
            "This device already has a passkey for that account. Log in with it instead."
        }
        "NotSupportedError" => "This browser cannot use passkeys.",
        "SecurityError" => "Passkeys are not available on this page.",
        _ => "The passkey did not finish. Try again.",
    }
    .to_owned()
}

/// The stage a finished custody hand-off for `kind` leads to. `active` is
/// whether the account it reached is served.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn settled(
    kind: &'static str,
    outcome: Result<(), (Option<&str>, &str)>,
    active: bool,
) -> Option<Stage> {
    match outcome {
        Ok(()) if active => None,
        Ok(()) => Some(Stage::Confirming { kind }),
        Err((code, message)) => match refusal(code, message) {
            Ok(()) => Some(Stage::Confirming { kind }),
            Err(message) => Some(Stage::Failed { kind, message }),
        },
    }
}

/// The ceremony a custody hand-off ran, when it is one the panel asked
/// for.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn kind_of(intent: &tonk_worker_api::CustodyIntent) -> Option<&'static str> {
    match intent {
        tonk_worker_api::CustodyIntent::CreateAccount(_) => Some(kind::CREATE),
        tonk_worker_api::CustodyIntent::Login(_) => Some(kind::LOG_IN),
        _ => None,
    }
}

/// The ceremony a custody hand-off's `data` asks for, when it is one the
/// panel asked for.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn kind_in(data: &wasm_bindgen::JsValue) -> Option<&'static str> {
    let request = js_sys::Reflect::get(data, &"request".into()).ok()?;
    let intent: tonk_worker_api::CustodyIntent = serde_wasm_bindgen::from_value(request).ok()?;
    kind_of(&intent)
}

/// Whether the account this profile holds is served, as the service says
/// now: right after a login the account's own facts have not synced here
/// yet, so they cannot say.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn served(tonk: &TonkState) -> bool {
    use tonk_account::customer::CustomerStatus;

    let Some(origin) =
        super::repository::app_origin().and_then(|origin| url::Url::parse(&origin).ok())
    else {
        return false;
    };
    matches!(
        super::customer::probe(tonk, &origin).await,
        Ok(Some(CustomerStatus::Active))
    )
}

/// Record where a custody hand-off for `kind` left the panel, and wait out
/// the emailed link when that is where it is.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn settle(
    state: &crate::router::AppState,
    kind: &'static str,
    outcome: Result<(), (Option<&str>, &str)>,
) {
    let tonk = state.read().await;
    let active = outcome.is_ok() && served(&tonk).await;
    let code = outcome.as_ref().err().and_then(|(code, _)| *code);
    let handed = outcome.is_ok();
    let created = handed && kind == kind::CREATE;
    let stage = settled(kind, outcome, active);
    log!("registration: the {kind} hand-off leaves the panel at {stage:?}");
    if let Some(attempt) = CEREMONY.take() {
        let client = attempt.client().cloned();
        let (ended, outcome) = account_journey::handed_off(stage.as_ref(), code);
        attempt.end(ended, outcome);
        if created {
            crate::router::navigate::notify_analytics(
                client.as_ref(),
                tonk_worker_api::AnalyticsEvent::AccountCreated,
            );
        }
        // A ceremony that went through to an account not yet served is
        // followed by the wait for its emailed link.
        if handed && matches!(stage, Some(Stage::Confirming { .. })) {
            watch(client.as_ref());
        }
    }
    let waiting = matches!(stage, Some(Stage::Confirming { .. }));
    record(&tonk, stage).await;
    drop(tonk);
    if waiting {
        await_confirmation(state.clone());
    }
}

/// While the panel waits for the emailed link, ask the service every few
/// seconds how the account stands. The link can be opened on another
/// device, which reaches this one only through the service; the probe that
/// hears `Active` records it, and that puts the panel away.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn await_confirmation(state: crate::router::AppState) {
    thread_local! {
        static WAITING: Cell<bool> = const { Cell::new(false) };
    }
    if WAITING.replace(true) {
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let _ = crate::r#async::sleep(web_time::Duration::from_secs(4)).await;
            let tonk = state.read().await;
            if !confirming(&tonk).await || served(&tonk).await {
                break;
            }
        }
        WAITING.set(false);
    });
}

/// A passkey ceremony the page asked for was refused before anything
/// reached this worker: the prompt was closed, or the browser would not
/// show it. `name` is the browser's name for the refusal.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn ceremony_refused(tonk: &TonkState, kind: &str, name: &str) {
    let kind = match kind {
        kind::CREATE => kind::CREATE,
        _ => kind::LOG_IN,
    };
    log!("registration: the page refused the {kind} passkey ({name})");
    if let Some(attempt) = CEREMONY.take() {
        let (ended, outcome) = account_journey::refused(name);
        attempt.end(ended, outcome);
    }
    let message = refused(name);
    record(tonk, Some(Stage::Failed { kind, message })).await;
}

/// The `/ucan/` endpoint of the deployment this page is on, which a new
/// account and a login both sync through.
fn endpoint() -> Option<String> {
    super::repository::app_origin().map(|origin| format!("{}/ucan/", origin.trim_end_matches('/')))
}

/// Ask the page that asserted `env`'s command for the passkey ceremony
/// `intent`, recording that the panel waits for it.
async fn ask(
    env: &crate::router::CommandEnv,
    kind: &'static str,
    intent: tonk_worker_api::CustodyIntent,
) {
    if !claim_ceremony() {
        log!("registration: a passkey ceremony is already out; not asking for another");
        return;
    }
    let tonk = env.state().read().await;
    record(&tonk, Some(Stage::Ceremony { kind })).await;
    let mut attempt = Attempt::begin(
        env.client(),
        account_journey::action_of(kind),
        account::Trigger::User,
    );
    attempt.reached(account::Stage::EmailLookup);
    attempt.reached(account_journey::passkey_stage(kind));
    CEREMONY.set(Some(attempt));
    // The page was never asked: nothing the page did before could say so.
    let unasked = || {
        if let Some(attempt) = CEREMONY.take() {
            attempt.end(
                account::Stage::WorkerHandoff,
                AccountOutcome::retryable(account::FailureKind::LocalState),
            );
        }
    };
    let failed = |message: &str| Stage::Failed {
        kind,
        message: message.to_owned(),
    };
    let Some(client) = env.client() else {
        unasked();
        record(&tonk, Some(failed("No page is open to ask for a passkey."))).await;
        return;
    };
    if let Err(error) = super::navigate::request_webauthn_with(
        client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(intent),
        None,
    )
    .await
    {
        log!("registration: the page could not be asked for a passkey: {error}");
        unasked();
        record(
            &tonk,
            Some(failed("This page could not be asked for a passkey.")),
        )
        .await;
    }
}

/// Log in with a passkey the person picks.
async fn log_in(env: &crate::router::CommandEnv) {
    let Some(endpoint) = endpoint() else {
        let tonk = env.state().read().await;
        let message = "This deployment's address is not known.".to_owned();
        record(
            &tonk,
            Some(Stage::Failed {
                kind: kind::LOG_IN,
                message,
            }),
        )
        .await;
        return;
    };
    let link = tonk_worker_api::DeviceLink {
        device_name: crate::onboarding::device_title(),
        endpoint: endpoint.clone(),
        provider: endpoint,
    };
    ask(
        env,
        kind::LOG_IN,
        tonk_worker_api::CustodyIntent::Login(link),
    )
    .await;
}

/// Run `account/open-registration`: the panel asks for an address.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::OpenRegistration>
    for crate::router::CommandEnv
{
    async fn execute(&self, _command: tonk_schema::command::OpenRegistration) {
        opened(self.client());
        let tonk = self.state().read().await;
        record(
            &tonk,
            Some(Stage::Address {
                email: String::new(),
            }),
        )
        .await;
    }
}

/// Run `account/start-registration`: look the address up, then name a new
/// account or log in to the one the address has.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::StartRegistration>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::StartRegistration) {
        let email = command.email.0.trim().to_owned();
        if email.is_empty() {
            return;
        }
        let answer = {
            let tonk = self.state().read().await;
            record(
                &tonk,
                Some(Stage::Address {
                    email: email.clone(),
                }),
            )
            .await;
            super::email_status::check(&tonk, &email).await
        };
        match next(answer) {
            Next::Name => {
                let tonk = self.state().read().await;
                record(&tonk, Some(Stage::Naming { email })).await;
            }
            Next::LogIn => log_in(self).await,
            Next::Stay => {}
        }
    }
}

/// Run `account/create`: ask the page for a new passkey to hold the
/// account.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::CreateAccount>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::CreateAccount) {
        let email = command.email.0.trim().to_owned();
        let name = command.name.0.trim().to_owned();
        if email.is_empty() || name.is_empty() {
            return;
        }
        // The account's first name is given here, with the request to
        // create it, where it used to be saved by a step of its own.
        account_journey::done_at_once(
            self.client(),
            account::AccountAction::SaveInitialDisplayName,
            account::Stage::LocalCommit,
        );
        let Some(endpoint) = endpoint() else {
            let tonk = self.state().read().await;
            let message = "This deployment's address is not known.".to_owned();
            record(
                &tonk,
                Some(Stage::Failed {
                    kind: kind::CREATE,
                    message,
                }),
            )
            .await;
            return;
        };
        let device = crate::onboarding::device_title();
        let creation = tonk_worker_api::AccountCreation {
            email,
            display_name: Some(name),
            device_name: device.clone(),
            remote: endpoint.clone(),
            provider: endpoint,
            created_on: Some(device),
        };
        ask(
            self,
            kind::CREATE,
            tonk_worker_api::CustodyIntent::CreateAccount(creation),
        )
        .await;
    }
}

/// Run `account/log-in`.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::LogIn> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::LogIn) {
        log_in(self).await;
    }
}

/// Run `account/open-sign-in-via`: the panel asks which Tonk holds the
/// account.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::OpenSignInVia>
    for crate::router::CommandEnv
{
    async fn execute(&self, _command: tonk_schema::command::OpenSignInVia) {
        let tonk = self.state().read().await;
        record(
            &tonk,
            Some(Stage::Via {
                origin: String::new(),
            }),
        )
        .await;
    }
}

/// Run `account/dismiss-registration`: put the panel away.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::DismissRegistration>
    for crate::router::CommandEnv
{
    async fn execute(&self, _command: tonk_schema::command::DismissRegistration) {
        // Putting the panel away gives up whatever it was waiting on.
        for waiting in [LOADING.take(), WATCHING.take()].into_iter().flatten() {
            waiting.end(account::Stage::Complete, AccountOutcome::cancelled());
        }
        let tonk = self.state().read().await;
        record(&tonk, None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{ASKING, Next, Stage, claim_ceremony, next, refused, settled};
    use tonk_schema::registration::kind;

    #[dialog_common::test]
    fn it_names_an_account_for_a_free_address() {
        assert_eq!(next("unregistered"), Next::Name);
    }

    #[dialog_common::test]
    fn it_asks_for_one_passkey_at_a_time() {
        ASKING.set(false);
        assert!(claim_ceremony());
        assert!(!claim_ceremony(), "a second ask while one is out");
        ASKING.set(false);
        assert!(
            claim_ceremony(),
            "a stage other than the ceremony gives it up"
        );
        ASKING.set(false);
    }

    #[dialog_common::test]
    fn it_logs_in_to_an_address_an_account_has() {
        assert_eq!(next("active"), Next::LogIn);
        assert_eq!(next("pending"), Next::LogIn);
    }

    #[dialog_common::test]
    fn it_stays_at_an_address_nothing_is_on_offer_for() {
        for answer in ["invalid", "suspended", "unavailable", "checking"] {
            assert_eq!(next(answer), Next::Stay, "{answer}");
        }
    }

    #[dialog_common::test]
    fn it_puts_the_panel_away_once_the_account_is_served() {
        assert_eq!(settled(kind::LOG_IN, Ok(()), true), None);
    }

    #[dialog_common::test]
    fn it_waits_for_the_emailed_link_of_an_account_not_yet_served() {
        assert_eq!(
            settled(kind::CREATE, Ok(()), false),
            Some(Stage::Confirming { kind: kind::CREATE })
        );
        assert_eq!(
            settled(kind::LOG_IN, Err((Some("awaiting-activation"), "")), false),
            Some(Stage::Confirming { kind: kind::LOG_IN })
        );
    }

    #[dialog_common::test]
    fn it_says_what_went_wrong_with_a_refused_hand_off() {
        let Some(Stage::Failed {
            kind: failed,
            message,
        }) = settled(
            kind::LOG_IN,
            Err((None, "the custody cell did not open")),
            false,
        )
        else {
            panic!("a refusal is a failure");
        };
        assert_eq!(failed, kind::LOG_IN);
        assert_eq!(message, "the custody cell did not open");
    }

    #[dialog_common::test]
    fn it_words_a_closed_passkey_prompt_as_one_to_try_again() {
        assert!(refused("NotAllowedError").contains("Try again"));
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod waiting_tests {
    use super::{Stage, confirming, record};
    use crate::router::customer::{record_activation, record_customer_status};
    use tonk_account::customer::CustomerStatus;
    use tonk_schema::registration::kind;

    /// The sweep that learns from a sync that the account is served is one
    /// of the ways the emailed link is heard of: the panel waiting for the
    /// link is put away by it.
    #[dialog_common::test]
    async fn it_puts_the_waiting_panel_away_when_activation_is_recorded() {
        let tonk = crate::router::tests::test_state().await;
        record_customer_status(
            &tonk,
            CustomerStatus::Registered,
            "waiting@example.com",
            None,
        )
        .await
        .expect("the registration is recorded");
        record(&tonk, Some(Stage::Confirming { kind: kind::CREATE })).await;
        assert!(confirming(&tonk).await);

        record_activation(&tonk).await;

        assert!(!confirming(&tonk).await);
    }

    /// A refusal reaches the worker as a message from the page, with no
    /// command after it: the panel showing the wait for a passkey still
    /// hears that it failed.
    #[dialog_common::test]
    async fn it_tells_the_panel_of_a_ceremony_the_page_refused() {
        use dialog_query::{ConceptQuery, Query, Term};

        let tonk = crate::router::tests::test_state().await;
        record(&tonk, Some(Stage::Ceremony { kind: kind::CREATE })).await;
        let this: dialog_artifacts::Entity = super::ENTITY.parse().unwrap();
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .expect("the profile branch opens");
        let mut failed = session
            .subscribe(
                ConceptQuery::from(Query::<super::RegistrationFailed> {
                    this: Term::from(this),
                    kind: Term::var("kind"),
                    message: Term::var("message"),
                }),
                None,
                0,
            )
            .expect("the panel subscribes");
        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&session.state));
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        while failed.receiver.try_recv().is_ok() {}

        super::ceremony_refused(&tonk, kind::CREATE, "NotAllowedError").await;

        let mut heard = Vec::new();
        while let Ok(bytes) = failed.receiver.try_recv() {
            heard.extend_from_slice(&bytes);
        }
        let heard = String::from_utf8_lossy(&heard);
        assert!(
            heard.contains("asserted") && heard.contains(kind::CREATE),
            "the panel hears of the failure: {heard:?}"
        );
    }
}

/// What the panel tells the account journey as it is used, with the state
/// of a signed-in device behind it.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod journey_tests {
    use dialog_capability::Provider;
    use tonk_schema::command::{DismissRegistration, OpenRegistration};
    use tonk_schema::domain::command::current::{dismiss_registration, open_registration};
    use tonk_schema::registration::kind;

    use super::{Stage, account_journey, activated, answered, record, watch};
    use crate::router::{CommandEnv, CommandOrigin};

    /// The parts of each event told a funnel is built on.
    fn told() -> Vec<String> {
        account_journey::heard()
            .iter()
            .map(|event| {
                let part = |key: &str| event.get(key).and_then(|v| v.as_str()).unwrap_or("-");
                format!(
                    "{} {} {} {} {}",
                    part("action"),
                    part("phase"),
                    part("stage"),
                    part("trigger"),
                    part("result")
                )
            })
            .collect()
    }

    async fn env() -> CommandEnv {
        let state = crate::router::command::tests::native::test_state().await;
        CommandEnv::new(state, CommandOrigin::default())
    }

    async fn open(env: &CommandEnv) {
        Provider::<OpenRegistration>::execute(
            env,
            OpenRegistration {
                this: "cmd:open".parse().expect("entity"),
                time: open_registration::Time(1.0),
            },
        )
        .await;
    }

    async fn dismiss(env: &CommandEnv) {
        Provider::<DismissRegistration>::execute(
            env,
            DismissRegistration {
                this: "cmd:dismiss".parse().expect("entity"),
                time: dismiss_registration::Time(2.0),
            },
        )
        .await;
    }

    #[dialog_common::test]
    async fn it_tells_the_panel_opening_and_its_first_answer() {
        let env = env().await;
        account_journey::listen();
        open(&env).await;
        answered();
        // Only the first answer ends the wait.
        answered();

        assert_eq!(
            told(),
            [
                "open_registration started input user -",
                "open_registration finished input user success",
                "load_registration started account_load automatic -",
                "load_registration finished account_load automatic success",
            ]
        );
    }

    #[dialog_common::test]
    async fn it_tells_a_panel_put_away_before_it_had_an_answer() {
        let env = env().await;
        account_journey::listen();
        open(&env).await;
        dismiss(&env).await;

        assert_eq!(
            told()[2..],
            [
                "load_registration started account_load automatic -",
                "load_registration finished complete automatic cancelled",
            ]
        );
    }

    #[dialog_common::test]
    async fn it_tells_the_wait_for_the_emailed_link_when_it_ends() {
        let env = env().await;
        let tonk = env.state().read().await;
        record(&tonk, Some(Stage::Confirming { kind: kind::CREATE })).await;
        account_journey::listen();
        watch(None);

        activated(&tonk).await;

        assert_eq!(
            told(),
            [
                "watch_activation started activation_wait automatic -",
                "watch_activation finished complete automatic success",
            ]
        );
    }

    #[dialog_common::test]
    async fn it_tells_a_wait_for_the_link_that_was_put_away() {
        let env = env().await;
        {
            let tonk = env.state().read().await;
            record(&tonk, Some(Stage::Confirming { kind: kind::CREATE })).await;
        }
        account_journey::listen();
        watch(None);

        dismiss(&env).await;

        assert_eq!(
            told(),
            [
                "watch_activation started activation_wait automatic -",
                "watch_activation finished complete automatic cancelled",
            ]
        );
    }
}
