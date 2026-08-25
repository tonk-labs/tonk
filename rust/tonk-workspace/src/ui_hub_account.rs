//! `<ui-hub-account>` — the Hub account switcher and settings surface.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Date;
use tonk_worker_api::{AccountDevice, AccountSummary, ProfileRosterEntry, ProfilesResponse};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, Node, UrlSearchParams, window,
};

type EventClosure = Closure<dyn FnMut(Event)>;
type KeyClosure = Closure<dyn FnMut(KeyboardEvent)>;

fn set_text(this: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = this.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

fn set_hidden(this: &HtmlElement, selector: &str, hidden: bool) {
    if let Ok(Some(element)) = this.query_selector(selector)
        && let Ok(element) = element.dyn_into::<HtmlElement>()
    {
        element.set_hidden(hidden);
    }
}

fn format_date(seconds: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = Date::new(&JsValue::from_f64(seconds as f64 * 1_000.0));
    let month = MONTHS
        .get(date.get_utc_month() as usize)
        .copied()
        .unwrap_or("");
    format!(
        "{} {month} {}",
        date.get_utc_date(),
        date.get_utc_full_year()
    )
}

fn render_account_summary(this: &HtmlElement, summary: &AccountSummary) {
    set_text(
        this,
        "[data-account-email]",
        summary.email.as_deref().unwrap_or("Unavailable"),
    );
    if let Some(passkey) = &summary.passkey {
        set_text(
            this,
            "[data-passkey-created]",
            &format_date(passkey.created_at),
        );
        set_text(this, "[data-passkey-created-on]", &passkey.created_on);
    } else {
        set_text(this, "[data-passkey-created]", "Not recorded");
        set_text(this, "[data-passkey-created-on]", "Not recorded");
    }
    set_hidden(this, "[data-account-summary-error]", true);
}

fn revoke_path(device: &AccountDevice) -> String {
    let params = UrlSearchParams::new().expect("URLSearchParams is available in browsers");
    params.append("revoke", &device.did);
    params.append("attachment", &device.attachment_id);
    format!(
        "/account?{}",
        params.to_string().as_string().unwrap_or_default()
    )
}

fn render_devices(this: &HtmlElement, devices: &[AccountDevice]) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Some(list) = this.query_selector("[data-device-list]").ok().flatten() else {
        return;
    };
    list.set_inner_html("");
    if devices.is_empty() {
        list.set_text_content(Some("No devices found."));
    }

    for device in devices {
        let Ok(row) = document.create_element("article") else {
            continue;
        };
        row.set_class_name("device-row");
        let _ = row.set_attribute("data-device", &device.did);
        let _ = row.set_attribute("data-status", &device.status);

        let Ok(heading) = document.create_element("div") else {
            continue;
        };
        heading.set_class_name("device-row__heading");
        let Ok(name) = document.create_element("strong") else {
            continue;
        };
        name.set_text_content(Some(&device.name));
        let _ = heading.append_child(&name);
        let Ok(state) = document.create_element("span") else {
            continue;
        };
        state.set_class_name("device-row__state");
        state.set_text_content(Some(if device.this_device {
            "current device"
        } else {
            &device.status
        }));
        let _ = heading.append_child(&state);
        let _ = row.append_child(&heading);

        let Ok(added) = document.create_element("div") else {
            continue;
        };
        added.set_class_name("device-row__added");
        added.set_text_content(Some(&format!("added {}", format_date(device.created_at))));
        let _ = row.append_child(&added);

        let revocable = !device.this_device
            && device.status == "active"
            && device
                .delegation_hex
                .as_deref()
                .is_some_and(|proof| !proof.is_empty());
        if revocable {
            let Ok(remove) = document.create_element("button") else {
                continue;
            };
            remove.set_class_name("device-row__remove");
            let _ = remove.set_attribute("type", "button");
            let _ = remove.set_attribute("data-revoke", "");
            let _ = remove.set_attribute("data-navigate", &revoke_path(device));
            remove.set_text_content(Some("remove access"));
            let _ = row.append_child(&remove);
        }
        let _ = list.append_child(&row);
    }
    set_hidden(this, "[data-devices-error]", true);
}

fn render_local_profile(this: &HtmlElement) {
    set_hidden(this, "[data-local-profile]", false);
    set_hidden(this, "[data-attached-account]", true);
    set_hidden(this, "[data-local-devices]", false);
    set_hidden(this, "[data-attached-devices]", true);
}

fn profile_label(profile: &ProfileRosterEntry) -> &str {
    profile
        .display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            profile
                .email
                .as_deref()
                .filter(|email| !email.trim().is_empty())
        })
        .unwrap_or(&profile.profile_name)
}

