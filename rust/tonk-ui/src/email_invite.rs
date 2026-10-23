//! The emailed locator remains on the trusted page throughout signup and claim.
use custom_elements::CustomElement;
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc};
use tonk_worker_api::email_invitation;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{HtmlButtonElement, HtmlElement};

/// Accept only the exact local invitation route and opaque locator.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn destination(value: &str) -> bool {
    let Some(id) = value.strip_prefix("/email-invite#") else {
        return false;
    };
    valid_id(id)
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What the page knows about this browser's account and the invitation.
///
/// The locator is opaque, so whether the invitation is live, or meant for
/// this account, is only learned from the claim's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Checking,
    Incomplete,
    SignedOut,
    Unconfirmed,
    Ready,
    WrongAccount,
    Gone,
}

/// The one primary action a stage offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Open the account ceremony, labelled as given.
    Account(&'static str),
    Recheck,
    Join,
}

/// The one secondary action a stage offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Secondary {
    Home,
    Resend,
    Settings,
}

struct View {
    heading: &'static str,
    body: &'static [&'static str],
    action: Option<Action>,
    secondary: Secondary,
    /// Whether the signed-in email is relevant enough to show.
    shows_account: bool,
}

fn view(stage: Stage) -> View {
    match stage {
        Stage::Checking => View {
            heading: "You’re invited to a space",
            body: &["Checking your account…"],
            action: None,
            secondary: Secondary::Home,
            shows_account: false,
        },
        Stage::Incomplete => View {
            heading: "This invitation link is incomplete",
            body: &[
                "Open the exact link from your invitation email.",
                "If it still doesn’t work, ask the person who invited you to send a new invitation.",
            ],
            action: None,
            secondary: Secondary::Home,
            shows_account: false,
        },
        Stage::SignedOut => View {
            heading: "You’re invited to a space",
            body: &[
                "Someone invited your email address to a shared space on Tonk.",
                "Create an account or sign in with the email address that received this invitation. After you confirm it, come back here to join.",
            ],
            action: Some(Action::Account("Create account or sign in")),
            secondary: Secondary::Home,
            shows_account: false,
        },
        Stage::Unconfirmed => View {
            heading: "Confirm your email",
            body: &[
                "Your account is set up. Before you can join, confirm your email address.",
                "Open the confirmation link we sent you, then come back here. This page checks again on its own.",
            ],
            action: Some(Action::Recheck),
            secondary: Secondary::Resend,
            shows_account: true,
        },
        Stage::Ready => View {
            heading: "You’re invited to a space",
            body: &["Your email is confirmed. Join the shared space with this account."],
            action: Some(Action::Join),
            secondary: Secondary::Home,
            shows_account: true,
        },
        Stage::WrongAccount => View {
            heading: "This invitation is for a different email",
            body: &[
                "This account’s email address didn’t receive this invitation.",
                "Sign in with the email address the invitation was sent to, or switch accounts in settings. You can also ask for a new invitation to this address.",
            ],
            action: Some(Action::Account("Use another account")),
            secondary: Secondary::Settings,
            shows_account: true,
        },
        Stage::Gone => View {
            heading: "This invitation is no longer available",
            body: &[
                "It has expired, been cancelled or already been used.",
                "Ask the person who invited you to send a new invitation.",
            ],
            action: None,
            secondary: Secondary::Home,
            shows_account: false,
        },
    }
}

/// The stage before claiming, from the account's root and customer status.
///
/// Only `Registered` waits on the activation email. An unread or other
/// status leaves the choice to the claim, which reports refusals itself.
fn account_stage(signed_in: bool, customer: Option<Option<&str>>) -> Stage {
    match (signed_in, customer) {
        (false, _) | (true, Some(None)) => Stage::SignedOut,
        (true, Some(Some("Registered"))) => Stage::Unconfirmed,
        (true, Some(Some(_))) | (true, None) => Stage::Ready,
    }
}

/// The stage a failed claim moves to, when its message names one.
///
/// Outcome messages carry the worker error's label (`Conflict: …`), so
/// match on the fixed message the label wraps.
fn failure_stage(message: &str) -> Option<Stage> {
    [
        (email_invitation::WRONG_ACCOUNT, Stage::WrongAccount),
        (email_invitation::UNCONFIRMED, Stage::Unconfirmed),
        (email_invitation::GONE, Stage::Gone),
    ]
    .into_iter()
    .find(|(fixed, _)| message.ends_with(fixed))
    .map(|(_, stage)| stage)
}

