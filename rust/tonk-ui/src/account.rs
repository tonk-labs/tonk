//! Top-document account creation and passkey self-link surface.

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, KeyboardEvent, window};

use tonk_account::AccountStateStatus;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountSpaceDeletionRequest, AccountStatus,
    RevokeDeviceAcknowledgement,
};

use crate::identity_bridge::{VerifyPasskeyInput, verify_passkey};
use crate::user_error::{self, AccountAction};

const STYLE_ID: &str = "tonk-account-styles";
/// Where a pending callback authorization's `(audience, callback)` is parked.
const CALLBACK: &str = "__tonkCliCallback";
const CONFIRMATION: &str = "__tonkAccountConfirmation";
const CONFIRMATION_RETURN_FOCUS: &str = "__tonkAccountConfirmationReturnFocus";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum Confirmation {
    SignOut,
    Revoke {
        did: String,
        self_revoke: bool,
    },
    Delete {
        plan: AccountDeletionPlan,
        requested_space: Option<String>,
    },
}

enum DeleteFailure {
    PreflightApi(crate::error::TonkUiError),
    MutationApi(crate::error::TonkUiError),
    Diagnostic(String),
}

/// The top-document account element. WebAuthn must not run in sealed guests.
#[derive(Default)]
struct TonkAccount;

impl CustomElement for TonkAccount {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(include_str!("account.html"));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        ensure_stylesheet();
        if Reflect::get(this.as_ref(), &"__tonkAccountBound".into())
            .map(|value| value.is_truthy())
            .unwrap_or(false)
        {
            return;
        }
        let _ = Reflect::set(this.as_ref(), &"__tonkAccountBound".into(), &JsValue::TRUE);
        bind(this);
        load_status(this.clone());
        // The panel's state is a function of the URL — /settings,
        // /settings?add=1, /settings/link — and of whether this browser
        // has an account. Neither reaches it as a reload.
        //
        // Add account moves between those routes with a client-side
        // navigation (a history push plus a synthetic popstate), and the
        // top-document router keeps this element mounted across account
        // routes, so nothing else re-reads the location. The ceremony
        // that creates or links the account runs in the registration
        // cluster, which says so with `ACCOUNT_CHANGED` rather than
        // reaching in here. Both are the same answer: re-derive from
        // what the worker reports now.
        for event in ["popstate", crate::register_dialog::ACCOUNT_CHANGED] {
            let host = this.clone();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                if host.is_connected() {
                    load_status(host.clone());
                }
            });
            if let Some(window) = window() {
                let _ = window
                    .add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
            }
            closure.forget();
        }
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

fn ensure_stylesheet() {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(include_str!("account.css")));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
    }
}

async fn service(host: &HtmlElement) -> Result<String, String> {
    if let Some(attribute) = host
        .get_attribute("service")
        .filter(|value| !value.is_empty())
    {
        return Ok(attribute);
    }
    // The deployment serving this page. It marks the account as
    // attached; nothing calls it as a service any more.
    proposed_remote()
}

fn input(host: &HtmlElement, selector: &str) -> Result<String, String> {
    let input: HtmlInputElement = host
        .query_selector(selector)
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into().ok())
        .ok_or_else(|| format!("missing form field {selector}"))?;
    let value = input.value().trim().to_string();
    if value.is_empty() {
        Err(format!("{} is required", input.name()))
    } else if !input.check_validity() {
        Err(input
            .validation_message()
            .ok()
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| format!("{} is invalid", input.name())))
    } else {
        Ok(value)
    }
}

/// The mode that shows no panel at all.
///
/// Reached when the deployment cannot be resolved, so there is no
/// service to sign into and nothing any panel could usefully offer. The
/// error text is the whole message. Named rather than left to fall out
/// of matching no panel, because that made "every panel hidden" the
/// behaviour of any unrecognised mode — a typo included — instead of a
/// state someone chose.
const NO_PANEL_MODE: &str = "blocked";

fn set_mode(host: &HtmlElement, mode: &str) {
    debug_assert!(
        mode == NO_PANEL_MODE || ["choice", "create", "link", "handoff", "success"].contains(&mode),
        "unknown account mode {mode:?} would hide every panel"
    );
    let _ = host.set_attribute("data-mode", mode);
    for (name, selector) in [
        ("choice", "#account-choice"),
        ("create", "#account-create"),
        ("link", "#account-link"),
        ("handoff", "#account-handoff"),
        ("success", "#account-success"),
    ] {
        if let Ok(Some(panel)) = host.query_selector(selector) {
            if name == mode {
                let _ = panel.remove_attribute("hidden");
            } else {
                let _ = panel.set_attribute("hidden", "");
            }
        }
    }
}

fn select_account_tab(host: &HtmlElement, name: &str, focus: bool) {
    let Ok(tabs) = host.query_selector_all("[data-account-tab]") else {
        return;
    };
    for index in 0..tabs.length() {
        let Some(tab) = tabs
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        else {
            continue;
        };
        let selected = tab.get_attribute("data-account-tab").as_deref() == Some(name);
        let _ = tab.set_attribute("aria-selected", if selected { "true" } else { "false" });
        let _ = tab.set_attribute("tabindex", if selected { "0" } else { "-1" });
        if selected && focus {
            let _ = tab.focus();
        }
    }
    let Ok(panes) = host.query_selector_all("[data-account-pane]") else {
        return;
    };
    for index in 0..panes.length() {
        let Some(pane) = panes
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        if pane.get_attribute("data-account-pane").as_deref() == Some(name) {
            let _ = pane.remove_attribute("hidden");
        } else {
            let _ = pane.set_attribute("hidden", "");
        }
    }
}

fn set_busy(host: &HtmlElement, busy: bool, status: &str) {
    let _ = host.set_attribute("data-busy", if busy { "true" } else { "false" });
    let _ = host.set_attribute("aria-busy", if busy { "true" } else { "false" });

    if busy
        && host
            .query_selector("[data-initiating]")
            .ok()
            .flatten()
            .is_none()
        && let Some(active) = window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
        && host.contains(Some(active.as_ref()))
        && active.tag_name() == "BUTTON"
    {
        let _ = active.set_attribute("data-initiating", "true");
        let _ = active.set_attribute(
            "data-idle-label",
            &active.text_content().unwrap_or_default(),
        );
    }
    if busy
        && host
            .query_selector("[data-initiating]")
            .ok()
            .flatten()
            .is_none()
    {
        let fallback = match host.get_attribute("data-mode").as_deref() {
            Some("create") => Some("#account-create-submit"),
            Some("link") => Some("#account-link-submit"),
            Some("handoff") => Some("#account-handoff-submit"),
            _ => None,
        };
        if let Some(selector) = fallback
            && let Ok(Some(action)) = host.query_selector(selector)
        {
            let _ = action.set_attribute("data-initiating", "true");
            let _ = action.set_attribute(
                "data-idle-label",
                &action.text_content().unwrap_or_default(),
            );
        }
    }
    if let Ok(Some(initiating)) = host.query_selector("[data-initiating]") {
        if busy && !status.is_empty() {
            initiating.set_text_content(Some(status));
        } else if !busy {
            if let Some(label) = initiating.get_attribute("data-idle-label") {
                initiating.set_text_content(Some(&label));
            }
            let _ = initiating.remove_attribute("data-idle-label");
            let _ = initiating.remove_attribute("data-initiating");
        }
    }

    if let Ok(buttons) = host.query_selector_all("button") {
        for index in 0..buttons.length() {
            if let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlButtonElement>().ok())
            {
                button.set_disabled(
                    busy || button.has_attribute("data-account-deletion-unavailable"),
                );
            }
        }
    }
    if let Ok(inputs) = host.query_selector_all("input") {
        for index in 0..inputs.length() {
            if let Some(input) = inputs
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlInputElement>().ok())
            {
                let account_edit_blocked =
                    input.id() == "account-display-name" && host.has_attribute(ACCOUNT_NOT_READY);
                if busy
                    || account_edit_blocked
                    || input.get_attribute("aria-busy").as_deref() != Some("true")
                {
                    input.set_disabled(busy || account_edit_blocked);
                }
            }
        }
    }
    if let Ok(links) = host.query_selector_all("a") {
        for index in 0..links.length() {
            let Some(link) = links
                .item(index)
                .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            if busy {
                let _ = link.set_attribute("aria-disabled", "true");
                let _ = link.set_attribute("tabindex", "-1");
            } else {
                let _ = link.remove_attribute("aria-disabled");
                let _ = link.remove_attribute("tabindex");
            }
        }
    }
    if let Ok(Some(element)) = host.query_selector("#account-working") {
        element.set_text_content((!status.is_empty()).then_some(status));
    }
    if !busy {
        update_confirmation_arming(host);
    }
}

fn show_error(host: &HtmlElement, message: impl AsRef<str>) {
    let _ = host.remove_attribute(ACCOUNT_GUIDANCE_SHOWN);
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(Some(message.as_ref()));
        let _ = error.remove_attribute("hidden");
        let _ = error.remove_attribute("data-flash");
        let _ = error.set_attribute("data-flash", "true");
        if let Ok(error) = error.dyn_into::<HtmlElement>() {
            let _ = error.focus();
        }
    }
}

fn clear_error(host: &HtmlElement) {
    let _ = host.remove_attribute(ACCOUNT_GUIDANCE_SHOWN);
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(None);
        let _ = error.set_attribute("hidden", "");
    }
}

fn log_action_error(action: AccountAction, detail: &str) {
    web_sys::console::error_1(&format!("account {action:?} failed: {detail}").into());
}

fn show_action_error(host: &HtmlElement, action: AccountAction, detail: &str) {
    log_action_error(action, detail);
    let problem = user_error::problem_from_diagnostic(action, detail);
    show_error(host, problem.message);
}

/// [`show_action_error`] for a failed passkey ceremony, which may carry
/// the service's own reason for refusing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn show_ceremony_error(
    host: &HtmlElement,
    action: AccountAction,
    error: &crate::custody_relay::CeremonyError,
) {
    log_action_error(action, &error.message);
    let problem = user_error::ceremony_problem(action, error);
    show_error(host, problem.message);
}

fn show_api_error(host: &HtmlElement, action: AccountAction, error: &crate::error::TonkUiError) {
    log_action_error(action, &error.to_string());
    let problem = user_error::api_problem(action, error);
    show_error(host, problem.message);
}

fn show_automatic_api_error(
    host: &HtmlElement,
    action: AccountAction,
    error: &crate::error::TonkUiError,
) {
    log_action_error(action, &error.to_string());
    let problem = user_error::api_problem(action, error);
    show_error(host, problem.message);
}

fn show_confirmation_api_error(
    host: &HtmlElement,
    action: AccountAction,
    error: &crate::error::TonkUiError,
) {
    log_action_error(action, &error.to_string());
    let problem = user_error::api_problem(action, error);
    show_confirmation_error(host, problem.message);
}

fn show_display_name_api_error(
    host: &HtmlElement,
    action: AccountAction,
    error: &crate::error::TonkUiError,
) {
    log_action_error(action, &error.to_string());
    let problem = user_error::api_problem(action, error);
    show_display_name_error(host, &problem.message);
}

fn show_account_guidance(host: &HtmlElement, message: &str) {
    let unchanged = host.has_attribute(ACCOUNT_GUIDANCE_SHOWN)
        && host
            .query_selector("#account-error")
            .ok()
            .flatten()
            .and_then(|error| error.text_content())
            .as_deref()
            == Some(message);
    if unchanged {
        return;
    }
    show_error(host, message);
    let _ = host.set_attribute(ACCOUNT_GUIDANCE_SHOWN, "true");
}

fn clear_account_guidance(host: &HtmlElement) {
    if host.has_attribute(ACCOUNT_GUIDANCE_SHOWN) {
        clear_error(host);
    }
}

fn focus_input(host: &HtmlElement, selector: &str) {
    if let Ok(Some(input)) = host.query_selector(selector)
        && let Ok(input) = input.dyn_into::<HtmlInputElement>()
    {
        let _ = input.focus();
    }
}

fn show_success(host: &HtmlElement) {
    clear_error(host);
    set_busy(host, false, "");
    set_mode(host, "success");
    select_account_tab(
        host,
        if revoke_target_from_url().is_some() {
            "devices"
        } else {
            "account"
        },
        false,
    );
    load_summary(host.clone());
    if host.has_attribute(ACCOUNT_NOT_READY) {
        set_text(
            host,
            "#account-device-list",
            "Available after email verification.",
        );
    } else {
        load_devices(host.clone());
    }
    load_profiles(host.clone());
    load_activation_notice(host.clone());
}

/// Surface a pending customer activation on the dashboard. Quiet on
/// every other answer: an active customer needs no notice, and a
/// deployment without registration should not decorate the panel with
/// its absence.
/// Say when the account's backup is still on its way, and settle the
/// outcome in the DOM. The ceremony pre-signed the publish and the
/// worker drains it on activation — no interaction is needed — but the
/// drain is asynchronous, so keep watching until it lands rather than
/// leaving a notice frozen at whatever the first look saw. Each round's
/// customer probe is also a retry: the worker replays pending work on
/// an Active answer.
///
/// `data-backup` on the host is the settled answer — "done" once the
/// queue holds no publish, "stuck" when it still does after a bounded
/// wait (a reload retries). The e2e suite waits on it before running
/// any ceremony that resolves the published cell.
async fn note_pending_backup(host: &HtmlElement) {
    // A minute of second-spaced checks: far beyond a healthy drain,
    // bounded so a genuinely stuck queue is reported, not spun on.
    for round in 0..60 {
        let waiting = crate::api::pending_work().await.is_ok_and(|queue| {
            queue.entries().iter().any(|work| {
                matches!(
                    work,
                    tonk_account::pending::PendingWork::PublishCustody { .. }
                )
            })
        });
        if !waiting {
            let _ = hide(host, "#account-backup-notice");
            let _ = host.set_attribute("data-backup", "done");
            return;
        }
        set_text(
            host,
            "#account-backup-notice",
            "Finishing your account's backup…",
        );
        let _ = show(host, "#account-backup-notice");
        if round > 0 {
            let _ = crate::api::customer_state().await;
        }
        wait_for(1_000).await;
    }
    let _ = host.set_attribute("data-backup", "stuck");
}

/// Unhide the element `selector` names.
fn show(host: &HtmlElement, selector: &str) -> Option<()> {
    host.query_selector(selector)
        .ok()
        .flatten()?
        .remove_attribute("hidden")
        .ok()
}

/// Hide the element `selector` names.
fn hide(host: &HtmlElement, selector: &str) -> Option<()> {
    host.query_selector(selector)
        .ok()
        .flatten()?
        .set_attribute("hidden", "")
        .ok()
}