fn render_profiles(this: &HtmlElement, response: &ProfilesResponse) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Some(list) = this.query_selector("[data-profile-list]").ok().flatten() else {
        return;
    };
    list.set_inner_html("");

    let active = response
        .profiles
        .iter()
        .find(|profile| profile.active || profile.profile_name == response.active);
    if let Some(active) = active {
        if let Ok(Some(label)) = this.query_selector("[data-account-label]") {
            label.set_text_content(Some(profile_label(active)));
        }
        let _ = this.set_attribute("data-active-profile", &active.profile_name);
        let _ = this.set_attribute(
            "data-active-provider",
            if active.provider.is_some() {
                "true"
            } else {
                "false"
            },
        );
        let _ = this.set_attribute(
            "data-active-name",
            active.display_name.as_deref().unwrap_or_default(),
        );
    }

    for profile in &response.profiles {
        let is_active = profile.active || profile.profile_name == response.active;
        let Ok(row) = document.create_element(if is_active { "div" } else { "button" }) else {
            continue;
        };
        row.set_class_name("account-menu__row account-menu__profile");
        let _ = row.set_attribute("data-profile", &profile.profile_name);
        let _ = row.set_attribute("role", "menuitem");
        if is_active {
            let _ = row.set_attribute("aria-current", "true");
            let _ = row.set_attribute("tabindex", "-1");
        } else {
            let _ = row.set_attribute("type", "button");
        }

        let Ok(label) = document.create_element("span") else {
            continue;
        };
        label.set_text_content(Some(profile_label(profile)));
        let _ = row.append_child(&label);
        if is_active {
            let Ok(current) = document.create_element("span") else {
                continue;
            };
            current.set_class_name("account-menu__current");
            current.set_text_content(Some("current"));
            let _ = row.append_child(&current);
        }
        let _ = list.append_child(&row);
    }
}

#[derive(Default)]
struct UiHubAccount {
    click: Option<EventClosure>,
    keydown: Option<KeyClosure>,
    outside_pointer: Option<EventClosure>,
    focusout: Option<EventClosure>,
    settings_opener: Rc<RefCell<Option<HtmlElement>>>,
    generation: Rc<Cell<u64>>,
    action_pending: Rc<Cell<bool>>,
    display_name_pending: Rc<Cell<bool>>,
}

impl CustomElement for UiHubAccount {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        if this
            .query_selector("[data-account-trigger]")
            .ok()
            .flatten()
            .is_none()
        {
            this.set_inner_html(include_str!("ui_hub_account.html"));
        }
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        if self.click.is_some() {
            return;
        }

        let token = self.generation.get().wrapping_add(1);
        self.generation.set(token);
        load_profiles(this.clone(), self.generation.clone(), token);

        let host = this.clone();
        let settings_opener = self.settings_opener.clone();
        let generation = self.generation.clone();
        let action_pending = self.action_pending.clone();
        let click: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            if target
                .closest("[data-account-trigger]")
                .ok()
                .flatten()
                .is_some()
            {
                let expanded = account_trigger(&host)
                    .and_then(|trigger| trigger.get_attribute("aria-expanded"))
                    .as_deref()
                    == Some("true");
                if !expanded {
                    open_menu(&host);
                }
                return;
            }
            if target
                .closest("[data-open-settings]")
                .ok()
                .flatten()
                .is_some()
            {
                open_settings(&host, &settings_opener);
                load_settings(host.clone(), generation.clone(), generation.get());
                return;
            }
            if target
                .closest("[data-return-spaces]")
                .ok()
                .flatten()
                .is_some()
            {
                close_menu(&host, false);
                close_settings(&host, &settings_opener, false);
                return;
            }
            if let Some(tab) = target.closest("[data-settings-tab]").ok().flatten()
                && let Some(name) = tab.get_attribute("data-settings-tab")
            {
                select_settings_tab(&host, &name);
                return;
            }
            if let Some(profile) = target.closest("button[data-profile]").ok().flatten()
                && let Some(profile_name) = profile.get_attribute("data-profile")
            {
                activate_profile(
                    host.clone(),
                    profile_name,
                    action_pending.clone(),
                    generation.clone(),
                    generation.get(),
                );
                return;
            }
            if target
                .closest("[data-add-profile]")
                .ok()
                .flatten()
                .is_some()
            {
                add_profile(
                    host.clone(),
                    action_pending.clone(),
                    generation.clone(),
                    generation.get(),
                );
                return;
            }
            if let Some(navigation) = target
                .closest("[data-navigate], [data-account-handoff]")
                .ok()
                .flatten()
            {
                let path = navigation
                    .get_attribute("data-navigate")
                    .unwrap_or_else(|| "/account".into());
                tonk_host::navigate_to(&path);
            }
        }));
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());

        let host = this.clone();
        let settings_opener = self.settings_opener.clone();
        let display_name_pending = self.display_name_pending.clone();
        let generation_for_key = self.generation.clone();
        let keydown: KeyClosure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            let display_name_target = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| {
                    target
                        .closest("[data-display-name]")
                        .ok()
                        .flatten()
                        .is_some()
                });
            if event.key() == "Enter" && display_name_target {
                event.prevent_default();
                commit_display_name(
                    host.clone(),
                    display_name_pending.clone(),
                    generation_for_key.clone(),
                    generation_for_key.get(),
                );
            } else if event.key() == "Escape" {
                if settings_open(&host) {
                    event.prevent_default();
                    close_settings(&host, &settings_opener, true);
                } else {
                    close_menu(&host, true);
                }
            }
        }));
        let _ = this.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());

        let host = this.clone();
        let display_name_pending = self.display_name_pending.clone();
        let generation_for_blur = self.generation.clone();
        let focusout: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let display_name_target = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| {
                    target
                        .closest("[data-display-name]")
                        .ok()
                        .flatten()
                        .is_some()
                });
            if display_name_target {
                commit_display_name(
                    host.clone(),
                    display_name_pending.clone(),
                    generation_for_blur.clone(),
                    generation_for_blur.get(),
                );
            }
        }));
        let _ =
            this.add_event_listener_with_callback("focusout", focusout.as_ref().unchecked_ref());

        let host = this.clone();
        let outside_pointer: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let inside = event
                .target()
                .and_then(|target| target.dyn_into::<Node>().ok())
                .is_some_and(|target| host.contains(Some(&target)));
            if !inside {
                close_menu(&host, true);
            }
        }));
        if let Some(document) = window().and_then(|window| window.document()) {
            let _ = document.add_event_listener_with_callback(
                "pointerdown",
                outside_pointer.as_ref().unchecked_ref(),
            );
        }

        self.click = Some(click);
        self.keydown = Some(keydown);
        self.focusout = Some(focusout);
        self.outside_pointer = Some(outside_pointer);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.action_pending.set(false);
        self.display_name_pending.set(false);
        set_action_pending(this, false);
        if let Ok(Some(input)) = this.query_selector("[data-display-name]")
            && let Ok(input) = input.dyn_into::<HtmlInputElement>()
        {
            input.set_disabled(false);
            let _ = input.remove_attribute("aria-busy");
        }
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        if let Some(keydown) = self.keydown.take() {
            let _ = this
                .remove_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        }
        if let Some(focusout) = self.focusout.take() {
            let _ = this
                .remove_event_listener_with_callback("focusout", focusout.as_ref().unchecked_ref());
        }
        if let Some(pointer) = self.outside_pointer.take()
            && let Some(document) = window().and_then(|window| window.document())
        {
            let _ = document.remove_event_listener_with_callback(
                "pointerdown",
                pointer.as_ref().unchecked_ref(),
            );
        }
        close_menu(this, false);
        close_settings(this, &self.settings_opener, false);
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

