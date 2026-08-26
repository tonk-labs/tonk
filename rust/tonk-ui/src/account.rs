//! Top-document account creation and passkey self-link surface.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, window};

use tonk_account::AccountStateStatus;

use crate::registration_notice;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountSpaceDeletionRequest, AccountStatus,
    RevokeDeviceAcknowledgement, RootStatus,
};

use crate::identity_bridge::{
    CeremonyOutput, CreateAccountInput, EnrollCustodyInput, UnlockWithPasskeyInput,
    VerifyPasskeyInput, create_account, enroll_custody_passkey, unlock_with_passkey,
    verify_passkey,
};

const STYLE_ID: &str = "tonk-account-styles";
/// Where a pending callback authorization's `(audience, callback)` is parked.
const CALLBACK: &str = "__tonkCliCallback";
const DELETION_PLAN: &str = "__tonkAccountDeletionPlan";

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
    Ok(crate::deployment::get()
        .await?
        .account_service_url
        .to_string())
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

fn set_busy(host: &HtmlElement, busy: bool, status: &str) {
    for selector in [
        "#account-entry-submit",
        "#account-choose-link",
        "#account-create-submit",
        "#account-create-back",
        "#account-link-submit",
        "#account-link-back",
        "#account-handoff-submit",
        "#account-unlink",
        "#account-delete-review",
        "#account-delete-submit",
        "#account-add-profile",
        "#account-use-different-account",
    ] {
        if let Ok(Some(button)) = host.query_selector(selector)
            && let Ok(button) = button.dyn_into::<HtmlButtonElement>()
        {
            button.set_disabled(busy);
        }
    }
    if let Ok(Some(element)) = host.query_selector("#account-working") {
        element.set_text_content((!status.is_empty()).then_some(status));
    }
}

fn show_error(host: &HtmlElement, message: impl AsRef<str>) {
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(Some(message.as_ref()));
        let _ = error.remove_attribute("hidden");
    }
}

fn clear_error(host: &HtmlElement) {
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(None);
        let _ = error.set_attribute("hidden", "");
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
    load_summary(host.clone());
    load_devices(host.clone());
    load_profiles(host.clone());
    load_activation_notice(host.clone());
}

/// Surface a pending customer activation on the dashboard. Quiet on
/// every other answer: an active customer needs no notice, and a
/// deployment without registration should not decorate the panel with
/// its absence.
/// Say when the account's backup is still on its way. The ceremony
/// pre-signed the publish, the worker drains it on activation, and no
/// interaction is needed — this only keeps the wait legible.
async fn note_pending_backup(host: &HtmlElement) {
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
        return;
    }
    set_text(
        host,
        "#account-backup-notice",
        "Finishing your account's backup…",
    );
    let _ = show(host, "#account-backup-notice");
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
            return;
        }
        let state = match crate::api::customer_state().await {
            Ok(state) => state,
            Err(_) => {
                set_text(&host, "#account-registration-value", "Unreachable");
                return;
            }
        };
        // A linked account the access service does not know is one that
        // predates registration (or the service's control state was
        // reset). This signed-in browser is the only party that can fix
        // that — registration is web-only — so enroll right here, with
        // the device-chained deposit since no ceremony is at hand, and
        // fall through to the ordinary pending notice.
        if state["status"].is_null()
            && crate::deployment::get()
                .await
                .is_ok_and(|config| config.service_did.is_some())
        {
            match crate::api::enroll_customer(None, &[]).await {
                // Enrollment is a command, so this returns once the
                // transient is committed, not once the service answers.
                // Re-reading here would race the handler and paint a
                // state already superseded; the row's subscription is
                // what shows the outcome, whenever it lands.
                Ok(()) => {}
                Err(error) => {
                    web_sys::console::error_1(
                        &format!("customer re-enrollment failed: {error}").into(),
                    );
                    set_text(
                        &host,
                        "#account-registration-value",
                        "Not registered — reload to retry",
                    );
                    return;
                }
            }
        }
        // Paint what this read says, then follow the fact instead of
        // asking again. The activation link is routinely opened
        // somewhere else -- another tab, or the phone the email is on --
        // and a dashboard that only learned about it on a manual reload
        // looked stuck at "pending" long after the account was live.
        //
        // A subscription rather than a poll because the state is a fact
        // on profile main: whoever confirms writes it, sync carries it,
        // and every tab showing this row repaints. No timer to tune, and
        // it costs nothing while nothing changes.
        render_registration(&host, &state).await;
        watch_registration(&host);
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
    // The worker drains the queued backup itself once activation
    // lands — the ceremony pre-signed the publish. Nothing to raise;
    // just say so while it is still on its way.
    if state["status"].as_str() == Some("Active") {
        note_pending_backup(host).await;
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

/// The tag the registration row's subscription carries.
const REGISTRATION_TAG: &str = "account-registration";

/// The tag the entry's address lookup carries.
const ENTRY_TAG: &str = "account-email-status";

/// Which subscription a frame belongs to.
///
/// Both subscriptions land on this one element, so every delegate is
/// handed every frame and has to recognise its own. The tag is what the
/// subscription asked to be called back with.
fn frame_tag(opts: &JsValue) -> Option<String> {
    Reflect::get(opts, &"tag".into()).ok()?.as_string()
}

/// Keep the registration row following the fact.
///
/// The row's first paint comes from the dashboard's own read; this is
/// what keeps it honest afterwards. Registration state is a fact on
/// profile main, so an activation performed anywhere — another device,
/// or a phone opening the emailed link — reaches this tab through sync,
/// and the row repaints without a reload.
///
/// Idempotent: the dashboard re-renders on several paths and only the
/// first call subscribes. The subscription and its delegates are parked
/// on the element, which outlives them.
fn watch_registration(host: &HtmlElement) {
    if Reflect::get(host.as_ref(), &"__tonkRegistrationWatch".into())
        .map(|value| value.is_truthy())
        .unwrap_or(false)
    {
        return;
    }
    let host = host.clone();
    spawn_local(async move {
        let Ok(RootStatus::Ready { root_did, .. }) = crate::api::root_status().await else {
            // No account on this profile, so no registration to follow.
            return;
        };
        if !host.is_connected() {
            return;
        }
        let _ = Reflect::set(
            host.as_ref(),
            &"__tonkRegistrationWatch".into(),
            &JsValue::TRUE,
        );

        let painter = host.clone();
        let reset: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if frame_tag(&opts).as_deref() != Some(REGISTRATION_TAG) {
                    return;
                }
                paint_registration(&painter, registration_notice::read_frame(&payload));
            }));
        let _ = Reflect::set(
            host.as_ref(),
            &"__tonkRegistrationReset".into(),
            reset.as_ref(),
        );
        reset.forget();

        let painter = host.clone();
        let update: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if frame_tag(&opts).as_deref() != Some(REGISTRATION_TAG) {
                    return;
                }
                // A delta with no assertion is a moment mid-write, not a
                // state: leave the row where it is rather than blanking
                // it and flickering.
                if let Some(registration) = registration_notice::read_delta(&payload) {
                    paint_registration(&painter, registration);
                }
            }));
        let _ = Reflect::set(
            host.as_ref(),
            &"__tonkRegistrationUpdate".into(),
            update.as_ref(),
        );
        update.forget();

        match registration_notice::subscribe(&host, &root_did) {
            // The subscription cancels upstream when dropped, and this
            // row lives as long as the panel does, so it is parked on
            // the element rather than held in a local.
            Ok(subscription) => {
                let kept = Closure::<dyn FnMut()>::new(move || {
                    let _ = &subscription;
                });
                let _ = Reflect::set(
                    host.as_ref(),
                    &"__tonkRegistrationSub".into(),
                    kept.as_ref(),
                );
                kept.forget();
            }
            Err(error) => {
                // The row keeps this render's snapshot; only liveness is
                // lost, and reloading still shows the truth.
                web_sys::console::warn_1(&format!("registration watch: {error}").into());
            }
        }
    });
}