fn load_activation_notice(host: HtmlElement) {
    spawn_local(async move {
        if !wants_enrollment().await {
            set_text(&host, "#account-registration-value", "Not used here");
            // Nothing is ever queued without registration; settle the
            // backup state so a waiter has an answer here too.
            let _ = host.set_attribute("data-backup", "done");
            return;
        }
        let (mut attempt, result) = crate::account_observability::observe(
            AccountAction::LoadRegistration,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::Automatic,
            tonk_analytics::account::AccountState::Unknown,
            crate::api::customer_state(),
        )
        .await;
        let mut state = match result {
            Ok(state) => {
                attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                state
            }
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::LoadRegistration, &error);
                attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
                if host.has_attribute(ACCOUNT_NOT_READY) {
                    show_automatic_api_error(&host, AccountAction::LoadRegistration, &error);
                } else {
                    log_action_error(AccountAction::LoadAccount, &error.to_string());
                }
                // One failed probe is not an answer. The dashboard often
                // loads while the worker is still hydrating the account
                // it navigated in from — activation hands the tab
                // straight here — and returning froze the panel on
                // "reload to retry" with `data-backup` never settled,
                // so nothing downstream (the pending-backup drain, the
                // e2e waiter) ever ran. Fall into the retry loop below
                // instead; `Value::Null` marks the probe as unanswered,
                // which also keeps the enrollment fallthrough from
                // treating a failure as an authoritative absence.
                serde_json::Value::Null
            }
        };
        // A linked account the access service does not know is one that
        // predates registration (or the service's control state was
        // reset). This signed-in browser is the only party that can fix
        // that — registration is web-only — so enroll right here, with
        // the device-chained deposit since no ceremony is at hand, and
        // fall through to the ordinary pending notice.
        if !state.is_null()
            && state["status"].is_null()
            && crate::deployment::get()
                .await
                .is_ok_and(|config| config.service_did.is_some())
        {
            let (mut enroll_attempt, result) = crate::account_observability::observe(
                AccountAction::LoadAccount,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::Recovery,
                tonk_analytics::account::AccountState::RegisteredUnready,
                crate::api::enroll_customer(None),
            )
            .await;
            match result {
                // Enrollment is a command, so this returns once the
                // transient is committed, not once the service answers.
                // Re-reading here would race the handler and paint a
                // state already superseded; the row's subscription is
                // what shows the outcome, whenever it lands.
                Ok(()) => enroll_attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    tonk_analytics::account::AccountOutcome::success(),
                ),
                Err(error) => {
                    let problem = user_error::api_problem(AccountAction::LoadAccount, &error);
                    enroll_attempt.finish(
                        tonk_analytics::account::Stage::RemoteCommit,
                        problem.outcome,
                    );
                    set_text(
                        &host,
                        "#account-registration-value",
                        "Not registered — reload to retry",
                    );
                    show_api_error(&host, AccountAction::LoadAccount, &error);
                    // Settled the same way the exhausted probe is:
                    // a reload retries, so say so.
                    let _ = host.set_attribute("data-backup", "stuck");
                    return;
                }
            }
        }
        // Render whatever the state says now, and while an activation is
        // pending keep asking: the link is opened in another tab (or on
        // the phone the email is on), and a dashboard that only learns
        // about it on a manual reload looks stuck at "pending" long
        // after the account is live.
        // A fresh enrollment can be briefly absent even though the command
        // succeeded. Do not classify that transient absence as "not
        // registered", and do not discard the only marker that says the
        // account repository is still unsafe to edit.
        let mut last_probe_error: Option<String> = None;
        for unsettled_round in 0..=20 {
            if !state["status"].is_null() {
                break;
            }
            if unsettled_round == 20 {
                set_text(
                    &host,
                    "#account-registration-value",
                    "Unavailable — reload to retry",
                );
                if host.has_attribute(ACCOUNT_NOT_READY) {
                    show_account_guidance(&host, ACCOUNT_STATUS_UNKNOWN);
                }
                if let Some(error) = last_probe_error {
                    log_action_error(AccountAction::LoadAccount, &error);
                }
                // The settled answer for a probe that never came:
                // "stuck" is the state a reload retries, which is what
                // the message asks for and what the e2e waiter does —
                // leaving the attribute unset left both waiting on
                // nothing.
                let _ = host.set_attribute("data-backup", "stuck");
                return;
            }
            wait_for(500).await;
            match crate::api::customer_state().await {
                Ok(fresh) => state = fresh,
                Err(error) => {
                    last_probe_error = Some(error.to_string());
                }
            }
        }

        loop {
            render_registration(&host, &state).await;
            if state["status"].as_str() != Some("Registered") {
                return;
            }
            wait_for(5_000).await;
            if let Ok(fresh) = crate::api::customer_state().await {
                state = fresh;
            }
        }
    });
}

/// Render the registration facts row, the activation banner, and the
/// resend button from one customer state, so the pending → active flip
/// also clears what pending showed.
async fn render_registration(host: &HtmlElement, state: &serde_json::Value) {
    // The facts row always answers; the banner below only nags while
    // an activation is actually pending.
    let label = match state["status"].as_str() {
        Some("Active") => "Active",
        Some("Registered") => "Waiting for email confirmation",
        Some("Suspended") => "Suspended",
        _ => "Not registered",
    };
    set_text(host, "#account-registration-value", label);
    // An unhydrated repository must remain read-only until it reaches Ready.
    // The customer answer explains why without exposing that implementation
    // state or prescribing a reload that cannot satisfy email confirmation.
    if host.has_attribute(ACCOUNT_NOT_READY) {
        match state["status"].as_str() {
            Some("Registered") => show_account_guidance(host, VERIFY_EMAIL),
            Some("Active") => {
                show_account_guidance(host, ACCOUNT_SETUP_FINISHING);
                let host = host.clone();
                spawn_local(async move { finish_account_readiness(&host).await });
            }
            Some("Suspended") => show_account_guidance(host, ACCOUNT_SYNC_PAUSED),
            _ => {}
        }
    }
    // The worker drains the queued backup itself once activation
    // lands — the ceremony pre-signed the publish. Nothing to raise;
    // just say so while it is still on its way. Detached: the watch is
    // bounded but long, and the banner cleanup below must not wait on
    // it.
    if state["status"].as_str() == Some("Active") {
        let host = host.clone();
        spawn_local(async move { note_pending_backup(&host).await });
    }
    if state["status"].as_str() != Some("Registered") {
        let _ = hide(host, "#account-activation-notice");
        let _ = hide(host, "#account-resend-activation");
        return;
    }
    let message = match state["email"].as_str() {
        Some(email) => {
            format!("Sync activation pending: open the link we emailed to {email}.")
        }
        None => "Sync activation pending: open the link in your activation email.".to_string(),
    };
    set_text(host, "#account-activation-notice", &message);
    let _ = show(host, "#account-activation-notice");
    // The way out of a stuck Registered: enrollment is idempotent
    // while Registered and resends the link, which is also the
    // recovery for one that expired.
    let _ = show(host, "#account-resend-activation");
}

/// Disable the resend button for the interval the service enforces,
/// showing what is left.
///
/// The countdown is local. The service refuses a too-soon resend
/// silently — telling a caller to wait would tell it the address is
/// registered — so the wait is displayed by the page that pressed the
/// button rather than reported by the service.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn count_down_resend(host: HtmlElement) {
    use tonk_account::customer::RESEND_INTERVAL_SECONDS;

    spawn_local(async move {
        let Ok(Some(button)) = host.query_selector("#account-resend-activation") else {
            return;
        };
        let label = button
            .text_content()
            .unwrap_or_else(|| "Resend".to_string());
        let _ = button.set_attribute("disabled", "");
        for remaining in (1..=RESEND_INTERVAL_SECONDS).rev() {
            button.set_text_content(Some(&format!("Resend in {remaining}s")));
            wait_for(1_000).await;
            // The panel may have moved on — activated, signed out —
            // while this was counting.
            if !button.is_connected() {
                return;
            }
        }
        button.set_text_content(Some(&label));
        let _ = button.remove_attribute("disabled");
    });
}

/// Keep retrying the local account pull after activation until authoritative
/// account settings are safe to edit. A later unrelated error owns the banner
/// and is not cleared by this background task.
async fn finish_account_readiness(host: &HtmlElement) {
    for round in 0..20 {
        match crate::api::account_status().await {
            Ok(AccountStatus::Registered {
                account_state: AccountStateStatus::Ready,
                ..
            }) => {
                let _ = host.remove_attribute(ACCOUNT_NOT_READY);
                clear_account_guidance(host);
                // The whole dashboard read while unhydrated, not only
                // these two: the summary's passkey facts answer None
                // until the account state is Ready, so the row it
                // rendered is a blank that never heals unless it is
                // re-read here with the rest.
                load_summary(host.clone());
                load_profiles(host.clone());
                load_devices(host.clone());
                return;
            }
            Ok(AccountStatus::Registered { .. }) => {}
            Ok(_) => return,
            Err(error) if round == 19 => {
                log_action_error(AccountAction::LoadAccount, &error.to_string());
            }
            Err(_) => {}
        }
        wait_for(1_000).await;
    }
}