fn current(generation: &Cell<u64>, token: u64) -> bool {
    generation.get() == token
}

fn show_error(this: &HtmlElement, selector: &str, message: &str) {
    set_text(this, selector, message);
    set_hidden(this, selector, false);
}

fn load_profiles(this: HtmlElement, generation: Rc<Cell<u64>>, token: u64) {
    spawn_local(async move {
        let result = tonk_host::get_json("/api/profiles").await.and_then(|body| {
            serde_json::from_str::<ProfilesResponse>(&body).map_err(|error| {
                tonk_host::error::ErrorDetail::new(
                    tonk_host::error::ErrorKind::Parse,
                    format!("parse profiles response: {error}"),
                )
            })
        });
        if !current(&generation, token) {
            return;
        }
        match result {
            Ok(response) => {
                render_profiles(&this, &response);
                set_hidden(&this, "[data-account-error]", true);
            }
            Err(error) => show_error(&this, "[data-account-error]", &error.message),
        }
    });
}

fn load_settings(this: HtmlElement, generation: Rc<Cell<u64>>, token: u64) {
    if this.get_attribute("data-active-provider").as_deref() == Some("false") {
        render_local_profile(&this);
        return;
    }

    set_text(&this, "[data-account-email]", "Loading…");
    set_text(&this, "[data-passkey-created]", "Loading…");
    set_text(&this, "[data-passkey-created-on]", "Loading…");
    set_hidden(&this, "[data-account-summary-error]", true);
    set_hidden(&this, "[data-devices-error]", true);

    let summary_host = this.clone();
    let summary_generation = generation.clone();
    spawn_local(async move {
        let result = tonk_host::get_json("/api/account/summary")
            .await
            .and_then(|body| {
                serde_json::from_str::<AccountSummary>(&body).map_err(|error| {
                    tonk_host::error::ErrorDetail::new(
                        tonk_host::error::ErrorKind::Parse,
                        format!("parse account summary: {error}"),
                    )
                })
            });
        if !current(&summary_generation, token) {
            return;
        }
        match result {
            Ok(summary) => render_account_summary(&summary_host, &summary),
            Err(error) => {
                set_text(&summary_host, "[data-account-email]", "Unavailable");
                set_text(&summary_host, "[data-passkey-created]", "Unavailable");
                set_text(&summary_host, "[data-passkey-created-on]", "Unavailable");
                show_error(
                    &summary_host,
                    "[data-account-summary-error]",
                    &error.message,
                );
            }
        }
    });

    spawn_local(async move {
        let result = tonk_host::get_json("/api/account/devices")
            .await
            .and_then(|body| {
                serde_json::from_str::<Vec<AccountDevice>>(&body).map_err(|error| {
                    tonk_host::error::ErrorDetail::new(
                        tonk_host::error::ErrorKind::Parse,
                        format!("parse account devices: {error}"),
                    )
                })
            });
        if !current(&generation, token) {
            return;
        }
        match result {
            Ok(devices) => render_devices(&this, &devices),
            Err(error) => show_error(&this, "[data-devices-error]", &error.message),
        }
    });
}

