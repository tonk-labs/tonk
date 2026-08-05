//! Top-document account creation and passkey self-link surface.

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, window};

use tonk_account::{AccountStateStatus, handoff::ResolvedLink};
use tonk_worker_api::{AccountStatus, RevocationProjection, RevokeDeviceAcknowledgement};

use crate::identity_bridge::{
    CeremonyOutput, CreateAccountInput, CreateRootInput, EstablishRepositoryInput, LinkDeviceInput,
    RevocationOutput, SignRevocationInput, complete_link, create_account, create_root,
    establish_account_repository, link_device, sign_revocation,
};

const STYLE_ID: &str = "tonk-account-styles";
const HANDOFF: &str = "__tonkCliHandoff";

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
    } else {
        Ok(value)
    }
}

fn set_mode(host: &HtmlElement, mode: &str) {
    let _ = host.set_attribute("data-mode", mode);
    for (name, selector) in [
        ("choice", "#account-choice"),
        ("create", "#account-create"),
        ("verify", "#account-verify"),
        ("link", "#account-link"),
        ("handoff", "#account-handoff"),
        ("setup", "#account-setup"),
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
        "#account-send-code",
        "#account-create-submit",
        "#account-link-submit",
        "#account-handoff-submit",
        "#account-setup-submit",
        "#account-unlink",
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
}

fn set_text(host: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = host.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

fn render_summary(host: &HtmlElement, summary: &tonk_worker_api::AccountSummary) {
    set_text(host, "#account-email-value", &summary.email);
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
/// The gate parks the operation it refused and sends the user here; this is
/// the other half. [`crate::account_gate::finish`] replays that operation —
/// which navigates into the space it created or joined — or, with nothing
/// parked, returns to the `next` this page was opened with. Only when neither
/// applies does the success panel show, which is the case where the user came
/// to `/account` on their own.
///
/// A replay failure is shown rather than swallowed. The account is real, so
/// the panel says so; the sentence underneath says the operation is not done.
fn settle(host: &HtmlElement) {
    settle_with(host, crate::account_gate::finish());
}

/// Show the account dashboard to a device that already had an account.
///
/// Same shape as [`settle`], minus the return-to-`next` step. A gated user who
/// signed in on another tab still gets their interrupted operation replayed;
/// someone who opened their account settings from a spot — the FAB's account
/// link carries `next` so its Back goes home — stays on the page they asked
/// for instead of being bounced straight back out of it.
fn settle_on_load(host: &HtmlElement) {
    settle_with(host, crate::account_gate::resume_pending());
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

fn show_handoff_success(host: &HtmlElement) {
    if let Ok(Some(message)) = host.query_selector("#account-success-message") {
        message.set_text_content(Some("The command-line profile is connected."));
    }
    show_success(host);
}

fn render_devices(host: &HtmlElement, devices: &[tonk_worker_api::AccountDevice]) {
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

        if device.this_device {
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
        let mut details = if device.status == "revoked" {
            format!("Access removed · Added {}", String::from(date))
        } else {
            format!("Added {}", String::from(date))
        };
        if !device.this_device && device.status == "active" && device.delegation_hex.is_none() {
            details.push_str(" · Sign in again on this device to enable removal");
        }
        meta.set_text_content(Some(&details));

        let _ = item.append_child(&identity);
        let _ = item.append_child(&meta);

        if device.status == "active" && (device.this_device || device.delegation_hex.is_some()) {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("class", "account__button account__button--remove");
            let _ = button.set_attribute("data-revoke", &device.did);
            let _ = button.set_attribute("data-attachment-id", &device.attachment_id);
            let _ = button.set_attribute("data-delegation-cid", &device.delegation_cid);
            let _ =
                button.set_attribute("aria-label", &format!("Remove access for {}", device.name));
            if let Some(delegation_hex) = &device.delegation_hex {
                let _ = button.set_attribute("data-delegation-hex", delegation_hex);
            }
            if device.this_device {
                let _ = button.set_attribute("data-self-revoke", "true");
            }
            button.set_text_content(Some("Remove access"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

fn revocation_status(
    acknowledgement: &RevokeDeviceAcknowledgement,
    self_revoke: bool,
) -> &'static str {
    if self_revoke {
        "Access removed from this device."
    } else if acknowledgement.projection == RevocationProjection::Stale {
        "Access removed. The device list may take a moment to update."
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

fn revoke_attachment_from_url() -> Option<String> {
    query_value("attachment")
}

/// Strip the query once the deep link has been acted on, mirroring what
/// [`load_handoff`] does with the fragment secret. Without this a
/// cancelled confirm re-fires on every later dashboard visit
/// in this tab.
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
        match crate::api::account_devices().await {
            Ok(devices) => {
                set_busy(&host, false, "");
                render_devices(&host, &devices);
                if let Some(did) = revoke_target_from_url() {
                    let attachment_id = revoke_attachment_from_url();
                    consume_revoke_target();
                    match devices.iter().find(|device| {
                        device.did == did
                            && device.status == "active"
                            && attachment_id
                                .as_deref()
                                .is_none_or(|expected| device.attachment_id == expected)
                    }) {
                        Some(device) if device.this_device || device.delegation_hex.is_some() => {
                            begin_revoke(
                                host.clone(),
                                device.attachment_id.clone(),
                                device.did.clone(),
                                device.delegation_cid.clone(),
                                device.delegation_hex.clone().unwrap_or_default(),
                                device.this_device,
                            )
                        }
                        Some(_) => show_error(
                            &host,
                            "This device was added before remote removal was supported. \
                             Sign in again on that device, then try removing it here.",
                        ),
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
    /// Explicit one-time descriptor ceremony for a legacy raw link.
    Setup,
    /// The signed-in dashboard.
    Success,
    /// The link/create choice, with a hint when a revoke deep link
    /// cannot proceed because this browser is not linked.
    Choice { revoke_hint: bool },
}

fn landing(account_state: Option<AccountStateStatus>, revoke_target: bool) -> Landing {
    match (account_state, revoke_target) {
        (Some(_), true) => Landing::Devices,
        (Some(AccountStateStatus::Unconfigured), false) => Landing::Setup,
        (Some(_), false) => Landing::Success,
        (None, revoke_hint) => Landing::Choice { revoke_hint },
    }
}

fn load_status(host: HtmlElement) {
    let handoff_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/account/link" || path.starts_with("/account/link/"));
    if handoff_route {
        load_handoff(host);
        return;
    }
    // The gate always arrives with a `next`. Without one the user came here
    // themselves, so anything parked belongs to an attempt they walked away
    // from — replaying it on this sign-in would create a spot nobody asked
    // for. Drop it before any ceremony can pick it up.
    if crate::account_gate::requested_next().is_none() {
        crate::account_gate::discard_pending();
    }
    set_busy(&host, true, "Checking this browser…");
    spawn_local(async move {
        if let Err(error) = service(&host).await {
            set_busy(&host, false, "");
            set_mode(&host, "blocked");
            show_error(&host, error);
            return;
        }
        match crate::api::account_status().await {
            Ok(status) => {
                let account_state = match status {
                    AccountStatus::Registered { account_state, .. } => Some(account_state),
                    AccountStatus::RootMissing { .. } | AccountStatus::Unregistered { .. } => None,
                };
                match landing(account_state, revoke_target_from_url().is_some()) {
                    Landing::Devices => load_devices(host),
                    Landing::Setup => {
                        set_busy(&host, false, "");
                        set_mode(&host, "setup");
                    }
                    Landing::Success => {
                        settle_on_load(&host);
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

fn load_handoff(host: HtmlElement) {
    let Some(window) = window() else {
        return show_error(&host, "window is unavailable");
    };
    let secret = window
        .location()
        .hash()
        .ok()
        .and_then(|hash| hash.strip_prefix('#').map(str::to_owned))
        .filter(|secret| !secret.is_empty());
    let Some(secret) = secret else {
        set_mode(&host, "handoff");
        return show_error(
            &host,
            "This link is missing its handoff secret. Start again from the terminal.",
        );
    };
    if let Ok(path) = window.location().pathname() {
        let _ = window
            .history()
            .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&path)));
    }

    set_busy(&host, true, "Checking the command-line request…");
    spawn_local(async move {
        let service_url = match service(&host).await {
            Ok(service_url) => service_url,
            Err(error) => {
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
                show_error(&host, error);
                return;
            }
        };
        match crate::api::resolve_account_link(&service_url, &secret).await {
            Ok(handoff) => {
                if let Ok(value) = serde_wasm_bindgen::to_value(&handoff) {
                    let _ = Reflect::set(host.as_ref(), &HANDOFF.into(), &value);
                }
                if let Ok(Some(name)) = host.query_selector("#account-handoff-name") {
                    name.set_text_content(Some(&handoff.device_name));
                }
                if let Ok(Some(did)) = host.query_selector("#account-handoff-did") {
                    did.set_text_content(Some(&handoff.device_did));
                }
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
            }
            Err(error) => {
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
                show_error(&host, error.to_string());
            }
        }
    });
}

async fn persist(
    provider: &str,
    ceremony: &CeremonyOutput,
    descriptor_hex: String,
    initialize_name: bool,
) -> Result<AccountStatus, String> {
    match crate::api::root_status()
        .await
        .map_err(|error| error.to_string())?
    {
        tonk_worker_api::RootStatus::Missing { .. } => {
            crate::api::save_root(
                ceremony.credential_id.clone(),
                ceremony.delegation_hex.clone(),
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        tonk_worker_api::RootStatus::Ready { .. } => {}
    }
    crate::api::save_account_link(
        provider.to_string(),
        ceremony.root_did.clone(),
        ceremony.credential_id.clone(),
        ceremony.delegation_hex.clone(),
        descriptor_hex,
        initialize_name,
    )
    .await
    .map_err(|error| error.to_string())
}

/// What to tell someone whose account predates the repository descriptor.
///
/// They cannot establish one from here: the setup panel runs against an
/// existing local link, and this browser has none. An already-linked device can
/// do it, and afterwards this one signs in normally.
const UNESTABLISHED_ACCOUNT_GUIDANCE: &str = "This account was created before shared account state existed, so it can't be added to a new \
     browser yet. Open /account on a browser that is already signed in to this account and \
     finish account setup there, then sign in here.";

/// The account repository remote this browser proposes: its own origin's
/// `/ucan/` endpoint. Only a ceremony ever signs one; the stored descriptor is
/// always the service-selected winner.
fn proposed_remote() -> Result<String, String> {
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
    settle(host);
    if initialize_name && is_unhydrated(&status) {
        show_error(
            host,
            "Your account was created, but its initial name could not be synchronized. Reload /account to retry account hydration.",
        );
    }
    Ok(())
}

async fn establish_repository(host: &HtmlElement) -> Result<(), String> {
    let ceremony = establish_account_repository(EstablishRepositoryInput {
        remote: proposed_remote()?,
    })
    .await
    .map_err(|error| error.to_string())?;
    let response = crate::api::submit_account_ceremony(
        &service(host).await?,
        "/account/repository/establish",
        &ceremony.invocation_hex,
    )
    .await
    .map_err(|error| error.to_string())?;
    let descriptor_hex = descriptor_hex(&response)?;
    let created = response
        .get("created")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "account service omitted created".to_string())?;

    // Persist only the service-selected winner. A losing ceremony candidate is
    // never written locally, and only the one `created: true` response may ask
    // the worker to seed this device's current profile name.
    let status = crate::api::establish_local_account_repository(descriptor_hex, created)
        .await
        .map_err(|error| error.to_string())?;
    settle(host);
    if is_unhydrated(&status) {
        show_error(
            host,
            "Account setup is saved, but account state is not synchronized yet. Reload /account to retry; do not choose another remote.",
        );
    }
    Ok(())
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
/// place the user was NOT — leaving means abandoning the spot they were
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
        // "Back to Tonk" is the truth for `/`, and a lie for a spot. The
        // destination changed, so the label has to.
        if link.text_content().as_deref() == Some("Back to Tonk") {
            link.set_text_content(Some("Back"));
        }
    }
}

fn bind(host: &HtmlElement) {
    prevent_form_navigation(host);
    bind_return_links(host);
    on_click(host, "#account-choose-create", |host| {
        clear_error(&host);
        set_mode(&host, "create");
        focus_input(&host, "#account-email");
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
    on_click(host, "#account-verify-back", |host| {
        clear_error(&host);
        set_busy(&host, false, "");
        set_mode(&host, "create");
        focus_input(&host, "#account-email");
    });

    on_click(host, "#account-send-code", |host| {
        clear_error(&host);
        let email = match input(&host, "#account-email") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        set_busy(&host, true, "Sending verification code…");
        spawn_local(async move {
            let service_url = match service(&host).await {
                Ok(service_url) => service_url,
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error);
                    return;
                }
            };
            match crate::api::request_account_code(&service_url, &email).await {
                Ok(()) => {
                    set_busy(&host, false, "");
                    if let Ok(Some(destination)) = host.query_selector("#account-code-email") {
                        destination.set_text_content(Some(&email));
                    }
                    set_mode(&host, "verify");
                    if let Ok(Some(code)) = host.query_selector("#account-code")
                        && let Ok(code) = code.dyn_into::<HtmlInputElement>()
                    {
                        code.set_value("");
                        let _ = code.focus();
                    }
                }
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });

    on_click(host, "#account-create-submit", |host| {
        clear_error(&host);
        let fields = (
            input(&host, "#account-email"),
            input(&host, "#account-code"),
        );
        let (email, code) = match fields {
            (Ok(email), Ok(code)) => (email, code),
            (Err(error), _) | (_, Err(error)) => return show_error(&host, error),
        };
        let device_name = crate::device_name::current();
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let status = crate::api::root_status()
                    .await
                    .map_err(|error| error.to_string())?;
                let (root_did, device_did, credential_id, delegation_hex, passkey) = match status {
                    tonk_worker_api::RootStatus::Ready {
                        root_did,
                        device_did,
                        credential_id,
                        delegation_hex,
                        passkey,
                        ..
                    } => (root_did, device_did, credential_id, delegation_hex, passkey),
                    tonk_worker_api::RootStatus::Missing { device_did } => {
                        // This ceremony is what creates the passkey, and it
                        // knows the address the code just verified — so the
                        // credential gets a name its owner will recognise in a
                        // passkey manager instead of an opaque handle. Only
                        // here: a root created by anything else has no account
                        // to name.
                        let created = create_root(CreateRootInput {
                            device_did,
                            label: Some(email.clone()),
                            created_on: Some(device_name.clone()),
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                        crate::api::save_root(
                            created.credential_id.clone(),
                            created.delegation_hex.clone(),
                            created.passkey.clone(),
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                        (
                            created.root_did,
                            created.device_did,
                            created.credential_id,
                            created.delegation_hex,
                            created.passkey,
                        )
                    }
                };
                let ceremony = create_account(CreateAccountInput {
                    email,
                    code,
                    device_did,
                    device_name,
                    root_did,
                    credential_id,
                    delegation_hex,
                    passkey,
                    remote: proposed_remote()?,
                })
                .await
                .map_err(|error| error.to_string())?;
                set_busy(&host, true, "Creating your account…");
                complete_remote(&host, "/accounts", ceremony, true).await
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-setup-submit", |host| {
        clear_error(&host);
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            if let Err(error) = establish_repository(&host).await {
                set_busy(&host, false, "");
                set_mode(&host, "setup");
                show_error(&host, error);
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
                let ceremony = link_device(LinkDeviceInput {
                    device_did,
                    device_name,
                })
                .await
                .map_err(|error| error.to_string())?;
                set_busy(&host, true, "Linking this browser…");
                complete_remote(&host, "/devices/link", ceremony, false).await
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
        let handoff = Reflect::get(host.as_ref(), &HANDOFF.into())
            .ok()
            .and_then(|value| serde_wasm_bindgen::from_value::<ResolvedLink>(value).ok());
        let Some(handoff) = handoff else {
            return show_error(
                &host,
                "This handoff is no longer available. Start again from the terminal.",
            );
        };
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let ceremony = complete_link(handoff)
                    .await
                    .map_err(|error| error.to_string())?;
                set_busy(&host, true, "Linking the command-line profile…");
                let _ = crate::api::submit_account_ceremony(
                    &service(&host).await?,
                    "/links/complete",
                    &ceremony.invocation_hex,
                )
                .await
                .map_err(|error| error.to_string())?;
                show_handoff_success(&host);
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-unlink", |host| {
        clear_error(&host);
        let confirmed = window()
            .map(|window| {
                window
                    .confirm_with_message(
                        "Sign out on this device? Your existing spots will stay here, but account syncing will stop until you sign in again.",
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
            let attachment_id = target
                .get_attribute("data-attachment-id")
                .unwrap_or_default();
            let delegation_cid = target
                .get_attribute("data-delegation-cid")
                .unwrap_or_default();
            let delegation_hex = target
                .get_attribute("data-delegation-hex")
                .unwrap_or_default();
            let self_revoke = target.get_attribute("data-self-revoke").is_some();
            begin_revoke(
                host_for_revoke.clone(),
                attachment_id,
                did,
                delegation_cid,
                delegation_hex,
                self_revoke,
            );
        });
        let _ = list.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }
}

/// Confirm, run the passkey ceremony, and revoke.
///
/// Only the account root can revoke another device, and the root lives
/// behind the passkey — so the ceremony runs here, in the page, and the
/// signed revocation travels with the request. Shared by the device
/// list's button and the CLI's `?revoke=` handoff.
fn begin_revoke(
    host: HtmlElement,
    attachment_id: String,
    did: String,
    delegation_cid: String,
    delegation_hex: String,
    self_revoke: bool,
) {
    let message = if self_revoke {
        "Remove access for this device? This permanently disconnects it from your Tonk account. To use it again, sign in to add it as a new device."
    } else {
        "Remove access for this device? This permanently disconnects it from your Tonk account. To use it again, sign in to add a new device.\n\nYou will be asked to confirm with your passkey."
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
            "Waiting for your passkey…"
        },
    );
    spawn_local(async move {
        let revocation_hex = if self_revoke {
            String::new()
        } else {
            let signed: Result<RevocationOutput, String> = sign_revocation(SignRevocationInput {
                delegation_cid,
                path_hex: delegation_hex,
            })
            .await
            .map_err(|error| error.to_string());
            match signed {
                Ok(output) => output.revocation_hex,
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error);
                    return;
                }
            }
        };
        set_busy(&host, true, "Revoking device…");
        match crate::api::revoke_account_device(attachment_id, did, revocation_hex).await {
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
                // mutable registry is deliberately best-effort and cannot
                // turn that success into a failure.
                match crate::api::account_devices().await {
                    Ok(devices) => render_devices(&host, &devices),
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

    #[dialog_common::test]
    fn it_authors_the_create_and_self_link_controls() {
        let host = host();
        for selector in [
            "#account-send-code",
            "#account-create-submit",
            "#account-link-submit",
            "#account-handoff-submit",
            "#account-setup-submit",
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
            Some("Log in")
        );
        assert!(
            host.query_selector("#account-retry-local")
                .unwrap()
                .is_none(),
            "local persistence recovery must not be exposed in the account UI"
        );
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
            ".account__passkey",
            ".account__signout",
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

    #[dialog_common::test]
    fn it_renders_recorded_and_legacy_passkey_facts_without_guessing() {
        let host = host();
        render_summary(
            &host,
            &tonk_worker_api::AccountSummary {
                email: "person@example.com".into(),
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
                email: "legacy@example.com".into(),
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
    fn it_distinguishes_publication_from_projection_and_self_revocation() {
        let stale = RevokeDeviceAcknowledgement {
            target_did: "did:key:device".into(),
            target_cid: "bafycid".into(),
            published: true,
            projection: RevocationProjection::Stale,
        };
        assert_eq!(
            revocation_status(&stale, false),
            "Access removed. The device list may take a moment to update."
        );
        assert_eq!(
            revocation_status(&stale, true),
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

    /// Enter has to do what Continue does, not nothing. Implicit submission
    /// clicks the form's submit button, and that click is what carries the
    /// send-code handler — so the button has to be the form's submit button
    /// rather than an inert `type="button"` beside it.
    #[dialog_common::test]
    fn it_lets_enter_send_the_verification_code() {
        let host = host();
        let button = host
            .query_selector("#account-send-code")
            .expect("query")
            .expect("continue button");
        assert_eq!(
            button.get_attribute("type").as_deref(),
            Some("submit"),
            "Continue must be the email form's submit button",
        );
    }

    #[dialog_common::test]
    fn it_switches_between_account_panels_without_reauthoring_the_dom() {
        let host = host();
        set_mode(&host, "verify");
        assert!(
            host.query_selector("#account-verify")
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
        assert!(
            host.query_selector("#account-create #account-code")
                .unwrap()
                .is_none(),
            "email and verification fields should be on separate screens"
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
        // A legacy persisted label must still render verbatim; this change only
        // affects names generated for new registrations.
        let devices = vec![
            tonk_worker_api::AccountDevice {
                attachment_id: "attachment-this".into(),
                did: "did:key:zThis".into(),
                delegation_cid: "bafythis".into(),
                delegation_hex: Some("beef".into()),
                name: "This browser".into(),
                status: "active".into(),
                created_at: 1_753_300_000,
                this_device: true,
            },
            tonk_worker_api::AccountDevice {
                attachment_id: "attachment-other".into(),
                did: "did:key:zOther".into(),
                delegation_cid: "bafyother".into(),
                delegation_hex: Some("beef".into()),
                name: "Old laptop".into(),
                status: "revoked".into(),
                created_at: 1_753_200_000,
                this_device: false,
            },
            tonk_worker_api::AccountDevice {
                attachment_id: "attachment-phone".into(),
                did: "did:key:zPhone".into(),
                delegation_cid: "bafyphone".into(),
                delegation_hex: Some("beef".into()),
                name: "Phone".into(),
                status: "active".into(),
                created_at: 1_753_100_000,
                this_device: false,
            },
            tonk_worker_api::AccountDevice {
                attachment_id: "attachment-legacy".into(),
                did: "did:key:zLegacy".into(),
                delegation_cid: "bafylegacy".into(),
                delegation_hex: None,
                name: "Legacy tablet".into(),
                status: "active".into(),
                created_at: 1_753_000_000,
                this_device: false,
            },
        ];
        render_devices(&host, &devices);

        let list = host
            .query_selector("#account-device-list")
            .unwrap()
            .unwrap();
        let items = list.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 4);
        let text = list.text_content().unwrap();
        assert!(text.contains("This browser"));
        assert!(text.contains("This device"));
        assert!(text.contains("Access removed"));
        assert!(text.contains("Added"));
        assert!(text.contains("Sign in again on this device to enable removal"));
        // Self-revocation is device-signed and the current row does not need
        // provider path bytes. Another device needs retained path evidence;
        // the legacy row remains visible but has no unsafe revoke action.
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

        // Another-device ceremony signs a revocation of a named delegation,
        // so its button carries the CID as well as the DID.
        let button = list
            .query_selector("button[data-revoke=\"did:key:zPhone\"]")
            .unwrap()
            .expect("the active, non-self row has a revoke button");
        assert_eq!(button.text_content().as_deref(), Some("Remove access"));
        assert_eq!(
            button.get_attribute("data-delegation-cid").as_deref(),
            Some("bafyphone")
        );
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
            landing(Some(AccountStateStatus::Unconfigured), false),
            Landing::Setup
        );
        assert_eq!(
            landing(Some(AccountStateStatus::Ready), false),
            Landing::Success
        );
        assert_eq!(landing(None, true), Landing::Choice { revoke_hint: true });
        assert_eq!(landing(None, false), Landing::Choice { revoke_hint: false });
    }
}