/// Resolve after `ms` milliseconds.
async fn wait_for(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(win) = window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn set_text(host: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = host.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

fn requested_space_deletion() -> Option<String> {
    let href = window()?.location().href().ok()?;
    url::Url::parse(&href)
        .ok()?
        .query_pairs()
        .find_map(|(name, value)| (name == "delete-space").then(|| value.into_owned()))
        .filter(|value| !value.is_empty())
}

fn confirmation(host: &HtmlElement) -> Option<Confirmation> {
    Reflect::get(host.as_ref(), &CONFIRMATION.into())
        .ok()
        .filter(|value| !value.is_null() && !value.is_undefined())
        .and_then(|value| serde_wasm_bindgen::from_value(value).ok())
}

fn update_confirmation_arming(host: &HtmlElement) {
    let armed = match confirmation(host) {
        Some(Confirmation::Delete { plan, .. }) => {
            let email_matches = host
                .query_selector("#account-delete-email")
                .ok()
                .flatten()
                .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
                .is_some_and(|input| input.value().trim() == plan.email);
            let understood = host
                .query_selector("#account-delete-understood")
                .ok()
                .flatten()
                .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
                .is_some_and(|input| input.checked());
            email_matches && understood
        }
        Some(_) => true,
        None => false,
    };
    if let Ok(Some(button)) = host.query_selector("#account-delete-submit")
        && let Ok(button) = button.dyn_into::<HtmlButtonElement>()
    {
        button.set_disabled(!armed || host.get_attribute("data-busy").as_deref() == Some("true"));
    }
}

fn close_confirmation(host: &HtmlElement) {
    if host.get_attribute("data-busy").as_deref() == Some("true") {
        return;
    }
    if let Ok(Some(surface)) = host.query_selector("#account-confirmation") {
        let _ = surface.set_attribute("hidden", "");
    }
    let _ = Reflect::delete_property(host.as_ref(), &CONFIRMATION.into());
    if let Ok(return_focus) = Reflect::get(host.as_ref(), &CONFIRMATION_RETURN_FOCUS.into())
        && let Ok(return_focus) = return_focus.dyn_into::<HtmlElement>()
    {
        let _ = return_focus.focus();
    }
    let _ = Reflect::delete_property(host.as_ref(), &CONFIRMATION_RETURN_FOCUS.into());
}

fn cancel_confirmation(host: &HtmlElement) {
    let action = match confirmation(host) {
        Some(Confirmation::SignOut) => Some(AccountAction::SignOut),
        Some(Confirmation::Revoke { .. }) => Some(AccountAction::RevokeDevice),
        Some(Confirmation::Delete {
            requested_space, ..
        }) => Some(if requested_space.is_some() {
            AccountAction::DeleteSpace
        } else {
            AccountAction::DeleteAccount
        }),
        None => None,
    };
    if let Some(action) = action {
        let mut attempt = crate::account_observability::WebAccountAttempt::start(
            action,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::User,
            tonk_analytics::account::AccountState::Ready,
        );
        attempt.finish(
            tonk_analytics::account::Stage::Input,
            tonk_analytics::account::AccountOutcome::cancelled(),
        );
    }
    close_confirmation(host);
}

fn open_confirmation(host: &HtmlElement, pending: Confirmation) -> Result<(), String> {
    if confirmation(host).is_none()
        && let Some(active) = window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
    {
        let _ = Reflect::set(
            host.as_ref(),
            &CONFIRMATION_RETURN_FOCUS.into(),
            active.as_ref(),
        );
    }
    let value = serde_wasm_bindgen::to_value(&pending)
        .map_err(|_| "could not retain confirmation state".to_string())?;
    Reflect::set(host.as_ref(), &CONFIRMATION.into(), &value)
        .map_err(|_| "could not retain confirmation state".to_string())?;

    if let Ok(Some(result)) = host.query_selector("#account-confirm-result-back") {
        let _ = result.set_attribute("hidden", "");
    }
    if let Ok(Some(error)) = host.query_selector("#account-confirm-error") {
        error.set_text_content(None);
        let _ = error.set_attribute("hidden", "");
    }
    if let Ok(Some(foot)) = host.query_selector(".account__dialog-foot") {
        let _ = foot.remove_attribute("hidden");
    }
    let (title, body, action, destructive) = match &pending {
        Confirmation::SignOut => (
            "sign out on this device",
            "Your existing spaces will stay on this device, but account syncing will stop until you sign in again.",
            "sign out",
            false,
        ),
        Confirmation::Revoke { self_revoke, .. } => (
            "remove device access",
            if *self_revoke {
                "This permanently disconnects this device from your Tonk account. To use it again, sign in to add it as a new device."
            } else {
                "This permanently disconnects the selected device from your Tonk account. To use it again, sign in to add a new device."
            },
            "remove access",
            false,
        ),
        Confirmation::Delete {
            plan,
            requested_space,
        } => {
            render_deletion_plan(host, plan, requested_space.as_deref())?;
            (
                if requested_space.is_some() {
                    "delete owned space permanently"
                } else {
                    "delete account permanently"
                },
                if requested_space.is_some() {
                    "This deletes only the selected owned space's hosted content from Tonk services. Your account and every other space remain."
                } else {
                    "This is not sign out. Tonk will permanently delete hosted content for owned spaces, account backups, device registrations, and the account record."
                },
                if requested_space.is_some() {
                    "delete selected owned space"
                } else {
                    "delete owned spaces and account"
                },
                true,
            )
        }
    };
    set_text(host, "#account-confirm-title", title);
    set_text(host, "#account-confirm-body", body);
    set_text(host, "#account-delete-submit", action);
    if let Ok(Some(arming)) = host.query_selector("#account-delete-arming") {
        if destructive {
            let _ = arming.remove_attribute("hidden");
        } else {
            let _ = arming.set_attribute("hidden", "");
        }
    }
    if let Ok(Some(surface)) = host.query_selector("#account-confirmation") {
        let _ = surface.remove_attribute("hidden");
    }
    update_confirmation_arming(host);
    let focus = if destructive {
        "#account-delete-email"
    } else {
        "#account-confirm-cancel"
    };
    focus_input_or_button(host, focus);
    Ok(())
}

fn show_confirmation_error(host: &HtmlElement, message: impl AsRef<str>) {
    if let Ok(Some(error)) = host.query_selector("#account-confirm-error") {
        error.set_text_content(Some(message.as_ref()));
        let _ = error.remove_attribute("hidden");
        if let Ok(error) = error.dyn_into::<HtmlElement>() {
            let _ = error.focus();
        }
    }
}

fn show_confirmation_validation_error(
    host: &HtmlElement,
    action: AccountAction,
    message: impl AsRef<str>,
) {
    let mut attempt = crate::account_observability::WebAccountAttempt::start(
        action,
        tonk_analytics::account::Surface::Settings,
        tonk_analytics::account::Trigger::User,
        tonk_analytics::account::AccountState::Ready,
    );
    attempt.finish(
        tonk_analytics::account::Stage::Input,
        tonk_analytics::account::AccountOutcome::terminal_failure(
            tonk_analytics::account::FailureKind::InvalidInput,
        ),
    );
    show_confirmation_error(host, message);
}

fn render_confirmation_result(host: &HtmlElement, message: &str) {
    set_busy(host, false, "");
    set_text(host, "#account-confirm-title", "complete");
    set_text(host, "#account-confirm-body", message);
    if let Ok(Some(arming)) = host.query_selector("#account-delete-arming") {
        let _ = arming.set_attribute("hidden", "");
    }
    if let Ok(Some(foot)) = host.query_selector(".account__dialog-foot") {
        let _ = foot.set_attribute("hidden", "");
    }
    if let Ok(Some(result)) = host.query_selector("#account-confirm-result-back") {
        let _ = result.remove_attribute("hidden");
    }
    focus_input_or_button(host, "#account-confirm-result-back");
}

fn focus_input_or_button(host: &HtmlElement, selector: &str) {
    if let Ok(Some(element)) = host.query_selector(selector)
        && let Ok(element) = element.dyn_into::<HtmlElement>()
    {
        let _ = element.focus();
    }
}

fn configure_deletion_entry(host: &HtmlElement) {
    if requested_space_deletion().is_none() {
        return;
    }
    if let Ok(Some(action)) = host.query_selector("#account-delete-review")
        && let Ok(action) = action.dyn_into::<HtmlButtonElement>()
    {
        let _ = action.remove_attribute("data-account-deletion-unavailable");
        action.set_disabled(false);
    }
    set_text(
        host,
        "#account-delete-title",
        "Delete owned space permanently",
    );
    set_text(
        host,
        "#account-delete-description",
        "This deletes one selected space's hosted content from Tonk services. Your account and every other space remain.",
    );
    set_text(
        host,
        "#account-delete-boundary",
        "Tonk cannot erase copies that other devices have already replicated.",
    );
    set_text(
        host,
        "#account-delete-review",
        "Review selected space deletion",
    );
    set_text(
        host,
        "#account-delete-understood-label",
        "I understand that this owned space's hosted content will be permanently deleted from Tonk services and cannot be recovered by Tonk.",
    );
}

fn render_deletion_plan(
    host: &HtmlElement,
    plan: &AccountDeletionPlan,
    requested: Option<&str>,
) -> Result<(), String> {
    let panel = host
        .query_selector("#account-delete-arming")
        .ok()
        .flatten()
        .ok_or_else(|| "missing deletion review panel".to_string())?;
    let _ = panel.remove_attribute("hidden");
    let visible: Vec<_> = plan
        .spaces
        .iter()
        .filter(|space| requested.is_none_or(|subject| space.subject == subject))
        .collect();
    if let Some(subject) = requested
        && visible.is_empty()
    {
        return Err(format!(
            "{subject} is not an owned hosted space for this account"
        ));
    }
    set_text(
        host,
        "#account-delete-title",
        if requested.is_some() {
            "Delete owned space permanently"
        } else {
            "Delete account permanently"
        },
    );
    set_text(
        host,
        "#account-delete-submit",
        if requested.is_some() {
            "Delete selected owned space"
        } else {
            "Delete owned spaces and account"
        },
    );
    set_text(
        host,
        "#account-delete-scope",
        &if requested.is_some() {
            format!(
                "This will permanently delete the selected owned hosted space. {} joined space{} will be left intact.",
                plan.joined_spaces,
                if plan.joined_spaces == 1 { "" } else { "s" },
            )
        } else {
            format!(
                "{} owned hosted space{} will be deleted. {} joined space{} will be left intact.",
                plan.spaces.len(),
                if plan.spaces.len() == 1 { "" } else { "s" },
                plan.joined_spaces,
                if plan.joined_spaces == 1 { "" } else { "s" },
            )
        },
    );
    let list = host
        .query_selector("#account-delete-spaces")
        .ok()
        .flatten()
        .ok_or_else(|| "missing deletion space list".to_string())?;
    list.set_inner_html("");
    let document = window()
        .and_then(|window| window.document())
        .ok_or_else(|| "document is unavailable".to_string())?;
    for space in &visible {
        let item = document
            .create_element("li")
            .map_err(|_| "could not render deletion space".to_string())?;
        let label = space.name.as_deref().unwrap_or(&space.subject);
        // A listed space is either being purged or waiting to be; a
        // finished deletion leaves no record to show.
        let state = match space.deleting_since {
            Some(_) => "deleting",
            None => "scheduled",
        };
        item.set_text_content(Some(&format!("{label} — {state}")));
        let _ = list.append_child(&item);
    }
    if visible.is_empty() {
        let item = document
            .create_element("li")
            .map_err(|_| "could not render empty deletion plan".to_string())?;
        item.set_text_content(Some("No owned hosted spaces"));
        let _ = list.append_child(&item);
    }
    if let Ok(Some(blocked)) = host.query_selector("#account-delete-blocked") {
        // Nothing can block deletion any more: authority is the
        // account's own chain, not a per-space recovered artifact.
        let _ = blocked.set_attribute("hidden", "");
        blocked.set_text_content(None);
    }
    if let Ok(Some(email)) = host.query_selector("#account-delete-email")
        && let Ok(email) = email.dyn_into::<HtmlInputElement>()
    {
        email.set_value("");
    }
    if let Ok(Some(check)) = host.query_selector("#account-delete-understood")
        && let Ok(check) = check.dyn_into::<HtmlInputElement>()
    {
        check.set_checked(false);
    }
    Ok(())
}

fn render_summary(host: &HtmlElement, summary: &tonk_worker_api::AccountSummary) {
    // The passkey facts come from the account repository, which answers even
    // with the account service unreachable. The verified address has no home
    // outside that service, so it alone goes unavailable.
    set_text(
        host,
        "#account-email-value",
        summary.email.as_deref().unwrap_or("Unavailable"),
    );
    match &summary.passkey {
        Some(passkey) => {
            let date = js_sys::Date::new(&JsValue::from_f64(passkey.created_at as f64 * 1000.0))
                .to_locale_date_string("default", &JsValue::UNDEFINED);
            set_text(host, "#account-passkey-created-value", &String::from(date));
            set_text(host, "#account-passkey-device-value", &passkey.created_on);
        }
        None => {
            set_text(host, "#account-passkey-created-value", "Unavailable");
            set_text(host, "#account-passkey-device-value", "Unavailable");
            set_text(
                host,
                "#account-passkey-detail",
                "This passkey was made before Tonk started recording creation details. Tonk cannot reliably reconstruct them.",
            );
        }
    }
}

fn load_summary(host: HtmlElement) {
    let mut attempt = crate::account_observability::WebAccountAttempt::start(
        AccountAction::LoadAccount,
        tonk_analytics::account::Surface::Settings,
        tonk_analytics::account::Trigger::Automatic,
        tonk_analytics::account::AccountState::Unknown,
    );
    spawn_local(async move {
        match crate::api::account_summary().await {
            Ok(summary) => {
                attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                render_summary(&host, &summary)
            }
            Err(error) => {
                for selector in [
                    "#account-email-value",
                    "#account-passkey-created-value",
                    "#account-passkey-device-value",
                ] {
                    set_text(&host, selector, "Unavailable");
                }
                set_text(
                    &host,
                    "#account-passkey-detail",
                    "We couldn't load these account details. Check your connection and reload settings.",
                );
                log_action_error(AccountAction::LoadAccount, &error.to_string());
                let problem = user_error::api_problem(AccountAction::LoadAccount, &error);
                attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
            }
        }
    });
}

/// Land a signed-in device where it was going.
///
/// [`crate::account_gate::finish`] returns to the `next` this page was
/// opened with; only without one does the success panel show, which is the
/// case where the user came to `/settings` on their own.
fn settle(host: &HtmlElement) {
    settle_with(host, crate::account_gate::finish());
}

/// Show the account dashboard to a device that already had an account.
///
/// Same shape as [`settle`], minus the return-to-`next` step. A gated user who
/// signed in on another tab still gets their interrupted operation replayed;
/// someone who opened their account settings from a space — the FAB's account
/// link carries `next` so its Back goes home — stays on the page they asked
/// for instead of being bounced straight back out of it.
fn settle_on_load(host: &HtmlElement) {
    settle_with(host, async { Ok(false) });
}

fn settle_with(
    host: &HtmlElement,
    finish: impl std::future::Future<Output = Result<bool, String>> + 'static,
) {
    show_success(host);
    let host = host.clone();
    spawn_local(async move {
        match finish.await {
            // Either it navigated — leave the panel exactly as it is rather
            // than repainting a page on its way out — or there was nothing to
            // return to, and the success panel is already the right answer.
            Ok(_) => {}
            Err(error) => show_action_error(&host, AccountAction::FinishPreviousAction, &error),
        }
    });
}

/// Render the device rows, marking the row whose DID is `own` — the
/// list itself is the same on every device, so "this device" is a
/// presentation attribute, the way an active tab is marked.
fn render_devices(host: &HtmlElement, devices: &[tonk_worker_api::AccountDevice], own: &str) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector("#account-device-list") else {
        return;
    };
    list.set_inner_html("");
    for device in devices {
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("class", "account__device-row");

        let Ok(identity) = document.create_element("div") else {
            continue;
        };
        let _ = identity.set_attribute("class", "account__device-identity");

        let Ok(name) = document.create_element("span") else {
            continue;
        };
        let _ = name.set_attribute("class", "account__device-name");
        name.set_text_content(Some(&device.name));
        let _ = identity.append_child(&name);

        let this_device = device.did == own;
        if this_device {
            let _ = item.set_attribute("data-this-device", "true");
            let Ok(marker) = document.create_element("span") else {
                continue;
            };
            let _ = marker.set_attribute("class", "account__device-current");
            marker.set_text_content(Some("this device"));
            let _ = identity.append_child(&marker);
        }

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__device-meta");
        let date = js_sys::Date::new(&JsValue::from_f64(device.created_at as f64 * 1000.0))
            .to_locale_date_string("default", &JsValue::UNDEFINED);
        meta.set_text_content(Some(&format!("added {}", String::from(date))));

        let _ = item.append_child(&identity);
        let _ = item.append_child(&meta);

        // Every listed device is removable: a row IS an active grant,
        // and this device's own account grant is enough to mint the
        // revocation — no passkey and no stored evidence involved.
        let Ok(button) = document.create_element("button") else {
            continue;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("class", "account__button account__button--remove");
        let _ = button.set_attribute("data-revoke", &device.did);
        let _ = button.set_attribute("aria-label", &format!("remove access for {}", device.name));
        if this_device {
            let _ = button.set_attribute("data-self-revoke", "true");
        }
        button.set_text_content(Some("remove access"));
        let _ = item.append_child(&button);
        let _ = list.append_child(&item);
    }
}

/// What a switcher row is titled: the roster's display name, else the
/// profile's storage name — never blank.
fn profile_row_label(entry: &tonk_worker_api::ProfileRosterEntry) -> String {
    entry
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| entry.email.clone().filter(|email| !email.trim().is_empty()))
        .unwrap_or_else(|| entry.profile_name.clone())
}

/// Render roster rows into `list_selector`. The active row is marked and
/// inert; every other row carries a Switch button with
/// `data-activate="{profile_name}"` for the shared click delegation.
/// `others_only` drops the active row entirely — the Choice panel's
/// compact list describes the profiles the user could go to, not the
/// fresh one they are on.
fn render_profile_rows(
    host: &HtmlElement,
    list_selector: &str,
    profiles: &tonk_worker_api::ProfilesResponse,
    others_only: bool,
) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector(list_selector) else {
        return;
    };
    list.set_inner_html("");
    for entry in &profiles.profiles {
        if others_only && entry.active {
            continue;
        }
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("class", "account__profile-row");
        if entry.active {
            let _ = item.set_attribute("data-active", "true");
            let _ = item.set_attribute("aria-current", "true");
        }

        let Ok(identity) = document.create_element("div") else {
            continue;
        };
        let _ = identity.set_attribute("class", "account__profile-identity");
        let Ok(name) = document.create_element("span") else {
            continue;
        };
        let _ = name.set_attribute("class", "account__profile-name");
        let label = profile_row_label(entry);
        name.set_text_content(Some(&label));
        let _ = identity.append_child(&name);
        if entry.active {
            let Ok(marker) = document.create_element("span") else {
                continue;
            };
            let _ = marker.set_attribute("class", "account__profile-current");
            marker.set_text_content(Some("current"));
            let _ = identity.append_child(&marker);
        }

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__profile-meta");
        meta.set_text_content(Some(match &entry.email {
            Some(email) => email,
            None if entry.provider.is_some() => "signed in",
            None => "local workspace",
        }));

        let _ = item.append_child(&identity);
        let _ = item.append_child(&meta);

        if !entry.active {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("class", "account__button account__button--switch");
            let _ = button.set_attribute("data-activate", &entry.profile_name);
            let _ = button.set_attribute("aria-label", &format!("Switch to {label}"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

/// Fill the signed-in dashboard's switcher section.
fn render_profiles(host: &HtmlElement, profiles: &tonk_worker_api::ProfilesResponse) {
    render_profile_rows(host, "#account-profile-list", profiles, false);
    if let Some(active) = profiles
        .profiles
        .iter()
        .find(|entry| entry.active || entry.profile_name == profiles.active)
        && let Ok(Some(input)) = host.query_selector("#account-display-name")
        && let Ok(input) = input.dyn_into::<HtmlInputElement>()
    {
        let label = profile_row_label(active);
        input.set_value(&label);
        let _ = input.set_attribute("data-confirmed-name", &label);
        let _ = input.remove_attribute("aria-busy");
        input.set_disabled(host.has_attribute(ACCOUNT_NOT_READY));
    }
}

fn show_display_name_error(host: &HtmlElement, message: &str) {
    if let Ok(Some(error)) = host.query_selector("#account-display-name-error") {
        error.set_text_content(Some(message));
        let _ = error.remove_attribute("hidden");
        let _ = error.remove_attribute("data-flash");
        let _ = error.set_attribute("data-flash", "true");
    }
}

fn commit_display_name(host: HtmlElement) {
    let Some(input) = host
        .query_selector("#account-display-name")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };
    if input.disabled()
        || host.has_attribute(ACCOUNT_NOT_READY)
        || input.get_attribute("aria-busy").as_deref() == Some("true")
    {
        return;
    }
    let confirmed = input
        .get_attribute("data-confirmed-name")
        .unwrap_or_default();
    let name = input.value().trim().to_owned();
    if name.is_empty() {
        input.set_value(&confirmed);
        return;
    }
    if name == confirmed {
        return;
    }
    input.set_disabled(true);
    let _ = input.set_attribute("aria-busy", "true");
    if let Ok(Some(error)) = host.query_selector("#account-display-name-error") {
        let _ = error.set_attribute("hidden", "");
    }
    spawn_local(async move {
        let (mut attempt, result) = crate::account_observability::observe(
            AccountAction::ChangeDisplayName,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::User,
            tonk_analytics::account::AccountState::Ready,
            crate::api::set_account_display_name(&name),
        )
        .await;
        match result {
            Ok(authoritative) => {
                attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                input.set_value(&authoritative);
                let _ = input.set_attribute("data-confirmed-name", &authoritative);
                set_text(
                    &host,
                    "#account-profile-list [data-active] .account__profile-name",
                    &authoritative,
                );
            }
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::ChangeDisplayName, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    problem.outcome,
                );
                input.set_value(&confirmed);
                show_display_name_api_error(&host, AccountAction::ChangeDisplayName, &error);
            }
        }
        input.set_disabled(host.has_attribute(ACCOUNT_NOT_READY));
        let _ = input.remove_attribute("aria-busy");
    });
}

/// Fill the Choice panel's compact switcher: the other roster entries,
/// plus the "Use a different account" affordance when this profile has a
/// persisted root — logging in here with another passkey would be
/// refused, so the way to another account is a fresh profile.
fn render_choice_profiles(
    host: &HtmlElement,
    profiles: &tonk_worker_api::ProfilesResponse,
    root_persisted: bool,
) {
    render_profile_rows(host, "#account-choice-profile-list", profiles, true);
    let has_others = profiles.profiles.iter().any(|entry| !entry.active);
    if let Ok(Some(section)) = host.query_selector("#account-choice-profiles") {
        if has_others {
            let _ = section.remove_attribute("hidden");
        } else {
            let _ = section.set_attribute("hidden", "");
        }
    }
    if let Ok(Some(button)) = host.query_selector("#account-use-different-account") {
        if root_persisted {
            let _ = button.remove_attribute("hidden");
        } else {
            let _ = button.set_attribute("hidden", "");
        }
    }
}

fn load_profiles(host: HtmlElement) {
    spawn_local(async move {
        let (mut attempt, result) = crate::account_observability::observe(
            AccountAction::LoadProfiles,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::Automatic,
            tonk_analytics::account::AccountState::Unknown,
            crate::api::list_profiles(),
        )
        .await;
        match result {
            Ok(profiles) => {
                attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                render_profiles(&host, &profiles)
            }
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::LoadProfiles, &error);
                attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
                show_automatic_api_error(&host, AccountAction::LoadProfiles, &error);
            }
        }
    });
}

fn load_choice_profiles(host: HtmlElement, root_persisted: bool) {
    spawn_local(async move {
        let (mut attempt, result) = crate::account_observability::observe(
            AccountAction::LoadProfiles,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::Automatic,
            tonk_analytics::account::AccountState::Unknown,
            crate::api::list_profiles(),
        )
        .await;
        match result {
            Ok(profiles) => {
                attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                render_choice_profiles(&host, &profiles, root_persisted)
            }
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::LoadProfiles, &error);
                attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
                show_automatic_api_error(&host, AccountAction::LoadProfiles, &error);
            }
        }
    });
}