/// The stage to show after a refused claim, given whether the account's
/// email is now read as confirmed.
///
/// A 409 also answers an invitation that changed or was already claimed;
/// for a verified `Active` account that is the only reading left. An
/// unread status proves nothing, so the refusal stays retryable.
fn settled(refused: Stage, confirmed: bool) -> Stage {
    match (refused, confirmed) {
        (Stage::Unconfirmed, true) => Stage::Gone,
        (refused, _) => refused,
    }
}

/// A failure message fit to show: the worker error's label removed, and
/// internal failures replaced by a retry hint.
fn readable(message: &str) -> &str {
    const RETRY: &str = "Couldn’t join the space. Your invitation is safe to retry.";
    for label in [
        "Router error: ",
        "Internal error: ",
        "Internal repository error: ",
    ] {
        if message.starts_with(label) {
            return RETRY;
        }
    }
    for label in [
        "Conflict: ",
        "Forbidden: ",
        "Not found: ",
        "Precondition failed: ",
    ] {
        if let Some(rest) = message.strip_prefix(label) {
            return rest;
        }
    }
    if message.starts_with("Upstream service returned") {
        return RETRY;
    }
    message
}

const ACCOUNT: &str = "#email-invite-account";
const RECHECK: &str = "#email-invite-recheck";
const JOIN: &str = "#email-invite-join";

fn show(host: &HtmlElement, stage: Stage, email: Option<&str>) {
    let document = host.owner_document();
    let View {
        heading,
        body,
        action,
        secondary,
        shows_account,
    } = view(stage);
    let _ = host.set_attribute("data-stage", &format!("{stage:?}").to_lowercase());
    if let Ok(Some(node)) = host.query_selector("#email-invite-title") {
        node.set_text_content(Some(heading));
    }
    if let (Ok(Some(node)), Some(document)) =
        (host.query_selector("#email-invite-body"), document.as_ref())
    {
        node.set_text_content(None);
        for line in body {
            if let Ok(p) = document.create_element("p") {
                p.set_text_content(Some(line));
                let _ = node.append_child(&p);
            }
        }
    }
    let email = email.filter(|_| shows_account);
    if let Ok(Some(row)) = host.query_selector("#email-invite-signed-in") {
        let _ = row.toggle_attribute_with_force("hidden", email.is_none());
    }
    if let Ok(Some(node)) = host.query_selector("#email-invite-email") {
        node.set_text_content(email);
    }
    for (selector, shown) in [
        (ACCOUNT, matches!(action, Some(Action::Account(_)))),
        (RECHECK, action == Some(Action::Recheck)),
        (JOIN, action == Some(Action::Join)),
    ] {
        if let Some(button) = button(host, selector) {
            button.set_hidden(!shown);
            button.set_disabled(false);
        }
    }
    for (selector, shown) in [
        ("#email-invite-home", secondary == Secondary::Home),
        ("#email-invite-resend", secondary == Secondary::Resend),
        ("#email-invite-settings", secondary == Secondary::Settings),
    ] {
        if let Ok(Some(node)) = host.query_selector(selector) {
            let _ = node.toggle_attribute_with_force("hidden", !shown);
        }
    }
    if let (Some(Action::Account(label)), Some(button)) = (action, button(host, ACCOUNT)) {
        button.set_text_content(Some(label));
    }
    if let Ok(Some(actions)) = host.query_selector(".account__actions") {
        let solo = action.is_none() && stage != Stage::Checking;
        let _ = actions.toggle_attribute_with_force("data-solo", solo);
    }
}