fn set_action_pending(this: &HtmlElement, pending: bool) {
    let _ = this.set_attribute("aria-busy", if pending { "true" } else { "false" });
    if let Ok(buttons) = this.query_selector_all("button[data-profile], [data-add-profile]") {
        for index in 0..buttons.length() {
            if let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            {
                if pending {
                    let _ = button.set_attribute("disabled", "");
                } else {
                    let _ = button.remove_attribute("disabled");
                }
            }
        }
    }
}

fn activate_profile(
    this: HtmlElement,
    profile_name: String,
    pending: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
    token: u64,
) {
    if pending.replace(true) {
        return;
    }
    set_action_pending(&this, true);
    set_hidden(&this, "[data-account-error]", true);
    spawn_local(async move {
        let request = tonk_worker_api::ActivateProfileRequest {
            profile: profile_name,
        };
        let result = match serde_json::to_string(&request) {
            Ok(body) => tonk_host::post_json("/api/profiles/activate", &body)
                .await
                .map_err(|error| error.message),
            Err(error) => Err(error.to_string()),
        };
        if !current(&generation, token) {
            return;
        }
        pending.set(false);
        set_action_pending(&this, false);
        match result {
            Ok(_) => tonk_host::navigate_to("/"),
            Err(message) => show_error(&this, "[data-account-error]", &message),
        }
    });
}

fn add_profile(this: HtmlElement, pending: Rc<Cell<bool>>, generation: Rc<Cell<u64>>, token: u64) {
    if pending.replace(true) {
        return;
    }
    set_action_pending(&this, true);
    set_hidden(&this, "[data-account-error]", true);
    spawn_local(async move {
        let result = tonk_host::post_json("/api/profiles/add", "{}").await;
        if !current(&generation, token) {
            return;
        }
        pending.set(false);
        set_action_pending(&this, false);
        match result {
            Ok(_) => tonk_host::navigate_to("/account"),
            Err(error) => show_error(&this, "[data-account-error]", &error.message),
        }
    });
}

fn commit_display_name(
    this: HtmlElement,
    pending: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
    token: u64,
) {
    if pending.get() {
        return;
    }
    let Some(input) = this
        .query_selector("[data-display-name]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };
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

    pending.set(true);
    input.set_disabled(true);
    let _ = input.set_attribute("aria-busy", "true");
    set_hidden(&this, "[data-display-name-error]", true);
    spawn_local(async move {
        let result = tonk_host::set_account_display_name(&name).await;
        if !current(&generation, token) {
            return;
        }
        pending.set(false);
        input.set_disabled(false);
        let _ = input.remove_attribute("aria-busy");
        match result {
            Ok(authoritative) => {
                input.set_value(&authoritative);
                let _ = input.set_attribute("data-confirmed-name", &authoritative);
                let _ = this.set_attribute("data-active-name", &authoritative);
                set_text(&this, "[data-account-label]", &authoritative);
            }
            Err(error) => {
                input.set_value(&confirmed);
                show_error(&this, "[data-display-name-error]", &error.message);
            }
        }
    });
}

fn account_trigger(this: &HtmlElement) -> Option<HtmlElement> {
    this.query_selector("[data-account-trigger]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into().ok())
}

fn open_menu(this: &HtmlElement) {
    let return_view = if settings_open(this) {
        "settings"
    } else {
        "spaces"
    };
    let _ = this.set_attribute("data-account-return-view", return_view);
    set_hidden(this, "[data-settings-view]", true);
    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        let _ = spaces.remove_attribute("aria-current");
    }
    if let Ok(Some(settings)) = this.query_selector("[data-open-settings]") {
        let _ = settings.remove_attribute("aria-current");
    }
    if let Some(root) = this.closest(".hubcol").ok().flatten() {
        let _ = root.set_attribute("data-hub-view", "accounts");
        if let Ok(Some(stack)) = root.query_selector("[data-spaces-view]")
            && let Ok(stack) = stack.dyn_into::<HtmlElement>()
        {
            stack.set_hidden(true);
        }
    }
    if let Some(trigger) = account_trigger(this) {
        let _ = trigger.set_attribute("aria-expanded", "true");
        let _ = trigger.set_attribute("aria-current", "page");
    }
    if let Ok(Some(menu)) = this.query_selector("[data-account-menu]")
        && let Ok(menu) = menu.dyn_into::<HtmlElement>()
    {
        menu.set_hidden(false);
    }
}

fn close_menu(this: &HtmlElement, restore_focus: bool) {
    let was_open = account_trigger(this)
        .and_then(|trigger| trigger.get_attribute("aria-expanded"))
        .as_deref()
        == Some("true");
    if let Some(trigger) = account_trigger(this) {
        let _ = trigger.set_attribute("aria-expanded", "false");
        let _ = trigger.remove_attribute("aria-current");
        if restore_focus {
            let _ = trigger.focus();
        }
    }
    if let Ok(Some(menu)) = this.query_selector("[data-account-menu]")
        && let Ok(menu) = menu.dyn_into::<HtmlElement>()
    {
        menu.set_hidden(true);
    }
    if !was_open {
        return;
    }

    let return_to_settings =
        this.get_attribute("data-account-return-view").as_deref() == Some("settings");
    let _ = this.remove_attribute("data-account-return-view");
    set_hidden(this, "[data-settings-view]", !return_to_settings);
    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        if return_to_settings {
            let _ = spaces.remove_attribute("aria-current");
        } else {
            let _ = spaces.set_attribute("aria-current", "page");
        }
    }
    if let Ok(Some(settings)) = this.query_selector("[data-open-settings]") {
        if return_to_settings {
            let _ = settings.set_attribute("aria-current", "page");
        } else {
            let _ = settings.remove_attribute("aria-current");
        }
    }
    if let Some(root) = this.closest(".hubcol").ok().flatten() {
        if return_to_settings {
            let _ = root.set_attribute("data-hub-view", "settings");
        } else {
            let _ = root.remove_attribute("data-hub-view");
        }
        if let Ok(Some(stack)) = root.query_selector("[data-spaces-view]")
            && let Ok(stack) = stack.dyn_into::<HtmlElement>()
        {
            stack.set_hidden(return_to_settings);
        }
    }
}