/// Reload so every surface re-renders the profile the worker now serves.
fn reload_into_switched_profile(host: &HtmlElement) {
    match window().map(|window| window.location().reload()) {
        Some(Ok(())) => {}
        _ => {
            set_busy(host, false, "");
            load_status(host.clone());
        }
    }
}

fn revocation_status(
    _acknowledgement: &RevokeDeviceAcknowledgement,
    self_revoke: bool,
) -> &'static str {
    if self_revoke {
        "Access removed from this device."
    } else {
        "Access removed."
    }
}

fn disable_authority_actions(host: &HtmlElement) {
    let _ = host.set_attribute("data-authority", "revoked");
    for selector in ["[data-revoke]", "#account-unlink"] {
        let Ok(elements) = host.query_selector_all(selector) else {
            continue;
        };
        for index in 0..elements.length() {
            if let Some(element) = elements.item(index)
                && let Ok(button) = element.dyn_into::<HtmlButtonElement>()
            {
                button.set_disabled(true);
            }
        }
    }
}

/// The device DID named by a `?revoke=` deep link, if any.
///
/// The CLI cannot run a passkey ceremony, so `tonk account revoke` opens
/// this page pointed at the device it wants cut off. The page still asks
/// for confirmation and still runs the ceremony — the link chooses the
/// target, it does not authorize anything.
fn query_value(name: &str) -> Option<String> {
    let search = window()?.location().search().ok()?;
    let query = search.strip_prefix('?')?;
    query.split('&').find_map(|pair| {
        let (candidate, value) = pair.split_once('=')?;
        if candidate != name || value.is_empty() {
            return None;
        }
        Some(
            js_sys::decode_uri_component(value)
                .map(String::from)
                .unwrap_or_else(|_| value.to_owned()),
        )
    })
}

fn adding_account() -> bool {
    query_value("add").as_deref() == Some("1")
}

/// Rotate only once the user submits Create or Log in.
///
/// Merely opening the add-account choice is reversible navigation. Rotating
/// there persisted an empty profile even when the user immediately went
/// back. Once the ceremony is actually requested it still needs a fresh
/// profile first, because the resulting grant is addressed to that profile's
/// DID.
async fn prepare_added_profile() -> Result<(), String> {
    if !adding_account() {
        return Ok(());
    }
    crate::api::add_account_profile()
        .await
        .map_err(|error| error.to_string())?;
    consume_add_account_request();
    Ok(())
}

/// Remove only the add marker after rotation, retaining a safe return path.
/// A failed or cancelled ceremony can then be retried without rotating again.
fn consume_add_account_request() {
    let Some(window) = window() else { return };
    let Ok(pathname) = window.location().pathname() else {
        return;
    };
    let path = query_value("next").map_or(pathname.clone(), |next| {
        format!(
            "{pathname}?next={}",
            url::form_urlencoded::byte_serialize(next.as_bytes()).collect::<String>()
        )
    });
    let _ = window
        .history()
        .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&path)));
}

fn revoke_target_from_url() -> Option<String> {
    query_value("revoke")
}

/// Strip the query once the deep link has been acted on. Without this a
/// cancelled confirm re-fires on every later dashboard visit in this tab.
fn consume_revoke_target() {
    let Some(window) = window() else { return };
    if let Ok(path) = window.location().pathname() {
        let _ = window
            .history()
            .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&path)));
    }
}

fn load_devices(host: HtmlElement) {
    set_busy(&host, true, "Loading devices…");
    spawn_local(async move {
        let mut attempt = crate::account_observability::WebAccountAttempt::start(
            AccountAction::LoadDevices,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::Automatic,
            tonk_analytics::account::AccountState::Ready,
        );
        // Which row is this device is answered separately from the list:
        // the rows are shared facts, identical everywhere, and identity
        // is the one thing only this device can answer for itself.
        let own = match crate::api::identify().await {
            Ok(identity) => identity.did,
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::LoadDevices, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::LocalPreflight,
                    problem.outcome,
                );
                set_busy(&host, false, "");
                show_automatic_api_error(&host, AccountAction::LoadDevices, &error);
                return;
            }
        };
        attempt.checkpoint(tonk_analytics::account::Stage::LocalPreflight);
        match crate::api::account_devices().await {
            Ok(devices) => {
                attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                set_busy(&host, false, "");
                render_devices(&host, &devices, &own);
                if let Some(did) = revoke_target_from_url() {
                    consume_revoke_target();
                    match devices.iter().find(|device| device.did == did) {
                        Some(device) => {
                            begin_revoke(host.clone(), device.did.clone(), device.did == own)
                        }
                        None => show_error(
                            &host,
                            "The device in this link is no longer connected to this account.",
                        ),
                    }
                }
            }
            Err(error) => {
                let problem = user_error::api_problem(AccountAction::LoadDevices, &error);
                attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
                set_busy(&host, false, "");
                show_automatic_api_error(&host, AccountAction::LoadDevices, &error);
            }
        }
    });
}

/// Where a fresh page load lands once the account status is known.
#[derive(Debug, PartialEq, Eq)]
enum Landing {
    /// Straight to the dashboard and load its devices: a `?revoke=` deep link
    /// names a device, and the removal ceremony lives there.
    Devices,
    /// The signed-in dashboard.
    Success,
    /// The link/create choice, with a hint when a revoke deep link
    /// cannot proceed because this browser is not linked.
    Choice { revoke_hint: bool },
}

fn landing(
    account_state: Option<AccountStateStatus>,
    revoke_target: bool,
    adding_account: bool,
) -> Landing {
    if adding_account {
        return Landing::Choice { revoke_hint: false };
    }
    match (account_state, revoke_target) {
        (Some(_), true) => Landing::Devices,
        (Some(_), false) => Landing::Success,
        (None, revoke_hint) => Landing::Choice { revoke_hint },
    }
}

/// Re-read the account and repaint the panel, after a ceremony.
///
/// The ceremony runs in the registration cluster now, which sits over
/// this panel and finishes without telling it anything — so a panel
/// that was showing "link an account" when the cluster opened is still
/// showing it when the cluster closes, over an account that now exists.
/// The cluster calls this on its way out, and ONLY when its ceremony
/// announced an account: the read is the same one the panel does when
/// it boots, but a `Unregistered` answer here is the enrollment still
/// landing rather than the signed-out answer it means on a boot.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn resettle() {
    let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector("tonk-account").ok().flatten())
        .and_then(|element| element.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    load_status_with(host, true);
}

/// One account read that rides out the moment it lands in.
///
/// The worker restarts across profile swaps and service-worker
/// adoption — both routine right after a ceremony — and a read caught
/// in that window fails at the transport (or decodes the asset
/// server's fallback) with nothing wrong above it. The panel's whole
/// face hangs on this one answer, so those failures retry, bounded;
/// every other error is a real answer and surfaces unchanged.
async fn account_status_settling(
    ride_out_unregistered: bool,
) -> Result<AccountStatus, crate::error::TonkUiError> {
    let mut last = None;
    for attempt in 0..30 {
        match crate::api::account_status().await {
            Err(error @ crate::error::TonkUiError::ApiError(_)) if attempt < 10 => {
                last = Some(Err(error));
            }
            // A ceremony just said the account exists, and the worker
            // agrees — the enrollment command that mounts the account
            // replica is simply still landing. `Unregistered` is that
            // command's before-state, so after an announcement it reads
            // as "not yet" for a bounded window rather than as the
            // signed-out answer it is on an ordinary boot.
            Ok(status @ AccountStatus::Unregistered { .. }) if ride_out_unregistered => {
                last = Some(Ok(status));
            }
            other => return other,
        }
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 500);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
    last.unwrap_or_else(|| {
        Err(crate::error::TonkUiError::ApiError(
            "account read never settled".to_string(),
        ))
    })
}

fn load_status(host: HtmlElement) {
    load_status_with(host, false);
}

/// `after_ceremony` marks a reload requested by the registration cluster
/// on its way out: a ceremony just finished, so an `Unregistered` answer
/// is the enrollment command still landing, not a signed-out profile.
fn load_status_with(host: HtmlElement, after_ceremony: bool) {
    let handoff_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/settings/link" || path.starts_with("/settings/link/"));
    if handoff_route {
        match callback_request() {
            Some((audience, callback, name)) => {
                // An unlinked browser registers first: the signup or login
                // ceremony is what creates the account and enrolls it with
                // the access service, and only then is there an account to
                // delegate from. The callback request rides the URL, so
                // once the ceremony settles this reloads into the approval
                // panel.
                set_busy(&host, true, "Checking this browser…");
                spawn_local(async move {
                    let (mut attempt, result) = crate::account_observability::observe(
                        AccountAction::LoadAccount,
                        tonk_analytics::account::Surface::Settings,
                        tonk_analytics::account::Trigger::Automatic,
                        tonk_analytics::account::AccountState::Unknown,
                        account_status_settling(after_ceremony),
                    )
                    .await;
                    match result {
                        Ok(AccountStatus::Registered { .. }) => {
                            attempt.finish(
                                tonk_analytics::account::Stage::AccountLoad,
                                tonk_analytics::account::AccountOutcome::success(),
                            );
                            load_callback_request(host, audience, callback, name);
                        }
                        Ok(status) => {
                            attempt.finish(
                                tonk_analytics::account::Stage::AccountLoad,
                                tonk_analytics::account::AccountOutcome::success(),
                            );
                            let root_persisted =
                                matches!(status, AccountStatus::Unregistered { .. });
                            set_busy(&host, false, "");
                            set_mode(&host, "choice");
                            show_error(
                                &host,
                                "Create your account or log in first; approving the \
                                 command-line device comes right after.",
                            );
                            load_choice_profiles(host.clone(), root_persisted);
                        }
                        Err(error) => {
                            let problem =
                                user_error::api_problem(AccountAction::LoadAccount, &error);
                            attempt.finish(
                                tonk_analytics::account::Stage::AccountLoad,
                                problem.outcome,
                            );
                            set_busy(&host, false, "");
                            set_mode(&host, "choice");
                            show_automatic_api_error(&host, AccountAction::LoadAccount, &error);
                        }
                    }
                });
            }
            // Without callback parameters there is nothing to approve:
            // `tonk account login` always carries them.
            None => {
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
                show_error(
                    &host,
                    "This approval link is incomplete. Start again from the terminal.",
                );
            }
        }
        return;
    }
    // A `?link=` query is the CLI callback sending the tab back with the
    // authorization outcome, reported here in the page's own styling.
    let link_outcome = query_value("link").map(|status| (status, query_value("message")));
    set_busy(&host, true, "Checking this browser…");
    let mut settle_attempt = crate::account_observability::take_settle_pending().then(|| {
        crate::account_observability::WebAccountAttempt::start(
            AccountAction::SettleAccount,
            tonk_analytics::account::Surface::Settings,
            tonk_analytics::account::Trigger::Recovery,
            tonk_analytics::account::AccountState::PendingActivation,
        )
    });
    let mut load_attempt = crate::account_observability::WebAccountAttempt::start(
        AccountAction::LoadAccount,
        tonk_analytics::account::Surface::Settings,
        tonk_analytics::account::Trigger::Automatic,
        tonk_analytics::account::AccountState::Unknown,
    );
    spawn_local(async move {
        if let Err(error) = service(&host).await {
            let problem = user_error::problem_from_diagnostic(AccountAction::LoadAccount, &error);
            load_attempt.finish(
                tonk_analytics::account::Stage::AccessService,
                problem.outcome,
            );
            if let Some(attempt) = settle_attempt.as_mut() {
                let problem =
                    user_error::problem_from_diagnostic(AccountAction::SettleAccount, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::AccessService,
                    problem.outcome,
                );
            }
            set_busy(&host, false, "");
            set_mode(&host, NO_PANEL_MODE);
            show_action_error(&host, AccountAction::LoadAccount, &error);
            return;
        }
        match account_status_settling(after_ceremony).await {
            Ok(status) => {
                tonk_common::log!("account: load_status read {status:?}");
                load_attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                // A persisted root with no provider is a signed-out
                // profile: logging in here with a DIFFERENT passkey is
                // refused, so the Choice panel offers a fresh profile.
                let root_persisted = matches!(status, AccountStatus::Unregistered { .. });
                let account_state = match status {
                    AccountStatus::Registered { account_state, .. } => Some(account_state),
                    AccountStatus::RootMissing { .. } | AccountStatus::Unregistered { .. } => None,
                };
                if account_state == Some(AccountStateStatus::Unhydrated) {
                    let _ = host.set_attribute(ACCOUNT_NOT_READY, "true");
                } else {
                    let _ = host.remove_attribute(ACCOUNT_NOT_READY);
                }
                let landing = landing(
                    account_state,
                    revoke_target_from_url().is_some(),
                    adding_account(),
                );
                if let Some(attempt) = settle_attempt.as_mut() {
                    let outcome = if matches!(landing, Landing::Success | Landing::Devices) {
                        tonk_analytics::account::AccountOutcome::success()
                    } else {
                        tonk_analytics::account::AccountOutcome::terminal_failure(
                            tonk_analytics::account::FailureKind::LocalState,
                        )
                    };
                    attempt.finish(tonk_analytics::account::Stage::Complete, outcome);
                }
                match landing {
                    Landing::Devices => show_success(&host),
                    Landing::Success => {
                        // Marked BEFORE settling, because settling starts
                        // the customer probe and that probe's answer is
                        // what decides which message this state deserves.
                        // Asking a second time from here instead would
                        // put two probes in flight at once — and the
                        // probe is not a read: it replays the work
                        // deferred while the account was unserved, the
                        // account backup among it.
                        settle_on_load(&host);
                        apply_link_outcome(&host, link_outcome.as_ref());
                    }
                    Landing::Choice { revoke_hint } => {
                        set_busy(&host, false, "");
                        set_mode(&host, "choice");
                        load_choice_profiles(host.clone(), root_persisted);
                        apply_link_outcome(&host, link_outcome.as_ref());
                        if revoke_hint {
                            show_error(
                                &host,
                                "This browser is not linked to an account. Link it \
                                 first, then reopen the revoke link from the terminal.",
                            );
                        }
                    }
                }
            }
            Err(error) => {
                let load_problem = user_error::api_problem(AccountAction::LoadAccount, &error);
                load_attempt.finish(
                    tonk_analytics::account::Stage::AccountLoad,
                    load_problem.outcome,
                );
                if let Some(attempt) = settle_attempt.as_mut() {
                    let problem = user_error::api_problem(AccountAction::SettleAccount, &error);
                    attempt.finish(tonk_analytics::account::Stage::AccountSync, problem.outcome);
                }
                set_busy(&host, false, "");
                set_mode(&host, "choice");
                show_automatic_api_error(&host, AccountAction::LoadAccount, &error);
            }
        }
    });
}

/// Report a `?link=` outcome the CLI callback sent this tab back with.
fn apply_link_outcome(host: &HtmlElement, outcome: Option<&(String, Option<String>)>) {
    let Some((status, message)) = outcome else {
        return;
    };
    if status == "ok" {
        if let Ok(Some(status)) = host.query_selector("#account-success-message") {
            status.set_text_content(Some("Command-line device linked."));
            let _ = status.remove_attribute("hidden");
        }
    } else {
        let message = message
            .as_deref()
            .unwrap_or("the command-line link did not complete");
        show_action_error(host, AccountAction::LinkCli, message);
    }
}

/// The loopback URL a `tonk account login` run is waiting on, if any.
///
/// The waiting process's audience and callback ride the query, so the
/// approval never touches the account service.
fn pending_callback_request() -> Option<(String, String, String)> {
    let on_link_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/settings/link" || path.starts_with("/settings/link/"));
    if on_link_route {
        callback_request()
    } else {
        None
    }
}

fn callback_request() -> Option<(String, String, String)> {
    Some((
        query_value("audience")?,
        query_value("callback")?,
        query_value("name")
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Command-line profile".to_string()),
    ))
}

