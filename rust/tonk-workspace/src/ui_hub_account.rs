//! `<ui-hub-account>` — the Hub account switcher and settings route.

use std::cell::Cell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_worker_api::{ProfileRosterEntry, ProfilesResponse};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, KeyboardEvent, Node, window};

type EventClosure = Closure<dyn FnMut(Event)>;
type KeyClosure = Closure<dyn FnMut(KeyboardEvent)>;

const ADD_ACCOUNT_PATH: &str = "/settings?add=1";

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
            label.set_text_content(Some(if active.provider.is_some() {
                profile_label(active)
            } else {
                crate::hub_account::trigger_label(Some("false"))
            }));
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
            let _ = row.set_attribute("aria-disabled", "true");
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
    generation: Rc<Cell<u64>>,
    action_pending: Rc<Cell<bool>>,
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
                if crate::hub_account::trigger_asks_to_link(
                    host.get_attribute("data-active-provider").as_deref(),
                ) {
                    close_menu(&host, false);
                    // The Hub is a sealed guest: WebAuthn needs a
                    // `window` and a user gesture, which an opaque
                    // realm does not have. The top page raises the
                    // cluster, asked through the same bridge the share
                    // row asks through.
                    tonk_host::request_registration(
                        &serde_json::json!({
                            "reason": tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT,
                            "space": "",
                        })
                        .to_string(),
                    );
                    return;
                }
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
                event.prevent_default();
                close_menu(&host, false);
                tonk_host::navigate_to("/settings");
                return;
            }
            if target
                .closest("[data-return-spaces]")
                .ok()
                .flatten()
                .is_some()
            {
                close_menu(&host, false);
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
                close_menu(&host, false);
                tonk_host::navigate_to(ADD_ACCOUNT_PATH);
                return;
            }
        }));
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());

        let host = this.clone();
        let keydown: KeyClosure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            let open = account_trigger(&host)
                .and_then(|trigger| trigger.get_attribute("aria-expanded"))
                .as_deref()
                == Some("true");
            if !open {
                return;
            }
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    move_menu_focus(&host, 1, false);
                }
                "ArrowUp" => {
                    event.prevent_default();
                    move_menu_focus(&host, -1, false);
                }
                "Home" => {
                    event.prevent_default();
                    move_menu_focus(&host, 1, true);
                }
                "End" => {
                    event.prevent_default();
                    move_menu_focus(&host, -1, true);
                }
                "Escape" => {
                    event.prevent_default();
                    close_menu(&host, true);
                }
                "Tab" => close_menu(&host, false),
                _ => {}
            }
        }));
        let _ = this.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());

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
        self.outside_pointer = Some(outside_pointer);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.action_pending.set(false);
        set_action_pending(this, false);
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        if let Some(keydown) = self.keydown.take() {
            let _ = this
                .remove_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
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
            Ok(_) => tonk_host::reload_page(),
            Err(message) => show_error(&this, "[data-account-error]", &message),
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
    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        let _ = spaces.remove_attribute("aria-current");
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
    if let Some(first) = menu_items(this).first() {
        let _ = first.focus();
    }
}

fn menu_items(this: &HtmlElement) -> Vec<HtmlElement> {
    let Ok(nodes) = this.query_selector_all("[data-account-menu] [role=menuitem]") else {
        return Vec::new();
    };
    (0..nodes.length())
        .filter_map(|index| nodes.item(index))
        .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
        .filter(|item| {
            item.get_attribute("aria-disabled").as_deref() != Some("true")
                && !item.matches(":disabled").unwrap_or(false)
                && !item.hidden()
        })
        .collect()
}