fn settings_open(this: &HtmlElement) -> bool {
    this.query_selector("[data-settings-view]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlElement>().ok())
        .is_some_and(|view| !view.hidden())
}

fn open_settings(this: &HtmlElement, opener: &Rc<RefCell<Option<HtmlElement>>>) {
    close_menu(this, false);
    *opener.borrow_mut() = this
        .query_selector("[data-open-settings]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into().ok());
    set_hidden(this, "[data-settings-view]", false);
    if let Some(root) = this.closest(".hubcol").ok().flatten() {
        let _ = root.set_attribute("data-hub-view", "settings");
        if let Ok(Some(stack)) = root.query_selector("[data-spaces-view]")
            && let Ok(stack) = stack.dyn_into::<HtmlElement>()
        {
            stack.set_hidden(true);
        }
    }
    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        let _ = spaces.remove_attribute("aria-current");
    }
    if let Ok(Some(settings)) = this.query_selector("[data-open-settings]") {
        let _ = settings.set_attribute("aria-current", "page");
    }
    select_settings_tab(this, "account");

    if this.get_attribute("data-active-provider").as_deref() == Some("false") {
        render_local_profile(this);
    } else {
        set_hidden(this, "[data-local-profile]", true);
        set_hidden(this, "[data-attached-account]", false);
        set_hidden(this, "[data-local-devices]", true);
        set_hidden(this, "[data-attached-devices]", false);
    }
    if let Some(name) = this.get_attribute("data-active-name")
        && let Ok(Some(input)) = this.query_selector("[data-display-name]")
        && let Ok(input) = input.dyn_into::<web_sys::HtmlInputElement>()
    {
        input.set_value(&name);
        let _ = input.set_attribute("data-confirmed-name", &name);
    }
}

fn close_settings(
    this: &HtmlElement,
    opener: &Rc<RefCell<Option<HtmlElement>>>,
    restore_focus: bool,
) {
    set_hidden(this, "[data-settings-view]", true);
    if let Some(root) = this.closest(".hubcol").ok().flatten() {
        let _ = root.remove_attribute("data-hub-view");
        if let Ok(Some(stack)) = root.query_selector("[data-spaces-view]")
            && let Ok(stack) = stack.dyn_into::<HtmlElement>()
        {
            stack.set_hidden(false);
        }
    }
    if let Ok(Some(settings)) = this.query_selector("[data-open-settings]") {
        let _ = settings.remove_attribute("aria-current");
    }
    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        let _ = spaces.set_attribute("aria-current", "page");
    }
    if let Some(opener) = opener.borrow_mut().take()
        && restore_focus
    {
        let _ = opener.focus();
    }
}

fn select_settings_tab(this: &HtmlElement, selected: &str) {
    if let Ok(tabs) = this.query_selector_all("[data-settings-tab]") {
        for index in 0..tabs.length() {
            let Some(tab) = tabs
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let active = tab.get_attribute("data-settings-tab").as_deref() == Some(selected);
            let _ = tab.set_attribute("aria-selected", if active { "true" } else { "false" });
            let _ = tab.set_attribute("tabindex", if active { "0" } else { "-1" });
        }
    }
    if let Ok(panes) = this.query_selector_all("[data-settings-pane]") {
        for index in 0..panes.length() {
            let Some(pane) = panes
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let hidden = pane.get_attribute("data-settings-pane").as_deref() != Some(selected);
            if let Ok(pane) = pane.dyn_into::<HtmlElement>() {
                pane.set_hidden(hidden);
            }
        }
    }
}

/// Register `<ui-hub-account>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-hub-account").is_undefined() {
        UiHubAccount::define("ui-hub-account");
    }
}

