//! The panel that adds an account to this profile.
//!
//! The panel is views of the stages in [`tonk_schema::registration`], and
//! this is what moves it from one to the next: the commands its views
//! assert, and the custody hand-off that ends a passkey ceremony. The
//! ceremony itself runs in the top-level page, the one place WebAuthn
//! can, which the worker asks for it; everything else is decided here.

use std::cell::Cell;

use dialog_artifacts::Entity;
use tonk_common::log;
use tonk_schema::registration::{
    ENTITY, RegistrationAddress, RegistrationCeremony, RegistrationConfirming, RegistrationFailed,
    RegistrationNaming, RegistrationVia, kind,
};

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
    let stage = settled(kind, outcome, active);
    log!("registration: the {kind} hand-off leaves the panel at {stage:?}");
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
    let failed = |message: &str| Stage::Failed {
        kind,
        message: message.to_owned(),
    };
    let Some(client) = env.client() else {
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