/// Approve a waiting command-line profile and return the grant through loopback.
///
/// The page runs the passkey ceremony, mints the `account → profile`
/// powerline, and delivers it to the loopback listener the CLI is holding open.
/// The HTTPS page navigates there with a bodyless GET carrying the grant in the
/// URL fragment; the loopback page then submits it by same-origin POST. This
/// keeps the grant out of the cross-scheme request Safari warns about.
fn load_callback_request(host: HtmlElement, audience: String, callback: String, name: String) {
    if let Ok(Some(label)) = host.query_selector("#account-handoff-name") {
        label.set_text_content(Some(&name));
    }
    if let Ok(Some(did)) = host.query_selector("#account-handoff-did") {
        did.set_text_content(Some(&audience));
    }
    // Park the request where the approve handler can find it.
    if let Ok(value) = serde_wasm_bindgen::to_value(&(audience, callback, name)) {
        let _ = Reflect::set(host.as_ref(), &CALLBACK.into(), &value);
    }
    let label_host = host.clone();
    spawn_local(async move {
        if let Ok(profiles) = crate::api::list_profiles().await
            && let Some(active) = profiles
                .profiles
                .iter()
                .find(|entry| entry.active || entry.profile_name == profiles.active)
        {
            set_text(
                &label_host,
                "#account-handoff-account",
                &profile_row_label(active),
            );
        }
    });
    set_busy(&host, false, "");
    set_mode(&host, "handoff");
}

/// Where the CLI's callback should send this tab once the terminal has
/// its answer: the account page, which renders the `?link=` outcome in
/// its own styling.
fn link_outcome_redirect() -> String {
    window()
        .and_then(|window| window.location().origin().ok())
        .map(|origin| format!("{origin}/settings"))
        .unwrap_or_else(|| "/settings".to_string())
}

/// Base64-encode an authorization payload for form delivery.
///
/// The callback decodes base64 before parsing, so the payload survives form
/// encoding without the caller having to reason about escaping.
pub(crate) fn encode_authorization(payload: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(payload)
}

/// Deliver an authorization to the waiting process through a loopback bridge.
fn deliver_to_callback(callback: &str, fields: &[(&str, &str)]) -> Result<(), String> {
    let target = crate::callback_url::delivery_url(callback, fields)?;
    window()
        .ok_or("window is unavailable")?
        .location()
        .set_href(&target)
        .map_err(|_| "could not deliver the authorization".to_owned())
}

/// Sign in with an existing passkey, with no panel to report into.
///
/// The counterpart to [`run_account_ceremony`], and the reason the
/// address is looked up before either runs: sending someone who already
/// has an account through creation leaves an orphan passkey in their
/// authenticator and fails at the end — which it did, with
/// `409 a different account is already signed in on this profile`,
/// because saving a new root over an existing one is what creation does.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn run_login_ceremony(
    narrate: impl Fn(&str),
) -> Result<(), crate::custody_relay::CeremonyError> {
    use crate::custody_relay::CeremonyError;

    narrate("Waiting for your passkey…");
    // One assertion, and the worker does the rest: it opens the account
    // from its custody cell, mints this browser's delegation, records
    // the root and submits the link. The page holds no key material.
    let provider = proposed_remote().map_err(CeremonyError::said)?;
    narrate("Linking this browser…");
    crate::custody_relay::mediate_now(
        "usePasskey",
        tonk_worker_api::CustodyIntent::Login(tonk_worker_api::DeviceLink {
            device_name: crate::device_name::current(),
            endpoint: proposed_remote().map_err(CeremonyError::said)?,
            provider,
        }),
    )
    .await?;
    Ok(())
}

/// Run the account-creation ceremony, with no panel to report into.
///
/// The same work `/account`'s create button does, lifted out of its
/// click handler so the registration cluster can run it too. Progress
/// goes to `narrate` rather than to `set_busy`, and the service is
/// resolved from the deployment rather than from a host element's
/// `service` attribute — the two callers differ in nothing else.
///
/// Extracted rather than duplicated: two copies of a passkey ceremony
/// would drift, and the half that drifted would leave an orphan
/// credential in someone's authenticator.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn run_account_ceremony(
    email: &str,
    narrate: impl Fn(&str),
) -> Result<(), crate::custody_relay::CeremonyError> {
    prepare_added_profile().await?;
    narrate("Waiting for your passkey…");

    // The page's whole part: one passkey ceremony. The worker
    // generates the account secret, seals it under the new passkey's
    // KEK, records the root, signs the creation request and enrolls, so
    // no key material exists in this document at any point.
    let provider = proposed_remote()?;
    narrate("Creating your account…");
    crate::custody_relay::mediate_now(
        "createPasskey",
        tonk_worker_api::CustodyIntent::CreateAccount(tonk_worker_api::AccountCreation {
            email: email.to_owned(),
            device_name: crate::device_name::current(),
            remote: proposed_remote()?,
            provider,
            created_on: Some(crate::device_name::current()),
        }),
    )
    .await
    .map_err(|error| error.message)?;
    Ok(())
}

/// Whether this deployment registers accounts with an access service at
/// all: deployments that publish no service identity have nothing to
/// enroll with.
async fn wants_enrollment() -> bool {
    deployment_service_did().await.is_some()
}

/// The access-service DID this deployment publishes. Absent config or
/// identity is ordinary: the deployment simply serves no enrollment.
async fn deployment_service_did() -> Option<String> {
    crate::deployment::get()
        .await
        .ok()
        .and_then(|config| config.service_did)
}

/// The account repository remote this browser proposes: its own origin's
/// `/ucan/` endpoint. Only a ceremony ever signs one; the stored descriptor is
/// always the service-selected winner.
pub(crate) fn proposed_remote() -> Result<String, String> {
    window()
        .and_then(|window| window.location().origin().ok())
        .map(|origin| format!("{}/ucan/", origin.trim_end_matches('/')))
        .ok_or_else(|| "window origin is unavailable".to_string())
}

/// Marks an account repository that is not ready for authoritative edits.
const ACCOUNT_NOT_READY: &str = "data-account-not-ready";
/// Marks a banner owned by account-readiness guidance, so the background
/// readiness probe never clears a later unrelated action error.
const ACCOUNT_GUIDANCE_SHOWN: &str = "data-account-guidance-shown";

/// The same state, when the emailed link is what it is waiting on.
///
/// Before that link is opened the access service refuses the pull, so a
/// freshly enrolled account is ALWAYS unsynchronized — expected, and not
/// something a reload can change. Naming the mechanism there and asking
/// for a retry that cannot succeed buries the one step that does.
const VERIFY_EMAIL: &str =
    "Check your email and open the verification link to verify your email address.";
const ACCOUNT_STATUS_UNKNOWN: &str =
    "We couldn't check whether your email is verified. Check your connection and reload settings.";
const ACCOUNT_SETUP_FINISHING: &str = "Your email is verified. Tonk is finishing account setup; reload settings if the display name stays unavailable.";
const ACCOUNT_SYNC_PAUSED: &str = "Online sync for this account is paused. Your local work is still available; try again later or contact Tonk support.";
const ACCOUNT_ENROLLMENT_NOT_STARTED: &str = "Your account is saved on this browser, but Tonk couldn't start email verification. Check your connection and reload settings to try again.";

fn on_click(host: &HtmlElement, selector: &str, callback: impl Fn(HtmlElement) + 'static) {
    let Ok(Some(element)) = host.query_selector(selector) else {
        return;
    };
    let host = host.clone();
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        if host.get_attribute("data-busy").as_deref() == Some("true") {
            return;
        }
        callback(host.clone());
    });
    let _ = element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn confirmation_focusables(host: &HtmlElement) -> Vec<HtmlElement> {
    let Ok(elements) = host.query_selector_all(
        "#account-confirmation button:not([disabled]), #account-confirmation input:not([disabled]), #account-confirmation summary",
    ) else {
        return Vec::new();
    };
    (0..elements.length())
        .filter_map(|index| elements.item(index))
        .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
        .filter(|element| {
            !element.has_attribute("hidden")
                && element.offset_parent().is_some()
                && element.tab_index() >= 0
        })
        .collect()
}

fn trap_confirmation_key(host: &HtmlElement, event: &KeyboardEvent) {
    if host
        .query_selector("#account-confirmation")
        .ok()
        .flatten()
        .is_none_or(|surface| surface.has_attribute("hidden"))
    {
        return;
    }
    if event.key() == "Escape" {
        event.prevent_default();
        close_confirmation(host);
        return;
    }
    if event.key() != "Tab" {
        return;
    }
    let focusable = confirmation_focusables(host);
    let (Some(first), Some(last)) = (focusable.first(), focusable.last()) else {
        return;
    };
    let active = window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element());
    if event.shift_key() && active.as_ref() == Some(first.as_ref()) {
        event.prevent_default();
        let _ = last.focus();
    } else if !event.shift_key() && active.as_ref() == Some(last.as_ref()) {
        event.prevent_default();
        let _ = first.focus();
    }
}

/// Stop every account form from ever navigating.
///
/// These panels are forms for the semantics — labels, `required`, a password
/// manager that recognises an email field — not to submit anywhere. None
/// carries an `action`, so a submission that got through would GET the current
/// URL: the panel reloads, whatever was typed is gone, and the handler that
/// should have run never does. The per-button `prevent_default` in [`on_click`]
/// covers the click path; this covers the form itself, including the implicit
/// submission Enter triggers.
fn prevent_form_navigation(host: &HtmlElement) {
    let Ok(forms) = host.query_selector_all(".account__form") else {
        return;
    };
    for index in 0..forms.length() {
        let Some(form) = forms.item(index) else {
            continue;
        };
        let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            event.prevent_default();
        });
        let _ = form.add_event_listener_with_callback("submit", closure.as_ref().unchecked_ref());
        closure.forget();
    }
}

/// Point the "back" links at wherever the user came from.
///
/// They read `/` in the markup because that is where someone who opened
/// `/settings` themselves belongs. Arriving through the gate, `/` is the one
/// place the user was NOT — leaving means abandoning the space they were
/// looking at — so the `next` the gate carried wins.
fn bind_return_links(host: &HtmlElement) {
    let Some(next) = crate::account_gate::requested_next() else {
        return;
    };
    let Ok(links) = host.query_selector_all("[data-return]") else {
        return;
    };
    for index in 0..links.length() {
        let Some(link) = links.item(index) else {
            continue;
        };
        let Ok(link) = link.dyn_into::<web_sys::Element>() else {
            continue;
        };
        let _ = link.set_attribute("href", &next);
        // "Back to Tonk" is the truth for `/`, and a lie for a space. The
        // destination changed, so the label has to.
        link.set_text_content(Some("back"));
    }
}