#[cfg(test)]
mod tests {
    use tonk_worker_api::{
        AccountDevice, AccountSummary, PasskeyMetadata, ProfileRosterEntry, ProfilesResponse,
    };
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{Event, EventInit, HtmlElement, KeyboardEvent, KeyboardEventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    fn profile(
        profile_name: &str,
        display_name: Option<&str>,
        email: Option<&str>,
        provider: Option<&str>,
        active: bool,
    ) -> ProfileRosterEntry {
        ProfileRosterEntry {
            profile_name: profile_name.into(),
            root_did: provider.map(|_| format!("did:key:{profile_name}")),
            provider: provider.map(str::to_owned),
            email: email.map(str::to_owned),
            display_name: display_name.map(str::to_owned),
            last_active_at: 0,
            active,
        }
    }

    fn account_element() -> HtmlElement {
        super::register();
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("ui-hub-account")
            .expect("create account element")
            .dyn_into()
            .expect("HtmlElement");
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("append account element");
        host
    }

    #[wasm_bindgen_test]
    fn it_registers_the_hub_account_element() {
        super::register();
        let registry = window().expect("window").custom_elements();
        assert!(
            !registry.get("ui-hub-account").is_undefined(),
            "ui-hub-account must be registered"
        );
    }

    #[wasm_bindgen_test]
    fn it_mounts_the_complete_header_and_attached_settings_view() {
        let host = account_element();

        assert!(
            host.query_selector("[data-account-menu]")
                .expect("valid selector")
                .is_some(),
            "the registered element must mount its account menu"
        );
        assert!(
            host.query_selector("[data-settings-view]")
                .expect("valid selector")
                .is_some(),
            "the registered element must mount its attached settings view"
        );
        assert_eq!(host.query_selector_all(".hubbar > *").unwrap().length(), 4);
        for rejected in [
            "[data-settings-dialog]",
            "[data-settings-overlay]",
            "[data-settings-scrim]",
            "[data-settings-close]",
        ] {
            assert!(host.query_selector(rejected).unwrap().is_none());
        }
        let display_name = host
            .query_selector("[data-display-name]")
            .expect("valid selector")
            .expect("display name input");
        assert_eq!(
            display_name.get_attribute("id").as_deref(),
            Some("display-name")
        );
        assert_eq!(
            display_name.get_attribute("name").as_deref(),
            Some("display-name")
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_renders_the_existing_profile_roster_in_the_account_menu() {
        let host = account_element();
        let response = ProfilesResponse {
            active: "primary".into(),
            profiles: vec![
                profile(
                    "primary",
                    Some("Ada Lovelace"),
                    Some("ada@example.com"),
                    Some("https://accounts.example"),
                    true,
                ),
                profile(
                    "second",
                    None,
                    Some("grace@example.com"),
                    Some("https://accounts.example"),
                    false,
                ),
                profile("Local workspace", None, None, None, false),
            ],
        };

        super::render_profiles(&host, &response);

        let label = host
            .query_selector("[data-account-label]")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap_or_default();
        assert_eq!(label, "Ada Lovelace");
        let current = host
            .query_selector("[data-profile=\"primary\"]")
            .unwrap()
            .expect("current profile row");
        assert_eq!(
            current.get_attribute("aria-current").as_deref(),
            Some("true")
        );
        assert_eq!(
            current.tag_name(),
            "DIV",
            "the current account is not actionable"
        );
        let switches = host.query_selector_all("button[data-profile]").unwrap();
        assert_eq!(switches.length(), 2);
        assert!(
            switches
                .item(0)
                .unwrap()
                .text_content()
                .unwrap()
                .contains("grace@example.com")
        );
        assert!(
            switches
                .item(1)
                .unwrap()
                .text_content()
                .unwrap()
                .contains("Local workspace")
        );
        assert!(host.query_selector("[data-add-profile]").unwrap().is_some());
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_opens_and_dismisses_the_account_menu_accessibly() {
        let host = account_element();
        let document = window().unwrap().document().unwrap();
        let trigger: HtmlElement = host
            .query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        trigger.click();
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("true")
        );
        assert!(!menu.hidden());

        let pointer_init = EventInit::new();
        pointer_init.set_bubbles(true);
        document
            .body()
            .unwrap()
            .dispatch_event(&Event::new_with_event_init_dict("pointerdown", &pointer_init).unwrap())
            .unwrap();
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert!(menu.hidden());
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&trigger)))
        );