#[derive(Default)]
struct EmailInvite;
impl CustomElement for EmailInvite {
    fn shadow() -> bool {
        false
    }
    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }
    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(include_str!("email_invite.html"));
    }
    fn connected_callback(&mut self, this: &HtmlElement) {
        if this.has_attribute("data-bound") {
            return;
        }
        let _ = this.set_attribute("data-bound", "");
        if let Some(doc) = this.owner_document()
            && doc.get_element_by_id("tonk-activate-styles").is_none()
            && let Ok(style) = doc.create_element("style")
        {
            style.set_id("tonk-activate-styles");
            style.set_text_content(Some(include_str!("account.css")));
            if let Some(head) = doc.head() {
                let _ = head.append_child(&style);
            }
        }
        if !valid_id(&locator()) {
            show(this, Stage::Incomplete, None);
            return;
        }
        show(this, Stage::Checking, None);
        // The page settles on its own: it re-reads the account while it
        // waits for signup or confirmation, including in another tab.
        let busy = Rc::new(Cell::new(false));
        let host = this.clone();
        let held = busy.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                if !host.is_connected() {
                    return;
                }
                let stage = host.get_attribute("data-stage").unwrap_or_default();
                let hidden = host.owner_document().is_some_and(|d| d.hidden());
                if !held.get()
                    && !hidden
                    && matches!(stage.as_str(), "checking" | "signedout" | "unconfirmed")
                {
                    refresh(&host).await;
                }
                sleep(3000).await;
            }
        });
        if let Some(button) = button(this, ACCOUNT) {
            let callback = Closure::<dyn FnMut()>::new(move || {
                wasm_bindgen_futures::spawn_local(async {
                    resolve_profile().await;
                    crate::register_dialog::open();
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
        if let Some(button) = button(this, RECHECK) {
            let host = this.clone();
            let action = button.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                action.set_disabled(true);
                let (host, action) = (host.clone(), action.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    refresh(&host).await;
                    if host.get_attribute("data-stage").as_deref() == Some("unconfirmed") {
                        status(
                            &host,
                            "Your email isn’t confirmed yet. Open the link in the confirmation email first.",
                        );
                    }
                    action.set_disabled(false);
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
        if let Some(button) = button(this, "#email-invite-resend") {
            let host = this.clone();
            let action = button.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if action.disabled() {
                    return;
                }
                action.set_disabled(true);
                let (host, action) = (host.clone(), action.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let sent = resend().await;
                    status(
                        &host,
                        if sent {
                            "Another confirmation email is on its way. Open its link, then come back here."
                        } else {
                            "Couldn’t send another email. Check your connection and try again."
                        },
                    );
                    action.set_disabled(false);
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
        if let Some(button) = button(this, JOIN) {
            let host = this.clone();
            let action = button.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if action.disabled() {
                    return;
                }
                action.set_disabled(true);
                busy.set(true);
                status(&host, "Checking your invitation…");
                let (host, action, busy) = (host.clone(), action.clone(), busy.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(message) = claim(&host, &locator()).await {
                        match failure_stage(&message) {
                            Some(stage) => {
                                status(&host, "");
                                let (_, email, confirmed) = account().await;
                                show(&host, settled(stage, confirmed), email.as_deref());
                            }
                            None => status(&host, readable(&message)),
                        }
                    }
                    action.set_disabled(false);
                    busy.set(false);
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
    }
    fn disconnected_callback(&mut self, _: &HtmlElement) {}
    fn attribute_changed_callback(
        &mut self,
        _: &HtmlElement,
        _: String,
        _: Option<String>,
        _: Option<String>,
    ) {
    }
}

/// Register the invitation page element.
pub fn register() {
    EmailInvite::define("tonk-email-invite");
}

fn locator() -> String {
    let id = web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .unwrap_or_default();
    id.trim_start_matches('#').to_owned()
}

fn button(host: &HtmlElement, selector: &str) -> Option<HtmlButtonElement> {
    host.query_selector(selector)
        .ok()
        .flatten()?
        .dyn_into()
        .ok()
}

fn status(host: &HtmlElement, message: &str) {
    if let Ok(Some(node)) = host.query_selector("#email-invite-status") {
        node.set_text_content(Some(message));
    }
}

/// Re-read the account and show the stage it implies, unless it is unchanged.
async fn refresh(host: &HtmlElement) {
    let (stage, email, _) = account().await;
    let current = host.get_attribute("data-stage").unwrap_or_default();
    if current != format!("{stage:?}").to_lowercase() || current == "checking" {
        show(host, stage, email.as_deref());
    }
}

/// The account's stage before claiming, its email when known, and whether
/// the service verifiably reads it as confirmed.
async fn account() -> (Stage, Option<String>, bool) {
    resolve_profile().await;
    let signed_in = matches!(
        crate::api::root_status().await,
        Ok(tonk_worker_api::RootStatus::Ready { .. })
    );
    let customer = if signed_in {
        crate::api::customer_state().await.ok()
    } else {
        None
    };
    let status = customer
        .as_ref()
        .map(|state| state.get("status").and_then(Value::as_str));
    let email = customer
        .as_ref()
        .and_then(|state| state.get("email").and_then(Value::as_str));
    let confirmed = status == Some(Some("Active"));
    (
        account_stage(signed_in, status),
        email.map(str::to_owned),
        confirmed,
    )
}

/// Ask for another activation email, as the account ceremony does.
async fn resend() -> bool {
    crate::api::transact_profile(json!({"claims":[{"op":"assert","application":{
        "predicate":{"kind":"transient","concept":{
            "description":"Request another activation email.",
            "with":{"at":{"the":"xyz.tonk.resend-activation/at","as":"natural:64"}}
        }},
        "parameters":{"at":js_sys::Date::now() as u64}
    }}]}))
    .await
    .is_ok()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

async fn claim(host: &HtmlElement, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(
            "This invitation link is incomplete. Open the exact link from your email.".into(),
        );
    }
    resolve_profile().await;
    let root = crate::api::root_status()
        .await
        .map_err(|_| "Couldn’t read your account. Try again.")?;
    let tonk_worker_api::RootStatus::Ready { root_did, .. } = root else {
        return Err("Create an account or sign in using the email that received this invitation, then select Join space.".into());
    };
    let request = format!(
        "request:email-invite-{}-{}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * u64::MAX as f64) as u64
    );
    let branch = crate::api::profile_branch();
    let field = |the: &str, kind: &str| json!({"the":the,"as":kind,"cardinality":"one"});
    crate::api::transact_profile(json!({"claims":[{"op":"assert","application":{
        "predicate":{"kind":"transient","concept":{"with":{
            "id":field("xyz.tonk.claim-email-invitation/id","Text"),
            "account":field("xyz.tonk.claim-email-invitation/account","Entity")
        }}},"parameters":{"this":request,"id":id,"account":root_did}
    }}]}))
    .await
    .map_err(|_| "Couldn’t start joining. Your invitation is safe to retry.")?;
    let query = json!({"predicate":{"with":{
        "state":field("xyz.tonk.email-invitation-status/state","Text"),
        "message":field("xyz.tonk.email-invitation-status/message","Text")
    }},"terms":{"this":request,"state":{"?":{"name":"state"}},"message":{"?":{"name":"message"}}}});
    // Read the command's overlay outcome. No claim is replayed by this wait.
    for _ in 0..60 {
        if !host.is_connected() {
            return Ok(());
        }
        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/repository/profile:tonk/branch/{branch}/query",
                crate::api::origin()
            ))
            .json(&query)
            .send()
            .await
            .map_err(|_| "Couldn’t read the result. Select Join space to retry safely.")?;
        if !response.status().is_success() {
            return Err("Couldn’t read the result. Select Join space to retry safely.".into());
        }
        let rows: Vec<Value> = response
            .json()
            .await
            .map_err(|_| "Couldn’t read the invitation result.")?;
        if let Some(fields) = rows
            .iter()
            .map(|r| &r["fields"])
            .find(|r| r["state"] != "pending")
        {
            let message = fields["message"]
                .as_str()
                .unwrap_or("Couldn’t join this space.");
            if fields["state"] == "joined" {
                status(host, "Space joined. Opening it…");
                return Ok(());
            }
            return Err(message.into());
        }
        sleep(1000).await;
    }
    Err("Joining is taking longer than expected. Select Join space to retry safely.".into())
}

async fn resolve_profile() {
    #[cfg(target_arch = "wasm32")]
    tonk_host::bridge::resolve_profile_with().await;
}

#[cfg(test)]
mod tests {
    use super::{Action, Secondary, Stage, account_stage, failure_stage, readable, settled, view};
    use tonk_worker_api::email_invitation;

    #[test]
    fn signup_return_is_only_an_exact_local_locator() {
        assert!(super::destination(&format!(
            "/email-invite#{}",
            "a".repeat(64)
        )));
        for value in [
            "//evil.test/email-invite#abc",
            "/email-invite#abc",
            "/email-invite?next=//evil.test#abc",
            "/email-invite/../other#abc",
        ] {
            assert!(!super::destination(value));
        }
    }

    #[test]
    fn it_offers_signup_until_an_account_exists() {
        assert_eq!(account_stage(false, None), Stage::SignedOut);
        assert_eq!(account_stage(true, Some(None)), Stage::SignedOut);
        assert_eq!(
            view(Stage::SignedOut).action,
            Some(Action::Account("Create account or sign in"))
        );
    }

    #[test]
    fn it_offers_join_only_once_the_email_is_confirmed() {
        assert_eq!(account_stage(true, Some(Some("Active"))), Stage::Ready);
        assert_eq!(view(Stage::Ready).action, Some(Action::Join));
        assert_eq!(
            account_stage(true, Some(Some("Registered"))),
            Stage::Unconfirmed
        );
        assert_eq!(view(Stage::Unconfirmed).action, Some(Action::Recheck));
    }

    #[test]
    fn it_leaves_an_unread_status_to_the_claim() {
        assert_eq!(account_stage(true, None), Stage::Ready);
    }

    #[test]
    fn it_maps_each_refused_claim_to_its_own_state() {
        assert_eq!(
            failure_stage(email_invitation::WRONG_ACCOUNT),
            Some(Stage::WrongAccount)
        );
        assert_eq!(
            failure_stage(email_invitation::UNCONFIRMED),
            Some(Stage::Unconfirmed)
        );
        assert_eq!(failure_stage(email_invitation::GONE), Some(Stage::Gone));
        assert_eq!(failure_stage("Delivery could not be confirmed."), None);
    }

    #[test]
    fn it_recognises_refusals_inside_the_worker_error_label() {
        let message = format!("Conflict: {}", email_invitation::UNCONFIRMED);
        assert_eq!(failure_stage(&message), Some(Stage::Unconfirmed));
        let message = format!("Conflict: {}", email_invitation::GONE);
        assert_eq!(failure_stage(&message), Some(Stage::Gone));
    }

    #[test]
    fn a_conflict_for_a_confirmed_account_means_the_invitation_is_gone() {
        assert_eq!(settled(Stage::Unconfirmed, true), Stage::Gone);
        assert_eq!(settled(Stage::Unconfirmed, false), Stage::Unconfirmed);
        assert_eq!(settled(Stage::WrongAccount, true), Stage::WrongAccount);
    }

    #[test]
    fn an_unread_confirmation_status_keeps_a_refusal_retryable() {
        // The status read failed, so the page offered Join; a 409 then
        // must not be taken as proof the invitation is gone.
        assert_eq!(account_stage(true, None), Stage::Ready);
        assert_eq!(settled(Stage::Unconfirmed, false), Stage::Unconfirmed);
        assert_eq!(view(Stage::Unconfirmed).action, Some(Action::Recheck));
    }

    #[test]
    fn it_never_shows_a_raw_error_label() {
        assert_eq!(
            readable("Conflict: The selected account changed. Try again."),
            "The selected account changed. Try again."
        );
        for raw in [
            "Internal error: boom",
            "Router error: no route",
            "Upstream service returned HTTP 502: bad gateway",
        ] {
            assert!(!readable(raw).contains(':'), "{raw}");
        }
    }

    #[test]
    fn it_offers_resend_while_waiting_for_confirmation() {
        assert_eq!(view(Stage::Unconfirmed).secondary, Secondary::Resend);
        assert_eq!(view(Stage::WrongAccount).secondary, Secondary::Settings);
        assert_eq!(view(Stage::Ready).secondary, Secondary::Home);
    }

    #[test]
    fn it_names_the_signed_in_email_only_where_it_matters() {
        for stage in [Stage::Unconfirmed, Stage::Ready, Stage::WrongAccount] {
            assert!(view(stage).shows_account, "{stage:?}");
        }
        for stage in [
            Stage::Checking,
            Stage::Incomplete,
            Stage::SignedOut,
            Stage::Gone,
        ] {
            assert!(!view(stage).shows_account, "{stage:?}");
        }
    }

    #[test]
    fn dead_ends_offer_no_primary_action() {
        for stage in [Stage::Checking, Stage::Incomplete, Stage::Gone] {
            assert_eq!(view(stage).action, None, "{stage:?}");
        }
    }
}