/// Paint the registration row and its pending banner.
fn paint_registration(host: &HtmlElement, registration: registration_notice::Registration) {
    set_text(host, "#account-registration-value", registration.label());
    let notice = registration.notice();
    if let Ok(Some(element)) = host.query_selector("#account-activation-notice") {
        match &notice {
            Some(message) => {
                element.set_text_content(Some(message));
                let _ = element.remove_attribute("hidden");
            }
            None => {
                let _ = element.set_attribute("hidden", "");
            }
        }
    }
    // The way out of a stuck pending state, and only offered while
    // there is something to resend.
    if let Ok(Some(resend)) = host.query_selector("#account-resend-activation") {
        if notice.is_some() {
            let _ = resend.remove_attribute("hidden");
        } else {
            let _ = resend.set_attribute("hidden", "");
        }
    }
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

fn configure_deletion_entry(host: &HtmlElement) {
    if requested_space_deletion().is_none() {
        return;
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

fn render_deletion_plan(host: &HtmlElement, plan: &AccountDeletionPlan) -> Result<(), String> {
    let panel = host
        .query_selector("#account-delete-review-panel")
        .ok()
        .flatten()
        .ok_or_else(|| "missing deletion review panel".to_string())?;
    let _ = panel.remove_attribute("hidden");
    let requested = requested_space_deletion();
    let visible: Vec<_> = plan
        .spaces
        .iter()
        .filter(|space| {
            requested
                .as_deref()
                .is_none_or(|subject| space.subject == subject)
        })
        .collect();
    if let Some(subject) = requested.as_deref()
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
        item.set_text_content(Some(&format!("{label} — {}", space.state)));
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
    let value = serde_wasm_bindgen::to_value(plan)
        .map_err(|_| "could not retain deletion plan".to_string())?;
    Reflect::set(host.as_ref(), &DELETION_PLAN.into(), &value)
        .map_err(|_| "could not retain deletion plan".to_string())?;
    focus_input(host, "#account-delete-email");
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
    spawn_local(async move {
        match crate::api::account_summary().await {
            Ok(summary) => render_summary(&host, &summary),
            Err(error) => {
                for selector in [
                    "#account-email-value",
                    "#account-passkey-created-value",
                    "#account-passkey-device-value",
                ] {
                    set_text(&host, selector, "Unavailable");
                }
                web_sys::console::warn_1(&format!("account summary unavailable: {error}").into());
            }
        }
    });
}

/// Land a signed-in device where it was going.
///
/// [`crate::account_gate::finish`] returns to the `next` this page was
/// opened with; only without one does the success panel show, which is the
/// case where the user came to `/account` on their own.
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
            Err(error) => show_error(
                &host,
                format!("You're signed in, but we couldn't finish what you started: {error}"),
            ),
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
            marker.set_text_content(Some("This device"));
            let _ = identity.append_child(&marker);
        }

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__device-meta");
        let date = js_sys::Date::new(&JsValue::from_f64(device.created_at as f64 * 1000.0))
            .to_locale_date_string("default", &JsValue::UNDEFINED);
        meta.set_text_content(Some(&format!("Added {}", String::from(date))));

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
        let _ = button.set_attribute("aria-label", &format!("Remove access for {}", device.name));
        if this_device {
            let _ = button.set_attribute("data-self-revoke", "true");
        }
        button.set_text_content(Some("Remove access"));
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
            marker.set_text_content(Some("Current"));
            let _ = identity.append_child(&marker);
        }

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__profile-meta");
        meta.set_text_content(Some(match &entry.email {
            Some(email) => email,
            None if entry.provider.is_some() => "Signed in",
            None => "Local workspace",
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
            button.set_text_content(Some("Switch"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

/// Fill the signed-in dashboard's switcher section.
fn render_profiles(host: &HtmlElement, profiles: &tonk_worker_api::ProfilesResponse) {
    render_profile_rows(host, "#account-profile-list", profiles, false);
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
        match crate::api::list_profiles().await {
            Ok(profiles) => render_profiles(&host, &profiles),
            Err(error) => {
                web_sys::console::warn_1(&format!("profile roster unavailable: {error}").into());
            }
        }
    });
}

fn load_choice_profiles(host: HtmlElement, root_persisted: bool) {
    spawn_local(async move {
        match crate::api::list_profiles().await {
            Ok(profiles) => render_choice_profiles(&host, &profiles, root_persisted),
            Err(error) => {
                web_sys::console::warn_1(&format!("profile roster unavailable: {error}").into());
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
    set_mode(&host, "success");
    set_busy(&host, true, "Loading devices…");
    spawn_local(async move {
        // Which row is this device is answered separately from the list:
        // the rows are shared facts, identical everywhere, and identity
        // is the one thing only this device can answer for itself.
        let own = match crate::api::identify().await {
            Ok(identity) => identity.did,
            Err(error) => {
                set_busy(&host, false, "");
                show_error(&host, error.to_string());
                return;
            }
        };
        match crate::api::account_devices().await {
            Ok(devices) => {
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
                set_busy(&host, false, "");
                show_error(&host, error.to_string());
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

fn landing(account_state: Option<AccountStateStatus>, revoke_target: bool) -> Landing {
    match (account_state, revoke_target) {
        (Some(_), true) => Landing::Devices,
        (Some(_), false) => Landing::Success,
        (None, revoke_hint) => Landing::Choice { revoke_hint },
    }
}

fn load_status(host: HtmlElement) {
    let handoff_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/account/link" || path.starts_with("/account/link/"));
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
                    match crate::api::account_status().await {
                        Ok(AccountStatus::Registered { .. }) => {
                            load_callback_request(host, audience, callback, name);
                        }
                        Ok(status) => {
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
                            set_busy(&host, false, "");
                            set_mode(&host, "choice");
                            show_error(&host, error.to_string());
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
    spawn_local(async move {
        if let Err(error) = service(&host).await {
            set_busy(&host, false, "");
            set_mode(&host, NO_PANEL_MODE);
            show_error(&host, error);
            return;
        }
        match crate::api::account_status().await {
            Ok(status) => {
                // A persisted root with no provider is a signed-out
                // profile: logging in here with a DIFFERENT passkey is
                // refused, so the Choice panel offers a fresh profile.
                let root_persisted = matches!(status, AccountStatus::Unregistered { .. });
                let account_state = match status {
                    AccountStatus::Registered { account_state, .. } => Some(account_state),
                    AccountStatus::RootMissing { .. } | AccountStatus::Unregistered { .. } => None,
                };
                match landing(account_state, revoke_target_from_url().is_some()) {
                    Landing::Devices => load_devices(host),
                    Landing::Success => {
                        settle_on_load(&host);
                        apply_link_outcome(&host, link_outcome.as_ref());
                        if account_state == Some(AccountStateStatus::Unhydrated) {
                            show_error(
                                &host,
                                "Account state is not synchronized yet. Reload /account to retry before changing your account name.",
                            );
                        }
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
                set_busy(&host, false, "");
                set_mode(&host, "choice");
                show_error(&host, error.to_string());
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
        set_text(
            host,
            "#account-success-message",
            "Command-line device linked.",
        );
    } else {
        let message = message
            .as_deref()
            .unwrap_or("the command-line link did not complete");
        show_error(host, format!("Command-line link failed: {message}."));
    }
}

/// The loopback URL a `tonk account login` run is waiting on, if any.
///
/// The waiting process's audience and callback ride the query, so the
/// approval never touches the account service.
fn pending_callback_request() -> Option<(String, String, String)> {
    let on_link_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/account/link" || path.starts_with("/account/link/"));
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

/// Approve a waiting command-line profile and post the grant straight back.
///
/// The page runs the passkey ceremony, mints the `account → profile`
/// powerline, and delivers it to the loopback listener the CLI is holding
/// open. Delivery is a form POST rather than `fetch`: a cross-origin form
/// submission needs no preflight and no permissive CORS header on a server
/// that exists for one request. This page renders in the top document, not
/// the sealed guest, so the submission is not subject to an iframe sandbox.
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
    set_busy(&host, false, "");
    set_mode(&host, "handoff");
}

/// Where the CLI's callback should send this tab once the terminal has
/// its answer: the account page, which renders the `?link=` outcome in
/// its own styling.
fn link_outcome_redirect() -> String {
    window()
        .and_then(|window| window.location().origin().ok())
        .map(|origin| format!("{origin}/account"))
        .unwrap_or_else(|| "/account".to_string())
}

/// Base64-encode an authorization payload for form delivery.
///
/// The callback decodes base64 before parsing, so the payload survives form
/// encoding without the caller having to reason about escaping.
pub(crate) fn encode_authorization(payload: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(payload)
}

/// Deliver an authorization to the waiting process by form POST.
fn post_to_callback(callback: &str, fields: &[(&str, &str)]) -> Result<(), String> {
    let document = window()
        .and_then(|window| window.document())
        .ok_or("document is unavailable")?;
    let form = document
        .create_element("form")
        .map_err(|_| "could not build the delivery form")?;
    form.set_attribute("method", "POST")
        .map_err(|_| "could not address the delivery form")?;
    form.set_attribute("action", callback)
        .map_err(|_| "could not address the delivery form")?;
    for (name, value) in fields {
        let input = document
            .create_element("input")
            .map_err(|_| "could not build the delivery form")?;
        let _ = input.set_attribute("type", "hidden");
        let _ = input.set_attribute("name", name);
        let _ = input.set_attribute("value", value);
        let _ = form.append_child(&input);
    }
    let body = document.body().ok_or("document has no body")?;
    body.append_child(&form)
        .map_err(|_| "could not attach the delivery form")?;
    form.unchecked_ref::<web_sys::HtmlFormElement>()
        .submit()
        .map_err(|_| "could not deliver the authorization".to_owned())
}

async fn persist(
    provider: &str,
    ceremony: &CeremonyOutput,
    descriptor_hex: String,
    initialize_name: bool,
) -> Result<AccountStatus, String> {
    let root_status = crate::api::root_status()
        .await
        .map_err(|error| error.to_string())?;
    let root = root_for_link(&root_status, ceremony);
    if root.needs_persist {
        crate::api::save_root(
            root.credential_id.to_string(),
            root.delegation_hex.to_string(),
            None,
            ceremony.encryption_key.clone(),
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    crate::api::save_account_link(
        provider.to_string(),
        root.root_did.to_string(),
        root.credential_id.to_string(),
        root.delegation_hex.to_string(),
        descriptor_hex,
        initialize_name,
    )
    .await
    .map_err(|error| error.to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct LinkRoot<'a> {
    root_did: &'a str,
    credential_id: &'a str,
    delegation_hex: &'a str,
    needs_persist: bool,
}

/// Select the grant the worker should attach after a remote link succeeds.
///
/// A same-root browser re-login reuses the still-active server generation, so
/// it must also reuse the local grant that generation records. A missing or
/// different root continues through the normal root-persistence boundary.
fn root_for_link<'a>(
    status: &'a tonk_worker_api::RootStatus,
    ceremony: &'a CeremonyOutput,
) -> LinkRoot<'a> {
    match status {
        tonk_worker_api::RootStatus::Ready {
            root_did,
            credential_id,
            delegation_hex,
            ..
        } if root_did == &ceremony.root_did => LinkRoot {
            root_did,
            credential_id,
            delegation_hex,
            needs_persist: false,
        },
        _ => LinkRoot {
            root_did: &ceremony.root_did,
            credential_id: &ceremony.credential_id,
            delegation_hex: &ceremony.delegation_hex,
            needs_persist: true,
        },
    }
}

/// What to tell someone whose account predates the repository descriptor.
///
/// They cannot establish one from here: the setup panel runs against an
/// existing local link, and this browser has none. An already-linked device can
/// do it, and afterwards this one signs in normally.
const UNESTABLISHED_ACCOUNT_GUIDANCE: &str = "This account was created before shared account state existed, so it can't be added to a new \
     browser yet. Open /account on a browser that is already signed in to this account and \
     finish account setup there, then sign in here.";

/// Whether this deployment registers accounts with an access service at
/// all: deployments that publish no service identity have nothing to
/// enroll with.
async fn wants_enrollment() -> bool {
    deployment_service_did().await.is_some()
}

/// The access-service DID this deployment publishes, for ceremonies that
/// mint account-signed deposits. Absent config or identity is ordinary:
/// the ceremony then mints nothing and enrollment falls back to a
/// device-issued deposit.
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

/// Read the exact stored descriptor the account service selected.
fn descriptor_hex(response: &serde_json::Value) -> Result<String, String> {
    response
        .get("descriptorHex")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "account service omitted descriptorHex".to_string())
}

/// Whether a link exists but its account repository has no trusted base yet.
fn is_unhydrated(status: &AccountStatus) -> bool {
    matches!(
        status,
        AccountStatus::Registered {
            account_state: AccountStateStatus::Unhydrated,
            ..
        }
    )
}

async fn complete_remote(
    host: &HtmlElement,
    path: &str,
    ceremony: CeremonyOutput,
    initialize_name: bool,
    enroll_email: Option<&str>,
) -> Result<(), String> {
    let provider = service(host).await?;
    let response = crate::api::submit_account_ceremony(&provider, path, &ceremony.invocation_hex)
        .await
        .map_err(|error| {
            let error = error.to_string();
            if error.contains(tonk_account::UNESTABLISHED_ACCOUNT_CONFLICT) {
                UNESTABLISHED_ACCOUNT_GUIDANCE.to_string()
            } else {
                error
            }
        })?;
    let status = match persist(
        &provider,
        &ceremony,
        descriptor_hex(&response)?,
        initialize_name,
    )
    .await
    {
        Ok(status) => status,
        Err(error) => {
            web_sys::console::error_1(
                &format!("failed to save the accepted account link: {error}").into(),
            );
            set_mode(host, "choice");
            return Err(
                "Your account is ready, but this browser couldn't finish signing in. Log in to continue."
                    .to_string(),
            );
        }
    };
    // Registration with the access service, on signup and login alike:
    // the account exists either way, so a failed enrollment does not
    // undo the attach. The login path names no email; the
    // worker resolves the account's recorded address. Deployments that
    // publish no service identity have no registration to perform.
    if wants_enrollment().await {
        set_busy(host, true, "Registering with the sync service…");
        // Enrollment is a command now, so this catches a dispatch that
        // never reached the worker, not a service that refused. A
        // refusal shows up where it belongs: the registration row
        // follows the fact, so it says what actually happened and keeps
        // saying it, instead of a one-shot verdict from this moment.
        if let Err(error) = crate::api::enroll_customer(enroll_email, &ceremony.deposits_hex).await
        {
            web_sys::console::error_1(&format!("customer enrollment failed: {error}").into());
            show_error(
                host,
                "Your account is ready, but registering it with the sync service could not be started. Reload /account to retry.",
            );
        }
    }
    // A pending callback approval takes precedence over settling: the
    // ceremony ran on the link page precisely to approve a waiting
    // device, and the account it just made is what the grant issues from.
    if let Some((audience, callback, name)) = pending_callback_request() {
        load_callback_request(host.clone(), audience, callback, name);
        return Ok(());
    }
    settle(host);
    // An unhydrated account right after signup is the expected state
    // while the email activation is pending — the access service
    // refuses the pull until then, the activation banner already says
    // so, and the background sweep hydrates once it lands. The alert is
    // for the unexpected case: activation is not what's in the way.
    if initialize_name && is_unhydrated(&status) && !activation_pending().await {
        show_error(
            host,
            "Your account was created, but its initial name could not be synchronized. Reload /account to retry account hydration.",
        );
    }
    Ok(())
}

/// Whether the customer is registered but not yet email-activated.
async fn activation_pending() -> bool {
    crate::api::customer_state()
        .await
        .is_ok_and(|state| state["status"].as_str() == Some("Registered"))
}

/// How long typing must pause before the address is looked up.
///
/// Long enough that walking through a domain does not spend a request
/// per keystroke, short enough that the button has settled by the time
/// a hand reaches it. The endpoint is rate limited per IP, so a shared
/// office address is a real budget and this is what keeps typing off it.
const LOOKUP_IDLE_MS: i32 = 600;

/// Where the entry form sends a user, once the lookup has answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryRoute {
    /// An account exists: run the assertion, not a creation.
    SignIn,
    /// Nothing holds this address.
    SignUp,
}

impl EntryRoute {
    /// The label the button carries, so the ceremony is never a
    /// surprise.
    fn label(self) -> &'static str {
        match self {
            Self::SignIn => "Sign in",
            Self::SignUp => "Create account",
        }
    }

    /// The stamp left on the form, read back when the button is clicked.
    fn stamp(self) -> &'static str {
        match self {
            Self::SignIn => "sign-in",
            Self::SignUp => "sign-up",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "sign-in" => Some(Self::SignIn),
            "sign-up" => Some(Self::SignUp),
            _ => None,
        }
    }
}

/// Read back what the lookup decided, if it has answered.
fn entry_answer(host: &HtmlElement) -> Option<EntryRoute> {
    host.query_selector("#account-entry-form")
        .ok()
        .flatten()
        .and_then(|form| form.get_attribute("data-route"))
        .as_deref()
        .and_then(EntryRoute::parse)
}

/// Look the address up once typing pauses, and label the button with
/// what it found.
///
/// The point is that a user never creates a passkey for an address that
/// already has one. Knowing which ceremony is coming is the secondary
/// benefit: a browser prompt is less alarming when the button that
/// summoned it said what it would do.
fn bind_entry_lookup(host: &HtmlElement) {
    let Ok(Some(field)) = host.query_selector("#account-entry-email") else {
        return;
    };
    watch_entry_status(host);
    let host = host.clone();
    let pending: Rc<RefCell<Option<i32>>> = Rc::new(RefCell::new(None));
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        // Every keystroke restarts the clock, so only the pause fires.
        if let Some(handle) = pending.borrow_mut().take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(handle);
        }
        // Typing invalidates the previous answer: a stale "Sign in" on a
        // half-edited address is worse than no label at all.
        clear_entry_answer(&host);
        let host = host.clone();
        let pending_inner = pending.clone();
        let tick = Closure::<dyn FnMut()>::once(move || {
            pending_inner.borrow_mut().take();
            run_entry_lookup(host);
        });
        if let Some(window) = web_sys::window()
            && let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                LOOKUP_IDLE_MS,
            )
        {
            *pending.borrow_mut() = Some(handle);
        }
        tick.forget();
    });
    let _ = field.add_event_listener_with_callback("input", closure.as_ref().unchecked_ref());
    closure.forget();
}

/// Forget what the last lookup decided.
fn clear_entry_answer(host: &HtmlElement) {
    if let Ok(Some(form)) = host.query_selector("#account-entry-form") {
        let _ = form.remove_attribute("data-route");
    }
    set_text(host, "#account-entry-submit", "Continue");
    if let Ok(Some(status)) = host.query_selector("#account-entry-status") {
        let _ = status.set_attribute("hidden", "");
    }
}

/// Follow the answers the worker publishes about typed addresses.
///
/// Installed once, before the first question is asked, so no answer can
/// land before there is something listening for it.
fn watch_entry_status(host: &HtmlElement) {
    if Reflect::get(host.as_ref(), &"__tonkEntryWatch".into())
        .map(|value| value.is_truthy())
        .unwrap_or(false)
    {
        return;
    }
    let _ = Reflect::set(host.as_ref(), &"__tonkEntryWatch".into(), &JsValue::TRUE);

    let painter = host.clone();
    let paint = move |payload: JsValue| {
        if let Some((address, state)) = registration_notice::read_email_status(&payload) {
            apply_entry_status(&painter, &address, &state);
        }
    };
    let reset_paint = paint.clone();
    let reset: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
            if frame_tag(&opts).as_deref() != Some(ENTRY_TAG) {
                return;
            }
            reset_paint(payload);
        }));
    let _ = Reflect::set(host.as_ref(), &"__tonkEntryReset".into(), reset.as_ref());
    reset.forget();

    let update: Closure<dyn FnMut(JsValue, JsValue)> =
        Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
            if frame_tag(&opts).as_deref() != Some(ENTRY_TAG) {
                return;
            }
            // A delta carries the assertion that replaced the last
            // answer; the entity is cardinality-one, so that is it.
            let asserted = Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
            paint(asserted);
        }));
    let _ = Reflect::set(host.as_ref(), &"__tonkEntryUpdate".into(), update.as_ref());
    update.forget();

    match registration_notice::subscribe_email_status(host) {
        Ok(subscription) => {
            let kept = Closure::<dyn FnMut()>::new(move || {
                let _ = &subscription;
            });
            let _ = Reflect::set(host.as_ref(), &"__tonkEntrySub".into(), kept.as_ref());
            kept.forget();
        }
        Err(error) => web_sys::console::warn_1(&format!("entry status watch: {error}").into()),
    }
}

/// Ask what is registered under the typed address.
///
/// The answer is not returned: the command publishes it as the
/// `EmailStatus` overlay fact, and [`watch_entry_status`] paints from
/// there. That is the same fact the share dialog's registration form
/// reads, so there is one implementation of what an address means.
fn run_entry_lookup(host: HtmlElement) {
    let Ok(email) = input(&host, "#account-entry-email") else {
        return;
    };
    spawn_local(async move {
        if let Err(error) = crate::api::check_email(&email).await {
            // A question that could not be asked leaves the entry as it
            // was. Both ceremonies still refuse correctly on their own,
            // so a missing label costs a surprise, not a wrong outcome.
            web_sys::console::warn_1(&format!("address lookup: {error}").into());
        }
    });
}

/// Paint the entry from an answer the worker published.
///
/// `address` guards against a stale frame: answers are written to one
/// entity, so a reply about what was typed two characters ago would
/// otherwise relabel the button under someone mid-edit.
fn apply_entry_status(host: &HtmlElement, address: &str, state: &str) {
    let typed = input(host, "#account-entry-email").unwrap_or_default();
    if typed.trim().to_lowercase() != address.trim().to_lowercase() {
        return;
    }
    // `invalid` and `unavailable` are not facts about the address: one
    // is a string that cannot be looked up, the other a service that did
    // not answer. Neither should offer an action, and reading either as
    // "nobody has this" would send someone into a ceremony that fails at
    // the end.
    let route = match state {
        "active" | "pending" => Some(EntryRoute::SignIn),
        "unregistered" => Some(EntryRoute::SignUp),
        _ => None,
    };
    if let Ok(Some(form)) = host.query_selector("#account-entry-form") {
        match route {
            Some(route) => {
                let _ = form.set_attribute("data-route", route.stamp());
                set_text(host, "#account-entry-submit", route.label());
            }
            None => {
                let _ = form.remove_attribute("data-route");
                set_text(host, "#account-entry-submit", "Continue");
            }
        }
    }
    if let Ok(Some(status)) = host.query_selector("#account-entry-status") {
        let message = match state {
            // The account exists and does not work yet, and signing in
            // is still the way to it.
            "pending" => Some(
                "This account is waiting for its email to be confirmed. Sign in to pick up where you left off.",
            ),
            "suspended" => Some("This account is no longer served. Contact support to restore it."),
            "unavailable" => Some("Could not reach the sign-in service. Try again in a moment."),
            _ => None,
        };
        match message {
            Some(message) => {
                status.set_text_content(Some(message));
                let _ = status.remove_attribute("hidden");
            }
            None => {
                let _ = status.set_attribute("hidden", "");
            }
        }
    }
}

fn on_click(host: &HtmlElement, selector: &str, callback: impl Fn(HtmlElement) + 'static) {
    let Ok(Some(element)) = host.query_selector(selector) else {
        return;
    };
    let host = host.clone();
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        callback(host.clone());
    });
    let _ = element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
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
/// `/account` themselves belongs. Arriving through the gate, `/` is the one
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
        if link.text_content().as_deref() == Some("Back to Tonk") {
            link.set_text_content(Some("Back"));
        }
    }
}

fn bind(host: &HtmlElement) {
    prevent_form_navigation(host);
    bind_return_links(host);
    configure_deletion_entry(host);
    bind_entry_lookup(host);
    on_click(host, "#account-entry-submit", |host| {
        clear_error(&host);
        let email = match input(&host, "#account-entry-email") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        // The lookup already ran while they typed; this reads its
        // answer rather than asking again, so the click does not wait on
        // a round trip it usually does not need. An unanswered lookup —
        // still in flight, or failed — sends them to sign-up, which is
        // the path that refuses correctly on its own if the address
        // turns out to be taken.
        match entry_answer(&host) {
            Some(EntryRoute::SignIn) => {
                // The link panel owns the assertion, and clicking its
                // button is how that ceremony starts everywhere else.
                // Calling it here keeps one definition of the flow
                // rather than a second copy that can drift from it.
                set_mode(&host, "link");
                if let Ok(Some(button)) = host.query_selector("#account-link-submit")
                    && let Some(button) = button.dyn_ref::<HtmlElement>()
                {
                    button.click();
                }
            }
            _ => {
                set_mode(&host, "create");
                if let Ok(Some(field)) = host.query_selector("#account-email")
                    && let Some(field) = field.dyn_ref::<web_sys::HtmlInputElement>()
                {
                    field.set_value(&email);
                }
                focus_input(&host, "#account-email");
            }
        }
    });
    on_click(host, "#account-choose-link", |host| {
        clear_error(&host);
        set_mode(&host, "link");
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
            let result = async {
                // One ceremony: the secret is generated, sealed under
                // the new passkey's KEK, published as the custody cell,
                // and the creation request signed. No key material is
                // ever stored — every later custody operation derives
                // its keys inside a fresh assertion.
                let device_did = crate::api::identify()
                    .await
                    .map_err(|error| error.to_string())?
                    .did;
                let created = create_account(CreateAccountInput {
                    email: email.clone(),
                    device_did,
                    device_name,
                    remote: proposed_remote()?,
                    created_on: Some(crate::device_name::current()),
                    service_did: deployment_service_did().await,
                })
                .await
                .map_err(|error| error.to_string())?;
                crate::api::save_root(
                    created.credential_id.clone(),
                    created.delegation_hex.clone(),
                    created.passkey.clone(),
                    created.encryption_key.clone(),
                )
                .await
                .map_err(|error| error.to_string())?;
                let ceremony = CeremonyOutput {
                    root_did: created.root_did,
                    credential_id: created.credential_id,
                    delegation_hex: created.delegation_hex,
                    invocation_hex: created.invocation_hex,
                    deposits_hex: created.deposits_hex,
                    encryption_key: created.encryption_key,
                };
                set_busy(&host, true, "Creating your account…");
                complete_remote(&host, "/accounts", ceremony, true, Some(&email)).await?;
                // Neither of these can land before the emailed link is
                // clicked: the service provisions nothing, and serves
                // nothing, for a customer that has not confirmed its
                // email. Both queue instead, and replay on activation.
                if let Err(error) =
                    crate::api::provision_custody(&created.custody_did, &created.consent_hex).await
                {
                    web_sys::console::warn_1(
                        &format!("custody provisioning deferred: {error}").into(),
                    );
                }
                if let Err(error) = crate::api::queue_custody_publish(
                    &created.custody_did,
                    &created.sealed_hex,
                    &created.publish_invocation_hex,
                )
                .await
                {
                    // The sealed secret is only in this page's memory
                    // until it is recorded, so failing to queue it is
                    // the one loss worth surfacing.
                    return Err(format!("could not record the account secret: {error}"));
                }
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-resend-activation", |host| {
        clear_error(&host);
        set_busy(&host, true, "Sending another activation email…");
        spawn_local(async move {
            // Enrollment is idempotent while Registered: the rows stand
            // and the link is sent again. No ceremony is at hand here,
            // so the deposits are the device-chained fallback.
            let result = crate::api::enroll_customer(None, &[]).await;
            set_busy(&host, false, "");
            match result {
                Ok(_) => set_text(
                    &host,
                    "#account-activation-notice",
                    "Sent. Open the link in your activation email.",
                ),
                Err(error) => show_error(&host, format!("could not resend: {error}")),
            }
        });
    });

    on_click(host, "#account-add-passkey", |host| {
        clear_error(&host);
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let (root_did, delegation_hex) = match crate::api::root_status()
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
                let enrolled = enroll_custody_passkey(EnrollCustodyInput {
                    account_did: root_did,
                    label,
                    endpoint: proposed_remote()?,
                })
                .await
                .map_err(|error| error.to_string())?;
                // Provision before publishing: the new custody DID is
                // nobody's consumer until this deposit lands, and the
                // service serves no unprovisioned subject.
                if let Err(error) =
                    crate::api::provision_custody(&enrolled.custody_did, &enrolled.consent_hex)
                        .await
                {
                    web_sys::console::warn_1(
                        &format!("custody provisioning deferred: {error}").into(),
                    );
                }
                // The cell is recorded on profile main either way; the
                // pre-signed invocation rides along when the ceremony's
                // own publish was refused, and the worker drains it once
                // the provisioning deposit lands.
                if let Err(error) = crate::api::queue_custody_publish(
                    &enrolled.custody_did,
                    &enrolled.sealed_hex,
                    enrolled.publish_invocation_hex.as_deref().unwrap_or(""),
                )
                .await
                {
                    return Err(format!(
                        "could not record the sealed account secret: {error}"
                    ));
                }
                crate::api::save_root(
                    enrolled.credential_id,
                    delegation_hex,
                    Some(tonk_worker_api::PasskeyMetadata {
                        created_at: (js_sys::Date::now() / 1000.0) as u64,
                        created_on: crate::device_name::current(),
                    }),
                    None,
                )
                .await
                .map_err(|error| error.to_string())?;
                Ok::<(), String>(())
            }
            .await;
            set_busy(&host, false, "");
            match result {
                Ok(()) => {
                    if let Ok(Some(button)) = host.query_selector("#account-add-passkey") {
                        let _ = button.set_attribute("hidden", "");
                    }
                    load_summary(host.clone());
                }
                Err(error) => show_error(&host, error),
            }
        });
    });

    on_click(host, "#account-link-submit", |host| {
        clear_error(&host);
        let device_name = crate::device_name::current();
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let device_did = crate::api::identify()
                    .await
                    .map_err(|error| error.to_string())?
                    .did;
                // One assertion derives the custody keypair, one
                // presigned GET fetches the sealed envelope, and the
                // unwrapped secret self-issues this device's delegation.
                // A cell still queued from before activation is the
                // worker's to drain — it holds the ceremony's
                // pre-signed publish and runs it on boot and on
                // activation.
                let ceremony = unlock_with_passkey(UnlockWithPasskeyInput {
                    device_did,
                    device_name,
                    endpoint: proposed_remote()?,
                    service_did: deployment_service_did().await,
                })
                .await
                .map_err(|error| error.to_string())?;
                set_busy(&host, true, "Linking this browser…");
                complete_remote(&host, "/devices/link", ceremony, false, None).await
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
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
                            crate::api::enroll_customer(None, &[])
                                .await
                                .map_err(|error| {
                                    format!(
                                        "register with the sync service before linking: {error}"
                                    )
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
                        "descriptorHex": authorized.descriptor_hex,
                        "credentialId": authorized.root_did,
                        "attachmentId": attachment_id,
                        "serviceUrl": service(&host).await.unwrap_or_default(),
                    })
                    .to_string();
                    let encoded = crate::account::encode_authorization(&payload);
                    let redirect = link_outcome_redirect();
                    post_to_callback(
                        &callback,
                        &[("authorize", &encoded), ("redirect", &redirect)],
                    )
                }
                .await;
                if let Err(error) = result {
                    set_busy(&host, false, "");
                    show_error(&host, error);
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
                let _ = window.location().set_href("/account");
            }
            return;
        };
        let redirect = link_outcome_redirect();
        if let Err(error) = post_to_callback(
            &callback,
            &[("deny", "declined in the browser"), ("redirect", &redirect)],
        ) {
            show_error(&host, error);
        }
    });

    on_click(host, "#account-unlink", |host| {
        clear_error(&host);
        let confirmed = window()
            .map(|window| {
                window
                    .confirm_with_message(
                        "Sign out on this device? Your existing spaces will stay here, but account syncing will stop until you sign in again.",
                    )
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        set_busy(&host, true, "Signing out…");
        spawn_local(async move {
            match crate::api::unlink_account().await {
                Ok(_) => match window().map(|window| window.location().reload()) {
                    Some(Ok(())) => {}
                    _ => {
                        set_busy(&host, false, "");
                        set_mode(&host, "choice");
                    }
                },
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });

    on_click(host, "#account-delete-review", |host| {
        clear_error(&host);
        set_busy(&host, true, "Loading the permanent deletion scope…");
        spawn_local(async move {
            match crate::api::account_deletion_plan().await {
                Ok(plan) => {
                    set_busy(&host, false, "");
                    if let Err(error) = render_deletion_plan(&host, &plan) {
                        show_error(&host, error);
                    }
                }
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });

    on_click(host, "#account-delete-submit", |host| {
        clear_error(&host);
        let plan = Reflect::get(host.as_ref(), &DELETION_PLAN.into())
            .ok()
            .and_then(|value| serde_wasm_bindgen::from_value::<AccountDeletionPlan>(value).ok());
        let Some(plan) = plan else {
            return show_error(&host, "Review the current deletion scope first.");
        };
        let requested = requested_space_deletion();
        let confirmed_email = match input(&host, "#account-delete-email") {
            Ok(email) if email == plan.email => email,
            Ok(_) => {
                return show_error(&host, "The confirmation email does not match this account.");
            }
            Err(error) => return show_error(&host, error),
        };
        let understood = host
            .query_selector("#account-delete-understood")
            .ok()
            .flatten()
            .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
            .is_some_and(|input| input.checked());
        if !understood {
            return show_error(
                &host,
                "Confirm that you understand the permanent consequences.",
            );
        }
        let destructive: Vec<_> = plan
            .spaces
            .iter()
            .filter(|space| {
                space.state != "deleted"
                    && requested
                        .as_deref()
                        .is_none_or(|subject| space.subject == subject)
            })
            .cloned()
            .collect();
        if requested.is_some() && destructive.len() != 1 {
            return show_error(&host, "The selected owned space is already deleted.");
        }
        let final_confirmation = if requested.is_some() {
            format!(
                "Permanently delete {} and its hosted content from Tonk services?\n\nYour account and every other space will remain. Copies already replicated to other devices cannot be erased by Tonk. This cannot be undone.",
                destructive[0]
                    .name
                    .as_deref()
                    .unwrap_or(&destructive[0].subject),
            )
        } else {
            format!(
                "Permanently delete {} owned space{}, their hosted content from Tonk services, all account backups, and the account for {}?\n\nJoined spaces will not be deleted. Copies already replicated to other devices cannot be erased by Tonk. This cannot be undone.",
                destructive.len(),
                if destructive.len() == 1 { "" } else { "s" },
                plan.email,
            )
        };
        if !window()
            .and_then(|window| window.confirm_with_message(&final_confirmation).ok())
            .unwrap_or(false)
        {
            return;
        }
        set_busy(&host, true, "Waiting for your passkey…");
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
                    .map_err(|error| error.to_string())?;
                    return Ok::<_, String>((Some(deleted.subject), None));
                }
                // Account deletion asks the human to verify with the
                // account's passkey, then the worker signs every
                // destructive invocation with this device's delegated
                // authority.
                let credential_id = match crate::api::root_status()
                    .await
                    .map_err(|error| error.to_string())?
                {
                    tonk_worker_api::RootStatus::Ready {
                        root_did,
                        credential_id,
                        ..
                    } => {
                        if root_did != plan.root_did {
                            return Err(
                                "this device's passkey belongs to a different account".into()
                            );
                        }
                        credential_id
                    }
                    tonk_worker_api::RootStatus::Missing { .. } => {
                        return Err(
                            "no account passkey is registered on this device to verify with".into(),
                        );
                    }
                };
                verify_passkey(VerifyPasskeyInput { credential_id })
                    .await
                    .map_err(|error| error.to_string())?;
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
                .map_err(|error| error.to_string())?;
                Ok((None, Some(deleted)))
            }
            .await;
            match result {
                Ok((Some(subject), None)) => {
                    let _ = window().map(|window| {
                        window.alert_with_message(&format!(
                            "Owned space {subject} deleted from Tonk services. Your account and other spaces remain. Tonk cannot erase copies already replicated to other devices."
                        ))
                    });
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/account");
                    }
                }
                Ok((None, Some(result))) => {
                    let _ = window().map(|window| {
                        window.alert_with_message(&format!(
                            "Account deleted. {} owned space{} removed from Tonk services; {} joined space{} left intact.",
                            result.deleted_spaces,
                            if result.deleted_spaces == 1 { "" } else { "s" },
                            result.retained_joined_spaces,
                            if result.retained_joined_spaces == 1 { "" } else { "s" },
                        ))
                    });
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/account");
                    }
                }
                Ok(_) => {
                    set_busy(&host, false, "");
                    show_error(&host, "the deletion result was incomplete");
                }
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error);
                }
            }
        });
    });

    // "Add account" (dashboard) and "Use a different account" (Choice)
    // are the same move: rotate onto a fresh profile and land on the
    // normal sign-in flow there, leaving this profile intact.
    for selector in ["#account-add-profile", "#account-use-different-account"] {
        on_click(host, selector, |host| {
            clear_error(&host);
            set_busy(&host, true, "Preparing another sign-in…");
            spawn_local(async move {
                match crate::api::add_account_profile().await {
                    Ok(_) => reload_into_switched_profile(&host),
                    Err(error) => {
                        set_busy(&host, false, "");
                        show_error(&host, error.to_string());
                    }
                }
            });
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
                match crate::api::activate_profile(profile).await {
                    Ok(_) => reload_into_switched_profile(&host),
                    Err(error) => {
                        set_busy(&host, false, "");
                        show_error(&host, error.to_string());
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

/// Confirm and revoke.
///
/// No passkey ceremony: the worker's own account grant is a powerline,
/// so it mints the revocation itself — for this device from its own
/// link, for another from the target's grant retained in the account
/// space. Shared by the device list's button and the CLI's `?revoke=`
/// handoff.
fn begin_revoke(host: HtmlElement, did: String, self_revoke: bool) {
    let message = if self_revoke {
        "Remove access for this device? This permanently disconnects it from your Tonk account. To use it again, sign in to add it as a new device."
    } else {
        "Remove access for this device? This permanently disconnects it from your Tonk account. To use it again, sign in to add a new device."
    };
    let confirmed = window()
        .map(|window| window.confirm_with_message(message).unwrap_or(false))
        .unwrap_or(false);
    if !confirmed {
        return;
    }
    clear_error(&host);
    set_busy(
        &host,
        true,
        if self_revoke {
            "Revoking this device…"
        } else {
            "Revoking device…"
        },
    );
    spawn_local(async move {
        match crate::api::revoke_account_device(did).await {
            Ok(acknowledgement) => {
                clear_error(&host);
                set_busy(
                    &host,
                    false,
                    revocation_status(&acknowledgement, self_revoke),
                );
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
                show_error(&host, error.to_string());
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
        install_subscription_shim();
    }
}

/// Forward host subscription frames to the per-instance delegates.
///
/// The host calls `element.reset(payload, opts)` as a method, so the
/// forwarder lives on the prototype where `this` binds correctly, and
/// the instance sets `__tonkReset` when it subscribes. Same pattern
/// `<tonk-display>` and `<ui-sync-status>` use.
fn install_subscription_shim() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-account");
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    // Two subscriptions share this element -- the registration row and
    // the entry's address lookup -- so one method fans out to both and
    // each delegate ignores what is not its own (see `frame_tag`).
    for (method, delegates) in [
        ("reset", ["__tonkRegistrationReset", "__tonkEntryReset"]),
        ("update", ["__tonkRegistrationUpdate", "__tonkEntryUpdate"]),
        ("error", ["__tonkRegistrationError", "__tonkEntryError"]),
    ] {
        let body = delegates
            .iter()
            .map(|delegate| {
                format!(
                    "if (typeof this.{delegate} === 'function') this.{delegate}(payload, opts);"
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let forwarder = js_sys::Function::new_with_args("payload, opts", &body);
        let _ = Reflect::set(&proto, &method.into(), &forwarder);
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
            .query_selector(".account__masthead [data-return]")
            .unwrap()
            .expect("the success panel offers a way back");
        assert_eq!(
            back.get_attribute("href").as_deref(),
            Some("/space/did:key:zBack")
        );
        assert_eq!(
            back.text_content().as_deref(),
            Some("Back"),
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
            host.query_selector(".account__masthead [data-return]")
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
        // The entry is one field, not a fork, so this button is no
        // longer half of a choice: it is the way past typing an address
        // at all, for someone whose authenticator already knows them.
        assert_eq!(
            host.query_selector("#account-choose-link")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Sign in with a passkey")
        );
        assert!(
            host.query_selector("#account-entry-email")
                .unwrap()
                .is_some(),
            "the address is asked for before either ceremony"
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
    fn it_selects_the_new_ceremony_when_the_root_differs() {
        let status = tonk_worker_api::RootStatus::Ready {
            root_did: "did:key:zOldRoot".into(),
            device_did: "did:key:zDevice".into(),
            credential_id: "old-credential".into(),
            delegation_cid: "bafyold".into(),
            delegation_hex: "00".into(),
            passkey: None,
            encryption_key: None,
        };
        let ceremony = CeremonyOutput {
            root_did: "did:key:zNewRoot".into(),
            credential_id: "new-credential".into(),
            delegation_hex: "11".into(),
            invocation_hex: "22".into(),
            deposits_hex: Vec::new(),
            encryption_key: None,
        };

        assert_eq!(
            root_for_link(&status, &ceremony),
            LinkRoot {
                root_did: "did:key:zNewRoot",
                credential_id: "new-credential",
                delegation_hex: "11",
                needs_persist: true,
            }
        );
    }

    #[dialog_common::test]
    fn it_reuses_the_stored_grant_when_the_same_root_links_again() {
        let status = tonk_worker_api::RootStatus::Ready {
            root_did: "did:key:zRoot".into(),
            device_did: "did:key:zDevice".into(),
            credential_id: "stored-credential".into(),
            delegation_cid: "bafystored".into(),
            delegation_hex: "00".into(),
            passkey: None,
            encryption_key: None,
        };
        let ceremony = CeremonyOutput {
            root_did: "did:key:zRoot".into(),
            credential_id: "ceremony-credential".into(),
            delegation_hex: "11".into(),
            invocation_hex: "22".into(),
            deposits_hex: Vec::new(),
            encryption_key: None,
        };

        assert_eq!(
            root_for_link(&status, &ceremony),
            LinkRoot {
                root_did: "did:key:zRoot",
                credential_id: "stored-credential",
                delegation_hex: "00",
                needs_persist: false,
            }
        );
    }

    #[dialog_common::test]
    fn it_disables_in_panel_navigation_while_account_work_is_in_flight() {
        let host = host();
        set_busy(&host, true, "Creating your account…");

        for selector in [
            "#account-entry-submit",
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
    }

    #[dialog_common::test]
    fn it_authors_a_single_signed_in_dashboard() {
        let host = host();
        assert_eq!(
            host.query_selector(".account > h1")
                .unwrap()
                .expect("account heading")
                .text_content()
                .as_deref(),
            Some("Account")
        );

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
            ".account__passkey",
            ".account__signout",
            "#account-delete-review",
            "#account-delete-submit",
            "#account-delete-email",
            "#account-delete-understood",
        ] {
            assert!(
                dashboard.query_selector(selector).unwrap().is_some(),
                "the signed-in dashboard is missing {selector}"
            );
        }
        assert!(
            host.query_selector(".account__masthead [data-return]")
                .unwrap()
                .is_some(),
            "the account masthead should offer a conventional return link"
        );

        let copy = dashboard.text_content().unwrap();
        assert!(copy.contains("device, browser profile, or password manager"));
        assert!(copy.contains("do not tell Tonk which passkey manager currently stores it"));
        assert!(copy.contains("syncing will stop until you sign in again"));
        assert!(copy.contains("This is not sign out"));
        assert!(copy.contains("Spaces created by other people will not be deleted"));
        assert!(copy.contains("cannot erase copies that other devices have already replicated"));
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
            host.query_selector("#account-devices").unwrap().is_none(),
            "the signed-in dashboard should be the only device-management surface"
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
            text.contains("Local workspace"),
            "a never-signed-in row says what it is"
        );
        assert!(text.contains("Alice") && text.contains("brave-otter"));

        let button = list
            .query_selector("button[data-activate=\"tonk-0a12\"]")
            .unwrap()
            .expect("the other profile's row offers a switch");
        assert_eq!(button.text_content().as_deref(), Some("Switch"));
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
        assert!(active.text_content().unwrap().contains("Current"));
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

    #[dialog_common::test]
    fn it_renders_recorded_and_legacy_passkey_facts_without_guessing() {
        let host = host();
        render_summary(
            &host,
            &tonk_worker_api::AccountSummary {
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
    /// action, which reloaded `/account?Email=…`, threw the typed address
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
        assert!(text.contains("This device"));
        assert!(text.contains("Added"));
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
        assert_eq!(button.text_content().as_deref(), Some("Remove access"));
    }

    /// A `?revoke=` deep link must land on the device list, where the
    /// ceremony runs — parking a linked browser on the success screen
    /// leaves the CLI polling until it times out.
    #[dialog_common::test]
    fn it_routes_a_revoke_deep_link_to_the_device_list() {
        assert_eq!(
            landing(Some(AccountStateStatus::Unconfigured), true),
            Landing::Devices
        );
        assert_eq!(
            landing(Some(AccountStateStatus::Ready), false),
            Landing::Success
        );
        assert_eq!(landing(None, true), Landing::Choice { revoke_hint: true });
        assert_eq!(landing(None, false), Landing::Choice { revoke_hint: false });
    }
}