        trigger.click();
        let init = KeyboardEventInit::new();
        init.set_key("Escape");
        init.set_bubbles(true);
        trigger
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
            )
            .unwrap();
        assert!(menu.hidden());
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&trigger)))
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_treats_the_account_roster_as_a_tab_and_restores_the_previous_view_on_dismiss() {
        let document = window().unwrap().document().unwrap();
        let hubcol: HtmlElement = document.create_element("main").unwrap().dyn_into().unwrap();
        hubcol.set_class_name("hubcol");
        let host = account_element();
        let stack: HtmlElement = document
            .create_element("section")
            .unwrap()
            .dyn_into()
            .unwrap();
        stack.set_class_name("stack");
        let _ = stack.set_attribute("data-spaces-view", "");
        hubcol.append_child(&host).unwrap();
        hubcol.append_child(&stack).unwrap();
        document.body().unwrap().append_child(&hubcol).unwrap();

        let trigger: HtmlElement = host
            .query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let settings_button: HtmlElement = host
            .query_selector("[data-open-settings]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let settings: HtmlElement = host
            .query_selector("[data-settings-view]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        settings_button.click();
        trigger.click();
        assert!(!menu.hidden());
        assert!(settings.hidden(), "the roster replaces the settings view");
        assert!(stack.hidden());
        assert_eq!(
            hubcol.get_attribute("data-hub-view").as_deref(),
            Some("accounts")
        );
        assert_eq!(
            trigger.get_attribute("aria-current").as_deref(),
            Some("page")
        );
        assert!(settings_button.get_attribute("aria-current").is_none());
        assert!(
            host.query_selector("[data-return-spaces]")
                .unwrap()
                .unwrap()
                .get_attribute("aria-current")
                .is_none()
        );

        let escape = KeyboardEventInit::new();
        escape.set_key("Escape");
        escape.set_bubbles(true);
        trigger
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape).unwrap(),
            )
            .unwrap();
        assert!(menu.hidden());
        assert!(!settings.hidden(), "closing the roster restores settings");
        assert!(stack.hidden());
        assert_eq!(
            hubcol.get_attribute("data-hub-view").as_deref(),
            Some("settings")
        );
        assert!(trigger.get_attribute("aria-current").is_none());
        assert_eq!(
            settings_button.get_attribute("aria-current").as_deref(),
            Some("page")
        );

        host.query_selector("[data-return-spaces]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        trigger.click();
        assert!(!menu.hidden());
        assert!(settings.hidden());
        assert!(stack.hidden(), "the roster replaces the spaces view");
        assert_eq!(
            trigger.get_attribute("aria-current").as_deref(),
            Some("page")
        );
        assert!(
            host.query_selector("[data-return-spaces]")
                .unwrap()
                .unwrap()
                .get_attribute("aria-current")
                .is_none()
        );
        trigger.click();
        assert!(!menu.hidden(), "the active account tab is not a toggle");
        assert!(settings.hidden());
        assert!(stack.hidden());
        assert_eq!(
            trigger.get_attribute("aria-current").as_deref(),
            Some("page")
        );
        assert_eq!(
            hubcol.get_attribute("data-hub-view").as_deref(),
            Some("accounts")
        );

        trigger
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape).unwrap(),
            )
            .unwrap();
        assert!(menu.hidden());
        assert!(settings.hidden());
        assert!(!stack.hidden(), "dismissing the roster restores spaces");
        assert!(trigger.get_attribute("aria-current").is_none());
        assert_eq!(
            host.query_selector("[data-return-spaces]")
                .unwrap()
                .unwrap()
                .get_attribute("aria-current")
                .as_deref(),
            Some("page")
        );

        hubcol.remove();
    }

    #[wasm_bindgen_test]
    fn it_renders_truthful_account_and_device_settings_only() {
        let host = account_element();
        super::render_account_summary(
            &host,
            &AccountSummary {
                email: Some("ada@example.com".into()),
                passkey: Some(PasskeyMetadata {
                    created_at: 1_754_380_800,
                    created_on: "Chrome on macOS".into(),
                }),
            },
        );
        assert_eq!(
            host.query_selector("[data-account-email]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(
            host.query_selector("[data-passkey-created]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("5 Aug 2025")
        );
        assert_eq!(
            host.query_selector("[data-passkey-created-on]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Chrome on macOS")
        );

        let devices = vec![
            AccountDevice {
                attachment_id: "current/1".into(),
                did: "did:key:current".into(),
                name: "This browser".into(),
                status: "active".into(),
                created_at: 1_754_380_800,
                delegation_cid: "cid-current".into(),
                delegation_hex: Some("aa".into()),
                this_device: true,
            },
            AccountDevice {
                attachment_id: "other & one".into(),
                did: "did:key:other/one".into(),
                name: "Other browser".into(),
                status: "active".into(),
                created_at: 1_754_380_800,
                delegation_cid: "cid-other".into(),
                delegation_hex: Some("bb".into()),
                this_device: false,
            },
            AccountDevice {
                attachment_id: "revoked".into(),
                did: "did:key:revoked".into(),
                name: "Old browser".into(),
                status: "revoked".into(),
                created_at: 1_754_380_800,
                delegation_cid: "cid-revoked".into(),
                delegation_hex: Some("cc".into()),
                this_device: false,
            },
        ];
        super::render_devices(&host, &devices);
        assert_eq!(
            host.query_selector_all("[data-device]").unwrap().length(),
            3
        );
        assert_eq!(
            host.query_selector_all("[data-revoke]").unwrap().length(),
            1
        );
        let revoke = host.query_selector("[data-revoke]").unwrap().unwrap();
        assert_eq!(
            revoke.get_attribute("data-navigate").as_deref(),
            Some("/account?revoke=did%3Akey%3Aother%2Fone&attachment=other+%26+one")
        );
        let text = host.text_content().unwrap_or_default();
        assert!(text.contains("current device"));
        assert!(text.contains("revoked"));
        assert_eq!(
            host.query_selector_all("[data-settings-tab]")
                .unwrap()
                .length(),
            2
        );
        for forbidden in ["usage", "upgrade", "metering", "syncing"] {
            assert!(
                !text.to_ascii_lowercase().contains(forbidden),
                "settings must not contain {forbidden}"
            );
        }
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_renders_legacy_unreachable_and_local_profile_states() {
        let host = account_element();
        super::render_account_summary(
            &host,
            &AccountSummary {
                email: None,
                passkey: None,
            },
        );
        assert_eq!(
            host.query_selector("[data-account-email]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Unavailable")
        );
        assert_eq!(
            host.query_selector("[data-passkey-created]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Not recorded")
        );

        super::render_local_profile(&host);
        let local: HtmlElement = host
            .query_selector("[data-local-profile]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let attached: HtmlElement = host
            .query_selector("[data-attached-account]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!local.hidden());
        assert!(attached.hidden());
        assert!(
            local
                .text_content()
                .unwrap()
                .contains("This profile is not connected to an account")
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_keeps_provider_free_profiles_local_spaces_and_creation_available() {
        let document = window().unwrap().document().unwrap();
        let hubcol: HtmlElement = document.create_element("main").unwrap().dyn_into().unwrap();
        hubcol.set_class_name("hubcol");
        let host = account_element();
        let stack: HtmlElement = document
            .create_element("section")
            .unwrap()
            .dyn_into()
            .unwrap();
        stack.set_class_name("stack");
        let _ = stack.set_attribute("data-spaces-view", "");
        stack.set_inner_html(
            r#"<a class="srow" href="/space/did:key:local">Local notes</a>
               <button class="snew" type="submit">create a new space</button>"#,
        );
        hubcol.append_child(&host).unwrap();
        hubcol.append_child(&stack).unwrap();
        document.body().unwrap().append_child(&hubcol).unwrap();

        super::render_profiles(
            &host,
            &ProfilesResponse {
                active: "Local workspace".into(),
                profiles: vec![profile("Local workspace", None, None, None, true)],
            },
        );
        let account: HtmlElement = host
            .query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(account.text_content().unwrap().contains("Local workspace"));
        account.click();
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!menu.hidden(), "the local profile still opens its roster");

        let settings: HtmlElement = host
            .query_selector("[data-open-settings]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        settings.click();
        let settings_view: HtmlElement = host
            .query_selector("[data-settings-view]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!settings_view.hidden());
        assert!(stack.hidden());
        let local_account: HtmlElement = host
            .query_selector("[data-local-profile]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!local_account.hidden());
        assert!(
            local_account
                .text_content()
                .unwrap()
                .contains("not connected to an account")
        );
        assert!(
            local_account
                .query_selector("[data-account-handoff]")
                .unwrap()
                .is_some()
        );

        let devices: HtmlElement = host
            .query_selector("[data-settings-tab=\"devices\"]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        devices.click();
        let local_devices: HtmlElement = host
            .query_selector("[data-local-devices]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!local_devices.hidden());
        assert!(
            local_devices
                .text_content()
                .unwrap()
                .contains("require an account")
        );

        let spaces: HtmlElement = host
            .query_selector("[data-return-spaces]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        spaces.click();
        assert!(!stack.hidden());
        assert!(stack.query_selector(".srow[href]").unwrap().is_some());
        assert!(
            stack
                .query_selector(".snew")
                .unwrap()
                .unwrap()
                .get_attribute("disabled")
                .is_none()
        );
        hubcol.remove();
    }

    #[wasm_bindgen_test]
    fn it_owns_the_attached_settings_view_and_header_lifecycle() {
        let document = window().unwrap().document().unwrap();
        let hubcol: HtmlElement = document.create_element("main").unwrap().dyn_into().unwrap();
        hubcol.set_class_name("hubcol");
        let host = account_element();
        let stack: HtmlElement = document
            .create_element("section")
            .unwrap()
            .dyn_into()
            .unwrap();
        stack.set_class_name("stack");
        let _ = stack.set_attribute("data-spaces-view", "");
        hubcol.append_child(&host).unwrap();
        hubcol.append_child(&stack).unwrap();
        document.body().unwrap().append_child(&hubcol).unwrap();

        let settings_button: HtmlElement = host
            .query_selector("[data-open-settings]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let spaces_button: HtmlElement = host
            .query_selector("[data-return-spaces]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        settings_button.click();

        let settings: HtmlElement = host
            .query_selector("[data-settings-view]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!settings.hidden());
        assert!(stack.hidden());
        assert_eq!(
            hubcol.get_attribute("data-hub-view").as_deref(),
            Some("settings")
        );
        assert_eq!(
            settings_button.get_attribute("aria-current").as_deref(),
            Some("page")
        );
        assert!(spaces_button.get_attribute("aria-current").is_none());
        assert!(menu.hidden(), "opening settings closes the account menu");

        let devices_tab: HtmlElement = host
            .query_selector("[data-settings-tab=\"devices\"]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        devices_tab.click();
        assert_eq!(
            devices_tab.get_attribute("aria-selected").as_deref(),
            Some("true")
        );
        let account_pane: HtmlElement = host
            .query_selector("[data-settings-pane=\"account\"]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let devices_pane: HtmlElement = host
            .query_selector("[data-settings-pane=\"devices\"]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(account_pane.hidden());
        assert!(!devices_pane.hidden());

        spaces_button.click();
        assert!(settings.hidden());
        assert!(!stack.hidden());
        assert!(hubcol.get_attribute("data-hub-view").is_none());
        assert_eq!(
            spaces_button.get_attribute("aria-current").as_deref(),
            Some("page")
        );
        assert!(settings_button.get_attribute("aria-current").is_none());

        settings_button.click();
        let escape_init = KeyboardEventInit::new();
        escape_init.set_key("Escape");
        escape_init.set_bubbles(true);
        settings
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape_init).unwrap(),
            )
            .unwrap();
        assert!(settings.hidden());
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&settings_button)))
        );
        hubcol.remove();
    }
}