fn bind(host: &HtmlElement) {
    prevent_form_navigation(host);
    bind_return_links(host);
    configure_deletion_entry(host);

    if let Ok(Some(rail)) = host.query_selector(".account__rail") {
        let tab_host = host.clone();
        let click = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(name) = target.get_attribute("data-account-tab") else {
                return;
            };
            select_account_tab(&tab_host, &name, true);
        });
        let _ = rail.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        click.forget();

        let tab_host = host.clone();
        let keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            let target_name = match event.key().as_str() {
                "ArrowLeft" | "ArrowRight" => event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                    .and_then(|target| target.get_attribute("data-account-tab"))
                    .map(|current| {
                        if current == "account" {
                            "devices"
                        } else {
                            "account"
                        }
                    }),
                "Home" => Some("account"),
                "End" => Some("devices"),
                _ => None,
            };
            if let Some(name) = target_name {
                event.prevent_default();
                select_account_tab(&tab_host, name, true);
            }
        });
        let _ = rail.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        keydown.forget();
    }

    if let Ok(Some(name)) = host.query_selector("#account-display-name") {
        let name_host = host.clone();
        let keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            if event.key() == "Enter" {
                event.prevent_default();
                commit_display_name(name_host.clone());
            }
        });
        let _ = name.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        keydown.forget();

        let name_host = host.clone();
        let blur = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            commit_display_name(name_host.clone());
        });
        let _ = name.add_event_listener_with_callback("blur", blur.as_ref().unchecked_ref());
        blur.forget();
    }

    let key_host = host.clone();
    let keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        trap_confirmation_key(&key_host, &event);
    });
    let _ = host.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
    keydown.forget();

    for selector in [
        "#account-confirm-close",
        "#account-confirm-cancel",
        "[data-confirmation-scrim]",
    ] {
        on_click(host, selector, |host| cancel_confirmation(&host));
    }

    for event_name in ["input", "change"] {
        if let Ok(Some(arming)) = host.query_selector("#account-delete-arming") {
            let arming_host = host.clone();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event| {
                update_confirmation_arming(&arming_host);
            });
            let _ = arming
                .add_event_listener_with_callback(event_name, closure.as_ref().unchecked_ref());
            closure.forget();
        }
    }
    // One entry, raising the same cluster the share flow raises. It
    // starts from the address and routes on the answer — create a
    // passkey for an address nobody holds, sign in for one that exists
    // — so the old up-front "create account / log in" fork asked the
    // user a question the lookup answers on its own.
    //
    // Opened here there is no interrupted share to finish, so the
    // cluster closes on "your account is ready" instead of handing over
    // a link.
    on_click(host, "#account-choose-link", |host| {
        clear_error(&host);
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        crate::register_dialog::open();
    });
    for selector in ["#account-create-back", "#account-link-back"] {
        on_click(host, selector, |host| {
            clear_error(&host);
            set_mode(&host, "choice");
        });
    }
    on_click(host, "#account-create-submit", |host| {
        clear_error(&host);
        let email = match input(&host, "#account-email") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        let device_name = crate::device_name::current();
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let mut attempt = crate::account_observability::WebAccountAttempt::start(
                AccountAction::CreateAccount,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::None,
            );
            let result = async {
                prepare_added_profile().await?;
                // The page's whole part: one passkey ceremony. The
                // worker generates the account secret, seals it under
                // the new passkey's KEK, records the root, signs the
                // creation request and enrolls — so no key material
                // exists in this document at any point.
                let provider = service(&host).await?;
                crate::custody_relay::mediate_now(
                    "createPasskey",
                    tonk_worker_api::CustodyIntent::CreateAccount(
                        tonk_worker_api::AccountCreation {
                            email: email.clone(),
                            device_name,
                            remote: proposed_remote()?,
                            provider,
                            created_on: Some(crate::device_name::current()),
                        },
                    ),
                )
                .await?;
                set_busy(&host, true, "Creating your account…");
                Ok::<(), crate::custody_relay::CeremonyError>(())
            }
            .await;
            match result {
                Ok(()) => attempt.finish(
                    tonk_analytics::account::Stage::ActivationWait,
                    tonk_analytics::account::AccountOutcome::blocked(
                        tonk_analytics::account::FailureKind::AwaitingActivation,
                    ),
                ),
                Err(error) => {
                    let problem =
                        user_error::ceremony_problem(AccountAction::CreateAccount, &error);
                    attempt.finish(
                        tonk_analytics::account::Stage::WorkerHandoff,
                        problem.outcome,
                    );
                    set_busy(&host, false, "");
                    show_ceremony_error(&host, AccountAction::CreateAccount, &error);
                }
            }
        });
    });

    on_click(host, "#account-resend-activation", |host| {
        clear_error(&host);
        set_busy(&host, true, "Sending another activation email…");
        spawn_local(async move {
            // A resend, not a re-enrollment: the rows stand at the
            // service, so the worker only signs the resend invocation —
            // no passkey prompt for someone who is waiting on an inbox.
            let (mut attempt, result) = crate::account_observability::observe(
                AccountAction::ResendActivation,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::PendingActivation,
                crate::api::resend_activation(),
            )
            .await;
            set_busy(&host, false, "");
            match result {
                Ok(_) => {
                    attempt.finish(
                        tonk_analytics::account::Stage::RemoteCommit,
                        tonk_analytics::account::AccountOutcome::success(),
                    );
                    set_text(
                        &host,
                        "#account-activation-notice",
                        "Sent. Open the link in your activation email.",
                    );
                    // Counted here, not answered by the service: a
                    // refusal that said "wait 40s" would confirm the
                    // address is registered to anyone who asked, so the
                    // service answers a resend the same way whether the
                    // account exists or not. The page knows it just
                    // pressed the button, which is enough to say so.
                    count_down_resend(host.clone());
                }
                Err(error) => {
                    let problem = user_error::api_problem(AccountAction::ResendActivation, &error);
                    attempt.finish(
                        tonk_analytics::account::Stage::RemoteCommit,
                        problem.outcome,
                    );
                    show_api_error(&host, AccountAction::ResendActivation, &error);
                }
            }
        });
    });

    on_click(host, "#account-add-passkey", |host| {
        clear_error(&host);
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let mut attempt = crate::account_observability::WebAccountAttempt::start(
                AccountAction::AddPasskey,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::Ready,
            );
            let result = async {
                let (root_did, _delegation_hex) = match crate::api::root_status()
                    .await
                    .map_err(|error| error.to_string())?
                {
                    tonk_worker_api::RootStatus::Ready {
                        root_did,
                        delegation_hex,
                        ..
                    } => (root_did, delegation_hex),
                    tonk_worker_api::RootStatus::Missing { .. } => {
                        return Err("no account on this browser to add a passkey for".into());
                    }
                };
                let label = crate::api::account_summary()
                    .await
                    .ok()
                    .and_then(|summary| summary.email);
                let _ = &label;
                // Two ceremonies, one handoff: assert the passkey that
                // holds the account, create the one being added, and
                // let the worker open and re-seal. The account secret
                // never reaches this document.
                crate::custody_relay::mediate_now(
                    "addPasskey",
                    tonk_worker_api::CustodyIntent::AddPasskey(tonk_worker_api::PasskeyAddition {
                        account_did: root_did,
                        endpoint: proposed_remote()?,
                    }),
                )
                .await?;
                Ok::<(), crate::custody_relay::CeremonyError>(())
            }
            .await;
            set_busy(&host, false, "");
            match result {
                Ok(()) => {
                    attempt.finish(
                        tonk_analytics::account::Stage::RemoteCommit,
                        tonk_analytics::account::AccountOutcome::success(),
                    );
                    if let Ok(Some(button)) = host.query_selector("#account-add-passkey") {
                        let _ = button.set_attribute("hidden", "");
                    }
                    load_summary(host.clone());
                }
                Err(error) => {
                    let problem = user_error::ceremony_problem(AccountAction::AddPasskey, &error);
                    attempt.finish(
                        tonk_analytics::account::Stage::WorkerHandoff,
                        problem.outcome,
                    );
                    show_ceremony_error(&host, AccountAction::AddPasskey, &error);
                }
            }
        });
    });

    on_click(host, "#account-link-submit", |host| {
        clear_error(&host);
        let device_name = crate::device_name::current();
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let mut attempt = crate::account_observability::WebAccountAttempt::start(
                AccountAction::LogIn,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::None,
            );
            let result = async {
                prepare_added_profile().await?;
                // One assertion, and the worker does the rest: it
                // opens the account from its custody cell, mints this
                // browser's delegation, records the root and submits
                // the link.
                let provider = service(&host).await?;
                set_busy(&host, true, "Linking this browser…");
                crate::custody_relay::mediate_now(
                    "usePasskey",
                    tonk_worker_api::CustodyIntent::Login(tonk_worker_api::DeviceLink {
                        device_name,
                        endpoint: proposed_remote()?,
                        provider,
                    }),
                )
                .await
            }
            .await;
            match result {
                Ok(_) => attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    tonk_analytics::account::AccountOutcome::success(),
                ),
                Err(error) => {
                    let problem = user_error::ceremony_problem(AccountAction::LogIn, &error);
                    attempt.finish(
                        tonk_analytics::account::Stage::WorkerHandoff,
                        problem.outcome,
                    );
                    set_busy(&host, false, "");
                    show_ceremony_error(&host, AccountAction::LogIn, &error);
                }
            }
        });
    });

    on_click(host, "#account-handoff-submit", |host| {
        clear_error(&host);
        // A callback authorization takes this button first: the panel asks
        // the same question, but the answer goes back to a waiting process
        // rather than to the account service.
        if let Some((audience, callback, name)) = Reflect::get(host.as_ref(), &CALLBACK.into())
            .ok()
            .and_then(|value| {
                serde_wasm_bindgen::from_value::<(String, String, String)>(value).ok()
            })
        {
            set_busy(&host, true, "Waiting for your passkey…");
            spawn_local(async move {
                let mut attempt = crate::account_observability::WebAccountAttempt::start(
                    AccountAction::LinkCli,
                    tonk_analytics::account::Surface::Settings,
                    tonk_analytics::account::Trigger::User,
                    tonk_analytics::account::AccountState::Ready,
                );
                let result = async {
                    // Registration precedes linking: a device linked to an
                    // unregistered account inherits a dead sync path, so a
                    // signed-in browser the access service does not know
                    // enrolls before it delegates. The fresh-browser path
                    // covers this inside its signup ceremony.
                    if wants_enrollment().await {
                        let known = crate::api::customer_state()
                            .await
                            .map(|state| !state["status"].is_null())
                            .unwrap_or(false);
                        if !known {
                            set_busy(&host, true, "Registering with the sync service…");
                            crate::api::enroll_customer(None).await.map_err(|error| {
                                format!("register with the sync service before linking: {error}")
                            })?;
                        }
                    }
                    set_busy(&host, true, "Waiting for your passkey…");
                    // The descriptor must name the same sync remote signup
                    // established — the page's own `/ucan/` endpoint — or the
                    // linked device mounts an account it can never reach.
                    let authorized = crate::identity_bridge::authorize_device(
                        crate::identity_bridge::AuthorizeDeviceInput {
                            device_did: audience.clone(),
                            remote: proposed_remote()?,
                            endpoint: proposed_remote()?,
                        },
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    // The service only accepts device registration from an
                    // active member, which this browser is and the waiting
                    // device is not: register it here, before the grant is
                    // delivered, so a device that installs the grant is
                    // already listed and able to reach the service.
                    set_busy(&host, true, "Registering the device…");
                    let registered = crate::api::register_account_device(
                        &audience,
                        &name,
                        &authorized.delegation_hex,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    let attachment_id = registered
                        .get("attachmentId")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    // The delegation alone would leave the device authorized
                    // but unable to find the account repository, so the
                    // descriptor rides along.
                    // The page knows which account service this deployment
                    // uses; the CLI records it rather than guessing from a
                    // flag default.
                    let payload = serde_json::json!({
                        "delegationHex": authorized.delegation_hex,
                        // The CLI records the account repository under this
                        // remote — the same one the grant above was minted
                        // for. Its schema requires the field, so omitting it
                        // fails the whole handoff as "payload is not
                        // readable"; the descriptor stays alongside for CLIs
                        // from before the remote rode the callback.
                        "remote": proposed_remote()?,
                        "descriptorHex": authorized.descriptor_hex,
                        "credentialId": authorized.root_did,
                        "attachmentId": attachment_id,
                        "serviceUrl": service(&host).await.unwrap_or_default(),
                    })
                    .to_string();
                    let encoded = crate::account::encode_authorization(&payload);
                    let redirect = link_outcome_redirect();
                    deliver_to_callback(
                        &callback,
                        &[("authorize", &encoded), ("redirect", &redirect)],
                    )?;
                    Ok::<(), String>(())
                }
                .await;
                match result {
                    Ok(()) => attempt.finish(
                        tonk_analytics::account::Stage::CallbackDelivery,
                        tonk_analytics::account::AccountOutcome::success(),
                    ),
                    Err(error) => {
                        let problem =
                            user_error::problem_from_diagnostic(AccountAction::LinkCli, &error);
                        attempt.finish(
                            tonk_analytics::account::Stage::CallbackDelivery,
                            problem.outcome,
                        );
                        set_busy(&host, false, "");
                        log_action_error(AccountAction::LinkCli, &error);
                        show_error(&host, problem.message);
                    }
                }
            });
            return;
        }
        show_error(
            &host,
            "This approval link is incomplete. Start again from the terminal.",
        );
    });

    // Cancelling a callback authorization tells the waiting process, rather
    // than only navigating away. Without this the CLI sits until its
    // five-minute deadline for a decision the user already made.
    on_click(host, "#account-handoff-cancel", |host| {
        let Some((_, callback, _)) =
            Reflect::get(host.as_ref(), &CALLBACK.into())
                .ok()
                .and_then(|value| {
                    serde_wasm_bindgen::from_value::<(String, String, String)>(value).ok()
                })
        else {
            // No callback parked, so Cancel is an ordinary link back to the
            // account page. `on_click` already suppressed the navigation, so
            // do it explicitly.
            if let Some(window) = window() {
                let _ = window.location().set_href("/settings");
            }
            return;
        };
        let redirect = link_outcome_redirect();
        if let Err(error) = deliver_to_callback(
            &callback,
            &[("deny", "declined in the browser"), ("redirect", &redirect)],
        ) {
            show_action_error(&host, AccountAction::LinkCli, &error);
        } else {
            let mut attempt = crate::account_observability::WebAccountAttempt::start(
                AccountAction::LinkCli,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::Ready,
            );
            attempt.finish(
                tonk_analytics::account::Stage::CallbackDelivery,
                tonk_analytics::account::AccountOutcome::cancelled(),
            );
        }
    });

    on_click(host, "#account-unlink", |host| {
        clear_error(&host);
        if let Err(error) = open_confirmation(&host, Confirmation::SignOut) {
            show_action_error(&host, AccountAction::SignOut, &error);
        }
    });

    on_click(host, "#account-delete-review", |host| {
        clear_error(&host);
        set_busy(&host, true, "Loading the permanent deletion scope…");
        spawn_local(async move {
            let (mut attempt, result) = crate::account_observability::observe(
                AccountAction::LoadDeletionPlan,
                tonk_analytics::account::Surface::Settings,
                tonk_analytics::account::Trigger::User,
                tonk_analytics::account::AccountState::Ready,
                crate::api::account_deletion_plan(),
            )
            .await;
            match result {
                Ok(plan) => {
                    attempt.finish(
                        tonk_analytics::account::Stage::AccountLoad,
                        tonk_analytics::account::AccountOutcome::success(),
                    );
                    set_busy(&host, false, "");
                    let pending = Confirmation::Delete {
                        plan,
                        requested_space: requested_space_deletion(),
                    };
                    if let Err(error) = open_confirmation(&host, pending) {
                        show_action_error(&host, AccountAction::LoadDeletionPlan, &error);
                    }
                }
                Err(error) => {
                    let problem = user_error::api_problem(AccountAction::LoadDeletionPlan, &error);
                    attempt.finish(tonk_analytics::account::Stage::AccountLoad, problem.outcome);
                    set_busy(&host, false, "");
                    show_api_error(&host, AccountAction::LoadDeletionPlan, &error);
                }
            }
        });
    });

    on_click(host, "#account-delete-submit", |host| {
        clear_error(&host);
        let Some(pending) = confirmation(&host) else {
            return show_confirmation_error(&host, "No confirmation is pending.");
        };
        match pending {
            Confirmation::SignOut => {
                set_busy(&host, true, "Signing out…");
                spawn_local(async move {
                    let (mut attempt, result) = crate::account_observability::observe(
                        AccountAction::SignOut,
                        tonk_analytics::account::Surface::Settings,
                        tonk_analytics::account::Trigger::User,
                        tonk_analytics::account::AccountState::Ready,
                        crate::api::unlink_account(),
                    )
                    .await;
                    match result {
                        Ok(_) => {
                            attempt.finish(
                                tonk_analytics::account::Stage::LocalCommit,
                                tonk_analytics::account::AccountOutcome::success(),
                            );
                            match window().map(|window| window.location().reload()) {
                                Some(Ok(())) => {}
                                _ => {
                                    set_busy(&host, false, "");
                                    close_confirmation(&host);
                                    set_mode(&host, "choice");
                                }
                            }
                        }
                        Err(error) => {
                            let problem = user_error::api_problem(AccountAction::SignOut, &error);
                            attempt.finish(
                                tonk_analytics::account::Stage::LocalCommit,
                                problem.outcome,
                            );
                            set_busy(&host, false, "");
                            show_confirmation_api_error(&host, AccountAction::SignOut, &error);
                        }
                    }
                });
            }
            Confirmation::Revoke { did, self_revoke } => {
                execute_revoke(host, did, self_revoke);
            }
            Confirmation::Delete {
                plan,
                requested_space,
            } => begin_delete(host, plan, requested_space),
        }
    });

    on_click(host, "#account-confirm-result-back", |_| {
        tonk_host::navigate_to("/settings");
    });

    // Opening Add account is reversible navigation. The final Create or Log
    // in submit prepares the fresh profile immediately before its ceremony.
    for selector in ["#account-add-profile", "#account-use-different-account"] {
        on_click(host, selector, |_| {
            tonk_host::navigate_to("/settings?add=1");
        });
    }

    // Switch rows are re-rendered wholesale, so both lists get one
    // delegated listener keyed off `data-activate` — the same pattern
    // the device list uses for `data-revoke`.
    for selector in ["#account-profile-list", "#account-choice-profile-list"] {
        let Ok(Some(list)) = host.query_selector(selector) else {
            continue;
        };
        let host_for_switch = host.clone();
        let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(profile) = target.get_attribute("data-activate") else {
                return;
            };
            let host = host_for_switch.clone();
            clear_error(&host);
            set_busy(&host, true, "Switching account…");
            spawn_local(async move {
                let (mut attempt, result) = crate::account_observability::observe(
                    AccountAction::SwitchProfile,
                    tonk_analytics::account::Surface::Settings,
                    tonk_analytics::account::Trigger::User,
                    tonk_analytics::account::AccountState::Ready,
                    crate::api::activate_profile(profile),
                )
                .await;
                match result {
                    Ok(_) => {
                        attempt.finish(
                            tonk_analytics::account::Stage::LocalCommit,
                            tonk_analytics::account::AccountOutcome::success(),
                        );
                        reload_into_switched_profile(&host)
                    }
                    Err(error) => {
                        let problem = user_error::api_problem(AccountAction::SwitchProfile, &error);
                        attempt
                            .finish(tonk_analytics::account::Stage::LocalCommit, problem.outcome);
                        set_busy(&host, false, "");
                        show_api_error(&host, AccountAction::SwitchProfile, &error);
                    }
                }
            });
        });
        let _ = list.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    if let Ok(Some(list)) = host.query_selector("#account-device-list") {
        let host_for_revoke = host.clone();
        let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(did) = target.get_attribute("data-revoke") else {
                return;
            };
            let self_revoke = target.get_attribute("data-self-revoke").is_some();
            begin_revoke(host_for_revoke.clone(), did, self_revoke);
        });
        let _ = list.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }
}