fn move_menu_focus(this: &HtmlElement, direction: i32, endpoint: bool) {
    let items = menu_items(this);
    if items.is_empty() {
        return;
    }
    let next = if endpoint {
        if direction > 0 { 0 } else { items.len() - 1 }
    } else {
        let active = window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element());
        let current = active
            .and_then(|active| {
                items
                    .iter()
                    .position(|item| active.is_same_node(Some(item)))
            })
            .unwrap_or_else(|| if direction > 0 { items.len() - 1 } else { 0 });
        (current as i32 + direction).rem_euclid(items.len() as i32) as usize
    };
    let _ = items[next].focus();
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

    if let Ok(Some(spaces)) = this.query_selector("[data-return-spaces]") {
        let _ = spaces.set_attribute("aria-current", "page");
    }
    if let Some(root) = this.closest(".hubcol").ok().flatten() {
        let _ = root.remove_attribute("data-hub-view");
        if let Ok(Some(stack)) = root.query_selector("[data-spaces-view]")
            && let Ok(stack) = stack.dyn_into::<HtmlElement>()
        {
            stack.set_hidden(false);
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
    use tonk_worker_api::{ProfileRosterEntry, ProfilesResponse};
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
    fn it_mounts_the_complete_header_without_an_inline_settings_view() {
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
                .is_none()
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
        let settings = host
            .query_selector("a[data-open-settings]")
            .expect("valid selector")
            .expect("settings route");
        assert_eq!(settings.get_attribute("href").as_deref(), Some("/settings"));
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
    fn it_moves_focus_through_the_account_menu_with_the_keyboard() {
        let host = account_element();
        super::render_profiles(
            &host,
            &ProfilesResponse {
                active: "primary".into(),
                profiles: vec![
                    profile("primary", Some("Ada"), None, Some("remote"), true),
                    profile("second", Some("Grace"), None, Some("remote"), false),
                    profile("third", Some("Katherine"), None, Some("remote"), false),
                ],
            },
        );
        let document = window().unwrap().document().unwrap();
        let trigger: HtmlElement = host
            .query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        trigger.click();

        let active_profile = || {
            document
                .active_element()
                .and_then(|element| element.get_attribute("data-profile"))
        };
        assert_eq!(active_profile().as_deref(), Some("second"));

        let key = |value: &str| {
            let init = KeyboardEventInit::new();
            init.set_key(value);
            init.set_bubbles(true);
            document
                .active_element()
                .unwrap()
                .dispatch_event(
                    &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
                )
                .unwrap();
        };
        key("ArrowUp");
        assert!(
            document
                .active_element()
                .unwrap()
                .has_attribute("data-add-profile"),
            "ArrowUp wraps to the last enabled item"
        );
        key("ArrowDown");
        assert_eq!(active_profile().as_deref(), Some("second"));
        key("End");
        assert!(
            document
                .active_element()
                .unwrap()
                .has_attribute("data-add-profile")
        );
        key("Home");
        assert_eq!(active_profile().as_deref(), Some("second"));
        key("Escape");
        assert!(
            document
                .active_element()
                .unwrap()
                .is_same_node(Some(&trigger))
        );
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );

        trigger.click();
        let focused = document.active_element().unwrap();
        let init = KeyboardEventInit::new();
        init.set_key("Tab");
        init.set_bubbles(true);
        let tab = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        focused.dispatch_event(&tab).unwrap();
        assert!(
            !tab.default_prevented(),
            "Tab keeps its browser-default move"
        );
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_treats_the_account_roster_as_a_tab_and_restores_spaces_on_dismiss() {
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
        trigger.click();
        assert!(!menu.hidden());
        assert!(stack.hidden());
        assert_eq!(
            hubcol.get_attribute("data-hub-view").as_deref(),
            Some("accounts")
        );
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

        let escape = KeyboardEventInit::new();
        escape.set_key("Escape");
        escape.set_bubbles(true);
        trigger
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape).unwrap(),
            )
            .unwrap();
        assert!(menu.hidden());
        assert!(!stack.hidden(), "closing the roster restores spaces");
        assert!(hubcol.get_attribute("data-hub-view").is_none());
        assert!(trigger.get_attribute("aria-current").is_none());

        host.query_selector("[data-return-spaces]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        trigger.click();
        assert!(!menu.hidden());
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
        assert_eq!(
            account
                .query_selector("[data-account-label]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("link an account"),
            "an unattached first-run profile must not present its storage name as an account"
        );
        let original_url = window().unwrap().location().href().unwrap();
        account.click();
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(menu.hidden(), "linking must not open the profile roster");
        // In place, not away. It used to navigate to /settings, which
        // put a panel and a second button between the label and the
        // ceremony; the trigger asks the top page for the ceremony
        // instead, and the Hub stays where it is.
        assert_eq!(
            window().unwrap().location().href().unwrap(),
            original_url,
            "linking an account must not navigate the Hub anywhere"
        );
        window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&original_url))
            .unwrap();

        let settings: HtmlElement = host
            .query_selector("[data-open-settings]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        settings.click();
        assert_eq!(
            window().unwrap().location().pathname().unwrap(),
            "/settings"
        );
        window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&original_url))
            .unwrap();
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
    fn it_routes_settings_to_the_top_level_page() {
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
        assert_eq!(settings_button.tag_name(), "A");
        assert_eq!(
            settings_button.get_attribute("href").as_deref(),
            Some("/settings")
        );
        let original_url = window().unwrap().location().href().unwrap();
        settings_button.click();
        assert_eq!(
            window().unwrap().location().pathname().unwrap(),
            "/settings"
        );
        assert!(
            !stack.hidden(),
            "routing must not swap in the old inline view"
        );
        assert!(hubcol.get_attribute("data-hub-view").is_none());
        window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&original_url))
            .unwrap();
        hubcol.remove();
    }
}