fn begin_delete(host: HtmlElement, plan: AccountDeletionPlan, requested: Option<String>) {
    let action = if requested.is_some() {
        AccountAction::DeleteSpace
    } else {
        AccountAction::DeleteAccount
    };
    let confirmed_email = match input(&host, "#account-delete-email") {
        Ok(email) if email == plan.email => email,
        Ok(_) => {
            return show_confirmation_validation_error(
                &host,
                action,
                "The confirmation email does not match this account.",
            );
        }
        Err(error) => return show_confirmation_validation_error(&host, action, error),
    };
    let understood = host
        .query_selector("#account-delete-understood")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
        .is_some_and(|input| input.checked());
    if !understood {
        return show_confirmation_validation_error(
            &host,
            action,
            "Confirm that you understand the permanent consequences.",
        );
    }
    let destructive: Vec<_> = plan
        .spaces
        .iter()
        // Every space the plan lists is still there: a finished
        // deletion takes its record with it.
        .filter(|space| {
            requested
                .as_deref()
                .is_none_or(|subject| space.subject == subject)
        })
        .cloned()
        .collect();
    if requested.is_some() && destructive.len() != 1 {
        return show_confirmation_validation_error(
            &host,
            action,
            "The selected owned space is already deleted.",
        );
    }
    set_busy(
        &host,
        true,
        if requested.is_some() {
            "Deleting selected space…"
        } else {
            "Waiting for your passkey…"
        },
    );
    let mut attempt = crate::account_observability::WebAccountAttempt::start(
        action,
        tonk_analytics::account::Surface::Settings,
        tonk_analytics::account::Trigger::User,
        tonk_analytics::account::AccountState::Ready,
    );
    spawn_local(async move {
        let result = async {
            if requested.is_some() {
                // Deleting one hosted space is deprovisioning — the
                // worker signs `/provider/remove` with this device's
                // own authority; no passkey ceremony is involved.
                let space = &destructive[0];
                let deleted = crate::api::delete_owned_space(&AccountSpaceDeletionRequest {
                    subject: space.subject.clone(),
                })
                .await
                .map_err(DeleteFailure::MutationApi)?;
                return Ok::<_, DeleteFailure>((Some(deleted.subject), None));
            }
            // Account deletion asks the human to verify with the
            // account's passkey, then the worker signs every
            // destructive invocation with this device's delegated
            // authority.
            let credential_id = match crate::api::root_status()
                .await
                .map_err(DeleteFailure::PreflightApi)?
            {
                tonk_worker_api::RootStatus::Ready {
                    root_did,
                    credential_id,
                    ..
                } => {
                    if root_did != plan.root_did {
                        return Err(DeleteFailure::Diagnostic(
                            "this device's passkey belongs to a different account".into(),
                        ));
                    }
                    credential_id
                }
                tonk_worker_api::RootStatus::Missing { .. } => {
                    return Err(DeleteFailure::Diagnostic(
                        "no account passkey is registered on this device to verify with".into(),
                    ));
                }
            };
            verify_passkey(VerifyPasskeyInput { credential_id })
                .await
                .map_err(|error| DeleteFailure::Diagnostic(error.to_string()))?;
            let spaces = destructive
                .iter()
                .map(|space| AccountSpaceDeletionRequest {
                    subject: space.subject.clone(),
                })
                .collect();
            let deleted = crate::api::delete_account(&AccountDeletionRequest {
                spaces,
                confirmed_email,
            })
            .await
            .map_err(DeleteFailure::MutationApi)?;
            Ok((None, Some(deleted)))
        }
        .await;
        match result {
            Ok((Some(subject), None)) => {
                attempt.finish(
                    tonk_analytics::account::Stage::Complete,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                render_confirmation_result(
                    &host,
                    &format!(
                        "Owned space {subject} was deleted from Tonk services. Your account and other spaces remain. Tonk cannot erase copies already replicated to other devices."
                    ),
                );
            }
            Ok((None, Some(result))) => {
                attempt.finish(
                    tonk_analytics::account::Stage::Complete,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                render_confirmation_result(
                    &host,
                    &format!(
                        "Account deleted. {} owned space{} removed from Tonk services; {} joined space{} left intact.",
                        result.deleted_spaces,
                        if result.deleted_spaces == 1 { "" } else { "s" },
                        result.retained_joined_spaces,
                        if result.retained_joined_spaces == 1 {
                            ""
                        } else {
                            "s"
                        },
                    ),
                );
            }
            Ok(_) => {
                set_busy(&host, false, "");
                let problem = user_error::problem_from_diagnostic(
                    action,
                    "the deletion result was incomplete",
                );
                attempt.finish(tonk_analytics::account::Stage::Complete, problem.outcome);
                log_action_error(action, "the deletion result was incomplete");
                show_confirmation_error(&host, problem.message);
            }
            Err(DeleteFailure::PreflightApi(error)) => {
                set_busy(&host, false, "");
                let problem = user_error::api_problem(action, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::LocalPreflight,
                    problem.outcome,
                );
                log_action_error(action, &error.to_string());
                show_confirmation_error(&host, problem.message);
            }
            Err(DeleteFailure::MutationApi(error)) => {
                set_busy(&host, false, "");
                log_action_error(action, &error.to_string());
                let problem = user_error::mutation_api_problem(action, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    problem.outcome,
                );
                show_confirmation_error(&host, problem.message);
            }
            Err(DeleteFailure::Diagnostic(error)) => {
                set_busy(&host, false, "");
                let problem = user_error::problem_from_diagnostic(action, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::LocalPreflight,
                    problem.outcome,
                );
                log_action_error(action, &error);
                show_confirmation_error(&host, problem.message);
            }
        }
    });
}

/// Confirm and revoke.
///
/// No passkey ceremony: the worker's own account grant is a powerline,
/// so it mints the revocation itself — for this device from its own
/// link, for another from the target's grant retained in the account
/// space. Shared by the device list's button and the CLI's `?revoke=`
/// handoff.
fn begin_revoke(host: HtmlElement, did: String, self_revoke: bool) {
    clear_error(&host);
    if let Err(error) = open_confirmation(&host, Confirmation::Revoke { did, self_revoke }) {
        show_action_error(&host, AccountAction::RevokeDevice, &error);
    }
}

fn execute_revoke(host: HtmlElement, did: String, self_revoke: bool) {
    set_busy(
        &host,
        true,
        if self_revoke {
            "Revoking this device…"
        } else {
            "Revoking device…"
        },
    );
    let mut attempt = crate::account_observability::WebAccountAttempt::start(
        AccountAction::RevokeDevice,
        tonk_analytics::account::Surface::Settings,
        tonk_analytics::account::Trigger::User,
        tonk_analytics::account::AccountState::Ready,
    );
    spawn_local(async move {
        match crate::api::revoke_account_device(did).await {
            Ok(acknowledgement) => {
                attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    tonk_analytics::account::AccountOutcome::success(),
                );
                clear_error(&host);
                set_busy(
                    &host,
                    false,
                    revocation_status(&acknowledgement, self_revoke),
                );
                close_confirmation(&host);
                if self_revoke {
                    disable_authority_actions(&host);
                    return;
                }

                // Canonical publication is already complete. Refreshing the
                // list is deliberately best-effort and cannot turn that
                // success into a failure — or overwrite its status line.
                let refreshed = async {
                    let own = crate::api::identify().await?.did;
                    let devices = crate::api::account_devices().await?;
                    Ok::<_, crate::error::TonkUiError>((devices, own))
                }
                .await;
                match refreshed {
                    Ok((devices, own)) => render_devices(&host, &devices, &own),
                    Err(error) => web_sys::console::warn_1(
                        &format!(
                            "device revocation published; device-list refresh failed: {error}"
                        )
                        .into(),
                    ),
                }
            }
            Err(error) => {
                set_busy(&host, false, "");
                log_action_error(AccountAction::RevokeDevice, &error.to_string());
                let problem = user_error::mutation_api_problem(AccountAction::RevokeDevice, &error);
                attempt.finish(
                    tonk_analytics::account::Stage::RemoteCommit,
                    problem.outcome,
                );
                show_confirmation_error(&host, problem.message);
            }
        }
    });
}

/// Register `<tonk-account>` with the top document.
pub fn register() {
    if let Some(window) = window()
        && window.custom_elements().get("tonk-account").is_undefined()
    {
        TonkAccount::define("tonk-account");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn host() -> HtmlElement {
        let host: HtmlElement = window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("tonk-account")
            .unwrap()
            .unchecked_into();
        let mut element = TonkAccount;
        element.inject_children(&host);
        host
    }

    /// Yield to the event loop for `ms` milliseconds.
    async fn yield_for(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let win = window().unwrap();
            win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    /// Build a `<tonk-account>` host with its panels injected, attach it to
    /// the document body so it sits in a real DOM tree, and give it a tick
    /// to settle.
    async fn mounted_account_host() -> HtmlElement {
        let host = host();
        window()
            .unwrap()
            .document()
            .unwrap()
            .body()
            .unwrap()
            .append_child(host.as_ref())
            .unwrap();
        yield_for(0).await;
        host
    }

    /// Swap the page query for the duration of a test, then put it back.
    ///
    /// `requested_next` reads `window.location.search`, and the test page has
    /// its own — restoring it keeps these tests from leaking into whatever
    /// runs next in the same document.
    struct Query(String);

    impl Query {
        fn set(search: &str) -> Self {
            let window = window().unwrap();
            let previous = window.location().search().unwrap_or_default();
            let path = window.location().pathname().unwrap();
            window
                .history()
                .unwrap()
                .replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{search}")))
                .unwrap();
            Self(format!("{path}{previous}"))
        }
    }

    impl Drop for Query {
        fn drop(&mut self) {
            if let Some(window) = window()
                && let Ok(history) = window.history()
            {
                let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&self.0));
            }
        }
    }

    /// Arriving through the gate, `/` is the one place the user was not.
    #[dialog_common::test]
    fn it_returns_the_back_links_to_where_the_gate_came_from() {
        let _query = Query::set("?next=%2Fspace%2Fdid%3Akey%3AzBack");
        let host = host();
        bind_return_links(&host);

        let back = host
            .query_selector("#account-success [data-return]")
            .unwrap()
            .expect("the success panel offers a way back");
        assert_eq!(
            back.get_attribute("href").as_deref(),
            Some("/space/did:key:zBack")
        );
        assert_eq!(
            back.text_content().as_deref(),
            Some("back"),
            "the label has to stop claiming it goes to Tonk"
        );
    }

    /// Opened directly, the back links stay pointed at the hub.
    #[dialog_common::test]
    fn it_leaves_the_back_links_alone_without_a_next() {
        let _query = Query::set("");
        let host = host();
        bind_return_links(&host);

        assert_eq!(
            host.query_selector("#account-success [data-return]")
                .unwrap()
                .expect("the success panel offers a way back")
                .get_attribute("href")
                .as_deref(),
            Some("/")
        );
    }

    /// A callback request is recognized by its query parameters.
    #[dialog_common::test]
    fn it_encodes_an_authorization_for_form_delivery() {
        use base64::Engine as _;

        let payload = r#"{"delegationHex":"ab","descriptorHex":"cd"}"#;
        let encoded = encode_authorization(payload);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("the callback decodes base64 before parsing");
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            payload,
            "the payload must survive encoding unchanged"
        );
        assert!(
            !encoded.contains('{') && !encoded.contains('"'),
            "encoding is what keeps the payload safe through form fields, got {encoded}"
        );
    }

    #[dialog_common::test]
    fn it_authors_the_create_and_self_link_controls() {
        let host = host();
        for selector in [
            "#account-create-submit",
            "#account-link-submit",
            "#account-handoff-submit",
        ] {
            assert!(
                host.query_selector(selector).unwrap().is_some(),
                "{selector}"
            );
        }
        assert_eq!(
            host.query_selector("#account-choose-link")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("link an account"),
            "the entry names the same act the share flow names",
        );
        assert!(
            host.query_selector("#account-choose-create")
                .unwrap()
                .is_none(),
            "the create/log-in fork is the lookup's to make, not the user's",
        );
        assert!(
            host.query_selector("#account-retry-local")
                .unwrap()
                .is_none(),
            "local persistence recovery must not be exposed in the account UI"
        );
    }

    #[dialog_common::test]
    fn it_rejects_invalid_authored_fields_before_network_work() {
        let host = host();
        let email: HtmlInputElement = host
            .query_selector("#account-email")
            .unwrap()
            .unwrap()
            .unchecked_into();
        email.set_value("not-an-email");
        assert!(input(&host, "#account-email").is_err());
        email.set_value("person@example.com");
        assert_eq!(
            input(&host, "#account-email").as_deref(),
            Ok("person@example.com")
        );
    }

    #[dialog_common::test]
    fn it_disables_in_panel_navigation_while_account_work_is_in_flight() {
        let host = host();
        set_mode(&host, "create");
        set_busy(&host, true, "Creating your account…");

        for selector in [
            "#account-choose-link",
            "#account-create-back",
            "#account-link-back",
        ] {
            let button: HtmlButtonElement = host
                .query_selector(selector)
                .unwrap()
                .unwrap()
                .unchecked_into();
            assert!(button.disabled(), "{selector} remained interactive");
        }
        let initiating = host
            .query_selector("#account-create-submit")
            .unwrap()
            .unwrap();
        assert!(initiating.has_attribute("data-initiating"));
        assert_eq!(
            initiating.text_content().as_deref(),
            Some("Creating your account…")
        );
        assert_eq!(host.get_attribute("aria-busy").as_deref(), Some("true"));
    }

    #[dialog_common::test]
    fn it_authors_the_attached_account_and_devices_settings_panels() {
        let host = host();
        let dashboard = host
            .query_selector("#account-success")
            .unwrap()
            .expect("signed-in dashboard");
        for selector in [
            "#account-device-list",
            "#account-unlink",
            "#account-email-value",
            "#account-passkey-created-value",
            "#account-passkey-device-value",
            "#account-profile-list",
            "#account-add-profile",
            "#account-display-name",
            "#account-delete-review",
            "[role=tablist]",
            "#account-pane-account",
            "#account-pane-devices",
        ] {
            assert!(
                dashboard.query_selector(selector).unwrap().is_some(),
                "the signed-in dashboard is missing {selector}"
            );
        }
        assert!(
            host.query_selector("#account-success [data-return]")
                .unwrap()
                .is_some(),
            "the account masthead should offer a conventional return link"
        );
        assert!(
            host.query_selector(".account__logo[role=img][aria-label=tonk]")
                .unwrap()
                .is_some(),
            "settings should carry the Tonk wordmark"
        );
        let status = host
            .query_selector("#account-success-message")
            .unwrap()
            .expect("optional settings outcome");
        assert!(
            status.has_attribute("hidden"),
            "the settings page should not restate that this device is signed in"
        );
        assert_eq!(status.text_content().as_deref(), Some(""));

        let outcome = ("ok".to_string(), None);
        apply_link_outcome(&host, Some(&outcome));
        assert!(!status.has_attribute("hidden"));
        assert_eq!(
            status.text_content().as_deref(),
            Some("Command-line device linked.")
        );

        let copy = dashboard.text_content().unwrap();
        assert!(copy.contains("device, browser profile, or password manager"));
        assert!(copy.contains("Spaces created by other people will not be deleted"));
        assert!(copy.contains("cannot erase copies already replicated to other devices"));
        for technical in ["authority", "grant", "relink required"] {
            assert!(
                !copy.contains(technical),
                "signed-in copy exposes technical term {technical}"
            );
        }
        assert!(
            host.query_selector("#account-manage-devices")
                .unwrap()
                .is_none(),
            "device management should not sit behind an interstitial"
        );
        assert!(
            host.query_selector(".account__danger").unwrap().is_none(),
            "destructive actions must not introduce a red danger surface"
        );

        select_account_tab(&host, "devices", false);
        let account_tab = host
            .query_selector("#account-tab-account")
            .unwrap()
            .unwrap();
        let devices_tab = host
            .query_selector("#account-tab-devices")
            .unwrap()
            .unwrap();
        assert_eq!(
            account_tab.get_attribute("aria-selected").as_deref(),
            Some("false")
        );
        assert_eq!(account_tab.get_attribute("tabindex").as_deref(), Some("-1"));
        assert_eq!(
            devices_tab.get_attribute("aria-selected").as_deref(),
            Some("true")
        );
        assert_eq!(devices_tab.get_attribute("tabindex").as_deref(), Some("0"));
        assert!(
            host.query_selector("#account-pane-account")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
        assert!(
            !host
                .query_selector("#account-pane-devices")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
    }

    #[dialog_common::test]
    fn whole_account_deletion_is_unavailable_without_disabling_exact_space_review() {
        let _query = Query::set("");
        let whole = host();
        configure_deletion_entry(&whole);
        let whole_action: HtmlButtonElement = whole
            .query_selector("#account-delete-review")
            .unwrap()
            .unwrap()
            .unchecked_into();
        assert!(whole_action.disabled());
        assert_eq!(
            whole_action.text_content().as_deref(),
            Some("account deletion temporarily unavailable")
        );
        assert_eq!(
            whole
                .query_selector("#account-delete-description")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some(
                "Secure account deletion is temporarily unavailable while Tonk strengthens its authorization. Your account, spaces, and local data are unchanged."
            )
        );

        let _exact_query = Query::set("?delete-space=did%3Akey%3AzOwned");
        let exact = host();
        configure_deletion_entry(&exact);
        let exact_action: HtmlButtonElement = exact
            .query_selector("#account-delete-review")
            .unwrap()
            .unwrap()
            .unchecked_into();
        assert!(!exact_action.disabled());
        assert_eq!(
            exact_action.text_content().as_deref(),
            Some("Review selected space deletion")
        );
    }

    fn roster_fixture() -> tonk_worker_api::ProfilesResponse {
        tonk_worker_api::ProfilesResponse {
            active: "tonk".into(),
            profiles: vec![
                tonk_worker_api::ProfileRosterEntry {
                    profile_name: "tonk".into(),
                    root_did: Some("did:key:zRootA".into()),
                    provider: Some("https://accounts.example".into()),
                    email: Some("person@example.com".into()),
                    display_name: Some("Alice".into()),
                    active: true,
                },
                tonk_worker_api::ProfileRosterEntry {
                    profile_name: "tonk-0a12".into(),
                    root_did: None,
                    provider: None,
                    email: None,
                    display_name: Some("brave-otter".into()),
                    active: false,
                },
            ],
        }
    }

    #[dialog_common::test]
    fn it_renders_local_and_account_roster_rows() {
        let host = host();
        render_profiles(&host, &roster_fixture());

        let list = host
            .query_selector("#account-profile-list")
            .unwrap()
            .unwrap();
        assert_eq!(list.query_selector_all("li").unwrap().length(), 2);
        let text = list.text_content().unwrap();
        assert!(
            text.contains("person@example.com"),
            "an account row shows its email"
        );
        assert!(
            text.contains("local workspace"),
            "a never-signed-in row says what it is"
        );
        assert!(text.contains("Alice") && text.contains("brave-otter"));

        let button = list
            .query_selector("button[data-activate=\"tonk-0a12\"]")
            .unwrap()
            .expect("the other profile's row offers a switch");
        assert!(button.text_content().unwrap_or_default().is_empty());
        assert_eq!(
            button.get_attribute("aria-label").as_deref(),
            Some("Switch to brave-otter")
        );
    }

    #[dialog_common::test]
    fn it_marks_the_active_profile_row_inert() {
        let host = host();
        render_profiles(&host, &roster_fixture());

        let list = host
            .query_selector("#account-profile-list")
            .unwrap()
            .unwrap();
        let active = list
            .query_selector("li[data-active]")
            .unwrap()
            .expect("the active profile renders a marked row");
        assert!(active.text_content().unwrap().contains("current"));
        assert!(
            active.query_selector("button").unwrap().is_none(),
            "the active row must not offer an action"
        );
        assert_eq!(
            list.query_selector_all("button[data-activate]")
                .unwrap()
                .length(),
            1,
            "only the other rows are switch targets"
        );
    }

    #[dialog_common::test]
    fn it_offers_a_different_account_from_the_choice_panel_when_a_root_is_persisted() {
        let host = host();

        render_choice_profiles(&host, &roster_fixture(), true);
        let button = host
            .query_selector("#account-use-different-account")
            .unwrap()
            .unwrap();
        assert!(
            !button.has_attribute("hidden"),
            "a persisted root must surface the way to another account"
        );
        let section = host
            .query_selector("#account-choice-profiles")
            .unwrap()
            .unwrap();
        assert!(
            !section.has_attribute("hidden"),
            "other roster entries render on the choice panel"
        );
        let list = host
            .query_selector("#account-choice-profile-list")
            .unwrap()
            .unwrap();
        assert_eq!(
            list.query_selector_all("li").unwrap().length(),
            1,
            "the compact list shows only the OTHER profiles"
        );

        render_choice_profiles(&host, &roster_fixture(), false);
        assert!(
            button.has_attribute("hidden"),
            "without a persisted root, plain log-in suffices"
        );
    }

    fn deletion_plan() -> AccountDeletionPlan {
        AccountDeletionPlan {
            root_did: "did:key:zAccount".into(),
            email: "person@example.com".into(),
            spaces: vec![tonk_worker_api::AccountDeletionSpace {
                subject: "did:key:zSpace".into(),
                name: Some("Project One".into()),
                deleting_since: None,
            }],
            joined_spaces: 2,
        }
    }

    #[dialog_common::test]
    async fn it_opens_and_closes_the_authored_confirmation_with_focus_restoration() {
        let host = mounted_account_host().await;
        // The trigger lives on the success panel, which the template
        // keeps `hidden` until a mode selects it — and focusing an
        // element inside a hidden subtree silently no-ops, which would
        // leave nothing real to restore. Reach the state the button is
        // actually pressed in.
        set_mode(&host, "success");
        let trigger: HtmlElement = host
            .query_selector("#account-unlink")
            .unwrap()
            .unwrap()
            .unchecked_into();
        trigger.focus().unwrap();

        open_confirmation(&host, Confirmation::SignOut).unwrap();
        let surface = host
            .query_selector("#account-confirmation")
            .unwrap()
            .unwrap();
        assert!(!surface.has_attribute("hidden"));
        assert_eq!(
            window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .id(),
            "account-confirm-cancel"
        );

        close_confirmation(&host);
        assert!(surface.has_attribute("hidden"));
        assert_eq!(
            window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .id(),
            "account-unlink"
        );
        host.remove();
    }

    #[dialog_common::test]
    async fn it_loops_keyboard_focus_inside_the_authored_confirmation() {
        let host = mounted_account_host().await;
        open_confirmation(&host, Confirmation::SignOut).unwrap();
        let document = window().unwrap().document().unwrap();
        let first: HtmlElement = host
            .query_selector("#account-confirm-close")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let last: HtmlElement = host
            .query_selector("#account-delete-submit")
            .unwrap()
            .unwrap()
            .unchecked_into();

        first.focus().unwrap();
        let backwards = web_sys::KeyboardEventInit::new();
        backwards.set_key("Tab");
        backwards.set_shift_key(true);
        let backwards =
            KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &backwards).unwrap();
        trap_confirmation_key(&host, &backwards);
        assert_eq!(document.active_element().unwrap().id(), last.id());

        let forwards = web_sys::KeyboardEventInit::new();
        forwards.set_key("Tab");
        let forwards =
            KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &forwards).unwrap();
        trap_confirmation_key(&host, &forwards);
        assert_eq!(document.active_element().unwrap().id(), first.id());
        host.remove();
    }

    #[dialog_common::test]
    fn it_requires_exact_deletion_arming_and_replaces_pending_operations() {
        let host = host();
        open_confirmation(&host, Confirmation::SignOut).unwrap();
        open_confirmation(
            &host,
            Confirmation::Revoke {
                did: "did:key:zDevice".into(),
                self_revoke: false,
            },
        )
        .unwrap();
        assert_eq!(
            confirmation(&host),
            Some(Confirmation::Revoke {
                did: "did:key:zDevice".into(),
                self_revoke: false,
            })
        );

        open_confirmation(
            &host,
            Confirmation::Delete {
                plan: deletion_plan(),
                requested_space: None,
            },
        )
        .unwrap();
        let submit: HtmlButtonElement = host
            .query_selector("#account-delete-submit")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let email: HtmlInputElement = host
            .query_selector("#account-delete-email")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let understood: HtmlInputElement = host
            .query_selector("#account-delete-understood")
            .unwrap()
            .unwrap()
            .unchecked_into();
        assert!(submit.disabled());
        email.set_value("wrong@example.com");
        understood.set_checked(true);
        update_confirmation_arming(&host);
        assert!(submit.disabled());
        email.set_value("person@example.com");
        update_confirmation_arming(&host);
        assert!(!submit.disabled());
    }

    #[dialog_common::test]
    fn it_keeps_every_intermediary_mode_in_the_ceremony_grammar() {
        let host = host();
        for selector in [
            "#account-choice.account__ceremony",
            "#account-create.account__ceremony",
            "#account-link.account__ceremony",
            "#account-handoff.account__ceremony",
        ] {
            let panel = host.query_selector(selector).unwrap().expect(selector);
            assert!(
                panel
                    .query_selector(".account__ceremony-head")
                    .unwrap()
                    .is_some()
            );
            assert!(
                panel
                    .query_selector(".account__narrator")
                    .unwrap()
                    .is_some()
            );
            assert!(panel.query_selector(".account__ghost").unwrap().is_some());
        }
        assert_eq!(
            host.query_selector("#account-email")
                .unwrap()
                .unwrap()
                .get_attribute("type")
                .as_deref(),
            Some("email")
        );
        assert!(
            host.query_selector("#account-handoff details")
                .unwrap()
                .is_some(),
            "technical DIDs stay behind optional disclosure"
        );
    }

    #[dialog_common::test]
    fn it_renders_recorded_and_legacy_passkey_facts_without_guessing() {
        let host = host();
        render_summary(
            &host,
            &tonk_worker_api::AccountSummary {
                display_name: None,
                email: Some("person@example.com".into()),
                passkey: Some(tonk_worker_api::PasskeyMetadata {
                    created_at: 1_754_380_800,
                    created_on: "Chrome on macOS".into(),
                }),
            },
        );
        assert_eq!(
            host.query_selector("#account-email-value")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("person@example.com")
        );
        assert_eq!(
            host.query_selector("#account-passkey-device-value")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Chrome on macOS")
        );

        render_summary(
            &host,
            &tonk_worker_api::AccountSummary {
                display_name: None,
                email: Some("legacy@example.com".into()),
                passkey: None,
            },
        );
        assert_eq!(
            host.query_selector("#account-passkey-created-value")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Unavailable")
        );
        assert!(
            host.query_selector("#account-passkey-detail")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
                .contains("cannot reliably reconstruct")
        );
    }

    #[dialog_common::test]
    fn it_renders_passkey_facts_without_a_verified_email() {
        let host = host();
        render_summary(
            &host,
            &tonk_worker_api::AccountSummary {
                display_name: None,
                email: None,
                passkey: Some(tonk_worker_api::PasskeyMetadata {
                    created_at: 1_754_380_800,
                    created_on: "Chrome on macOS".into(),
                }),
            },
        );

        assert_eq!(
            host.query_selector("#account-email-value")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Unavailable"),
            "the verified address lives only at the account service"
        );
        assert_eq!(
            host.query_selector("#account-passkey-device-value")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Chrome on macOS")
        );
        let created = host
            .query_selector("#account-passkey-created-value")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap();
        assert!(
            !created.is_empty() && created != "Unavailable",
            "the account repository carries the creation date without the provider"
        );
    }

    #[dialog_common::test]
    fn it_does_not_author_fixed_browser_registration_names() {
        let host = host();
        assert!(
            host.query_selector("#account-create-device-name")
                .unwrap()
                .is_none()
        );
        assert!(
            host.query_selector("#account-link-device-name")
                .unwrap()
                .is_none()
        );
        assert!(!host.inner_html().contains("This browser"));
    }

    #[dialog_common::test]
    fn it_distinguishes_self_revocation_in_the_status_line() {
        let acknowledgement = RevokeDeviceAcknowledgement {
            target_did: "did:key:device".into(),
            target_cid: "bafycid".into(),
            published: true,
        };
        assert_eq!(
            revocation_status(&acknowledgement, false),
            "Access removed."
        );
        assert_eq!(
            revocation_status(&acknowledgement, true),
            "Access removed from this device."
        );
    }

    /// No account form may navigate.
    ///
    /// Enter in a single-field form implicitly submits it — the email panel
    /// has exactly one field and, before this, no submit button, so the
    /// browser submitted the form itself: a GET to the current URL with no
    /// action, which reloaded `/settings?Email=…`, threw the typed address
    /// away, and never ran the handler that sends the code. Nothing about
    /// that is specific to the email panel, so the guard covers every form
    /// the panel authors.
    #[dialog_common::test]
    async fn it_prevents_every_account_form_from_navigating() {
        let host = mounted_account_host().await;
        bind(&host);
        let forms = host.query_selector_all(".account__form").expect("query");
        assert!(forms.length() > 0, "the panel authors forms to guard");

        let mut unguarded = Vec::new();
        for index in 0..forms.length() {
            let form: web_sys::Element = forms.item(index).expect("form").unchecked_into();
            let init = web_sys::EventInit::new();
            init.set_cancelable(true);
            init.set_bubbles(true);
            let event =
                web_sys::Event::new_with_event_init_dict("submit", &init).expect("submit event");
            form.dispatch_event(&event).expect("dispatch");
            if !event.default_prevented() {
                unguarded.push(form.parent_element().map(|panel| panel.id()));
            }
        }
        host.remove();

        assert!(
            unguarded.is_empty(),
            "these forms would navigate on Enter: {unguarded:?}",
        );
    }

    /// Enter has to do what Create account does, not nothing. Implicit
    /// submission clicks the form's submit button, and that click is what
    /// carries the creation handler — so the button has to be the form's
    /// submit button rather than an inert `type="button"` beside it.
    #[dialog_common::test]
    fn it_lets_enter_submit_account_creation() {
        let host = host();
        let button = host
            .query_selector("#account-create-submit")
            .expect("query")
            .expect("create button");
        assert_eq!(
            button.get_attribute("type").as_deref(),
            Some("submit"),
            "Create account must be the email form's submit button",
        );
    }

    #[dialog_common::test]
    fn it_switches_between_account_panels_without_reauthoring_the_dom() {
        let host = host();
        set_mode(&host, "link");
        assert!(
            host.query_selector("#account-link")
                .unwrap()
                .unwrap()
                .get_attribute("hidden")
                .is_none()
        );
        assert!(
            host.query_selector("#account-create")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
    }

    #[dialog_common::test]
    async fn it_prefers_an_explicit_service_attribute_over_deployment_config() {
        let host = host();
        host.set_attribute("service", "http://127.0.0.1:8787")
            .unwrap();
        assert_eq!(service(&host).await.unwrap(), "http://127.0.0.1:8787");
    }

    #[dialog_common::test]
    async fn it_renders_the_device_list_with_a_this_device_marker() {
        let host = mounted_account_host().await;
        let devices = vec![
            tonk_worker_api::AccountDevice {
                did: "did:key:zThis".into(),
                name: "This browser".into(),
                created_at: 1_753_300_000,
            },
            tonk_worker_api::AccountDevice {
                did: "did:key:zPhone".into(),
                name: "Phone".into(),
                created_at: 1_753_100_000,
            },
        ];
        render_devices(&host, &devices, "did:key:zThis");

        let list = host
            .query_selector("#account-device-list")
            .unwrap()
            .unwrap();
        let items = list.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 2);
        let text = list.text_content().unwrap();
        assert!(text.contains("This browser"));
        assert!(text.contains("this device"));
        assert!(text.contains("added"));
        // Every row is an active grant, so every row is removable — no
        // stored path evidence or passkey involved.
        assert_eq!(
            list.query_selector_all("button[data-revoke]")
                .unwrap()
                .length(),
            2
        );
        assert!(
            list.query_selector("button[data-self-revoke]")
                .unwrap()
                .is_some()
        );
        let button = list
            .query_selector("button[data-revoke=\"did:key:zPhone\"]")
            .unwrap()
            .expect("another device's row has a revoke button");
        assert_eq!(button.text_content().as_deref(), Some("remove access"));
    }

    /// A `?revoke=` deep link must land on the device list, where the
    /// ceremony runs — parking a linked browser on the success screen
    /// leaves the CLI polling until it times out.
    #[dialog_common::test]
    fn it_routes_a_revoke_deep_link_to_the_device_list() {
        assert_eq!(
            landing(Some(AccountStateStatus::Unconfigured), true, false),
            Landing::Devices
        );
        assert_eq!(
            landing(Some(AccountStateStatus::Ready), false, false),
            Landing::Success
        );
        assert_eq!(
            landing(None, true, false),
            Landing::Choice { revoke_hint: true }
        );
        assert_eq!(
            landing(None, false, false),
            Landing::Choice { revoke_hint: false }
        );
    }

    /// Opening Add account is only a choice screen. It must not reuse the
    /// active account's dashboard, and reaching this screen must not itself
    /// rotate the worker onto a new profile.
    #[dialog_common::test]
    fn it_offers_account_choices_before_preparing_an_added_profile() {
        assert_eq!(
            landing(Some(AccountStateStatus::Ready), false, true),
            Landing::Choice { revoke_hint: false }
        );
    }
}
