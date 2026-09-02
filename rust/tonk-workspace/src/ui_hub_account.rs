//! `<ui-hub-account>` — the Hub account switcher and settings route.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{JSON, Reflect};
use tonk_host::consumer::{self, Subscription};
use tonk_worker_api::{ProfileRosterEntry, ProfilesResponse};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, KeyboardEvent, Node, window};

type EventClosure = Closure<dyn FnMut(Event)>;
type KeyClosure = Closure<dyn FnMut(KeyboardEvent)>;
type FrameClosure = Closure<dyn FnMut(JsValue, JsValue)>;

const ADD_ACCOUNT_PATH: &str = "/settings?add=1";

/// The PROFILE branch's routing context — fixed, like `<ui-profile-name>`'s.
const PROFILE_WITH: &str = "main@profile:tonk";

/// The account-name subscription tag.
const NAME_TAG: &str = "ui-hub-account:name";

/// The registration subscription tag — the "an account is linked" signal.
const REGISTERED_TAG: &str = "ui-hub-account:registered";

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
        set_trigger_mode(this, active.provider.is_none());
    }

    // The current account is the TAB itself, so the page holds only ways
    // onward: one row per other account, wearing its switch verb.
    for profile in &response.profiles {
        if profile.active || profile.profile_name == response.active {
            continue;
        }
        let Ok(row) = document.create_element("button") else {
            continue;
        };
        row.set_class_name("account-menu__row account-menu__profile");
        let _ = row.set_attribute("data-profile", &profile.profile_name);
        let _ = row.set_attribute("role", "menuitem");
        let _ = row.set_attribute("type", "button");

        let Ok(name) = document.create_element("span") else {
            continue;
        };
        name.set_class_name("an");
        name.set_text_content(Some(profile_label(profile)));
        let _ = row.append_child(&name);
        let _ = row.append_with_str_1("switch account");
        if let Ok(glyph) = document.create_element("span") {
            glyph.set_class_name("g");
            let _ = glyph.set_attribute("aria-hidden", "true");
            glyph.set_text_content(Some("\u{25b8}"));
            let _ = row.append_child(&glyph);
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
    focusin: Option<EventClosure>,
    subscriptions: Rc<RefCell<Vec<Subscription>>>,
    name_reset: Rc<RefCell<Option<FrameClosure>>>,
    name_update: Rc<RefCell<Option<FrameClosure>>>,
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

        // Live account subscriptions: the trigger label must flip from
        // "link an account" to the member's name the moment a registration
        // ceremony lands, and follow later renames — without a reload. Two
        // facts on the profile branch carry those signals, and neither
        // covers the other:
        //
        // - `xyz.tonk.account/registered-at` is asserted the moment an
        //   account links (activation only comes later, with the emailed
        //   confirmation — the display-name seed waits on it, so it CANNOT
        //   be the login signal; the first e2e run proved that the hard
        //   way).
        // - `xyz.tonk.account/display-name` follows renames once the
        //   account state is ready.
        //
        // A delta on either re-reads the roster, which is where the label,
        // the provider flag and the switch rows all come from.
        let host = this.clone();
        let name_reset: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if frame_tag(&opts).as_deref() == Some(NAME_TAG)
                    && let Some(name) = read_name_from_frame(&payload)
                {
                    apply_account_name(&host, &name);
                }
            }));
        let _ = Reflect::set(this, &"__tonkReset".into(), name_reset.as_ref());
        *self.name_reset.borrow_mut() = Some(name_reset);

        let host = this.clone();
        let generation = self.generation.clone();
        let name_update: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                if frame_tag(&opts).as_deref() == Some(NAME_TAG)
                    && let Some(name) = read_name_from_delta(&payload)
                {
                    apply_account_name(&host, &name);
                }
                // Something about the account changed (it linked, or it was
                // renamed): re-read the roster so the trigger and the rows
                // agree with it.
                load_profiles(host.clone(), generation.clone(), generation.get());
            }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), name_update.as_ref());
        *self.name_update.borrow_mut() = Some(name_update);

        subscribe_account_signals(this, self.subscriptions.clone());

        // Focus returning to the trigger is the ceremony's own restore
        // path; while the linking page is up it doubles as "the cluster
        // is gone — put the spaces back".
        let host = this.clone();
        let focusin: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            if host.has_attribute("data-linking")
                && event
                    .target()
                    .and_then(|target| target.dyn_into::<Element>().ok())
                    .and_then(|target| target.closest("[data-account-trigger]").ok().flatten())
                    .is_some()
            {
                leave_linking(&host);
            }
        }));
        let _ = this.add_event_listener_with_callback("focusin", focusin.as_ref().unchecked_ref());
        self.focusin = Some(focusin);

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
                    // row asks through — but seated IN the column, one
                    // gap under the bar, because linking is this tab's
                    // page, not a dialog over it. The tab activates and
                    // the stack steps aside while the ceremony is up;
                    // focus returning to the trigger (the dialog's own
                    // restore path) puts the page back.
                    enter_linking(&host);
                    let anchor = host
                        .query_selector(".hubbar")
                        .ok()
                        .flatten()
                        .map(|bar| bar.get_bounding_client_rect());
                    let payload = match anchor {
                        Some(rect) => serde_json::json!({
                            "reason": tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT,
                            "space": "",
                            "anchor": {
                                "left": rect.left(),
                                "bottom": rect.bottom(),
                                "width": rect.width(),
                            },
                        }),
                        None => serde_json::json!({
                            "reason": tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT,
                            "space": "",
                        }),
                    };
                    tonk_host::request_registration(&payload.to_string());
                    return;
                }
                let expanded = account_trigger(&host)
                    .and_then(|trigger| trigger.get_attribute("aria-expanded"))
                    .as_deref()
                    == Some("true");
                if !expanded {
                    open_menu(&host);
                } else if settings_open(&host) {
                    // From settings the account tab leads back to the
                    // account view — the one press that is otherwise dead.
                    show_settings(&host, false);
                }
                return;
            }
            if target
                .closest("[data-open-settings]")
                .ok()
                .flatten()
                .is_some()
            {
                // Settings opens INSIDE the hub — the stack steps aside and
                // the section hangs from the same bar, exactly like the
                // account view (the wireframe's reading). The anchor keeps
                // its `/settings` href only as the no-handler fallback.
                event.prevent_default();
                open_settings_view(&host);
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
        // Dropping the subscriptions cancels the upstream host subscriptions.
        self.subscriptions.borrow_mut().clear();
        self.name_reset.borrow_mut().take();
        self.name_update.borrow_mut().take();
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        if let Some(focusin) = self.focusin.take() {
            let _ = this
                .remove_event_listener_with_callback("focusin", focusin.as_ref().unchecked_ref());
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

/// Shape the trigger for its two roles. Neither draws a dropdown caret —
/// the cell reads as a tab of the hub bar either way.
///
/// Unlinked, it is a plain action button — pressing it raises the
/// registration cluster, no menu ever opens — so the menu-button ARIA
/// contract (`aria-haspopup`/`aria-expanded`/`aria-controls`) would
/// promise a dropdown that does not exist. Linked, it is the
/// account-menu button and gets it back.
fn set_trigger_mode(this: &HtmlElement, asks_to_link: bool) {
    let Some(trigger) = account_trigger(this) else {
        return;
    };
    if asks_to_link {
        let _ = trigger.remove_attribute("aria-haspopup");
        let _ = trigger.remove_attribute("aria-expanded");
        let _ = trigger.remove_attribute("aria-controls");
    } else {
        let _ = trigger.set_attribute("aria-haspopup", "menu");
        let _ = trigger.set_attribute("aria-controls", "hub-account-menu");
        if trigger.get_attribute("aria-expanded").is_none() {
            let _ = trigger.set_attribute("aria-expanded", "false");
        }
    }
}

/// Open the account subscriptions for `this`, on a microtask — the same
/// detached-reaction guard `<ui-sync-status>` uses.
fn subscribe_account_signals(this: &HtmlElement, subscriptions: Rc<RefCell<Vec<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        if !host.is_connected() || !subscriptions.borrow().is_empty() {
            return;
        }
        // The routing context is the element's own `with`: the account
        // facts live on the PROFILE branch, never on a space.
        let _ = host.set_attribute("with", PROFILE_WITH);
        let consumer: Element = host.clone().into();
        for (tag, body) in [
            (NAME_TAG, account_name_query_body()),
            (REGISTERED_TAG, account_registered_query_body()),
        ] {
            match body {
                Ok(body) => {
                    let tag = JsValue::from_str(tag);
                    match consumer::subscribe(&consumer, &body, Some(&tag)) {
                        Ok(sub) => subscriptions.borrow_mut().push(sub),
                        Err(err) => {
                            // Dispatch failure: the one-shot roster read
                            // still painted the label; only liveness is
                            // lost.
                            tonk_common::log!("ui-hub-account: subscribe failed: {err:?}");
                        }
                    }
                }
                Err(err) => tonk_common::log!("ui-hub-account: query build failed: {err}"),
            }
        }
    });
}

/// The `tag` a subscription frame was addressed with.
fn frame_tag(opts: &JsValue) -> Option<String> {
    Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
}

/// The subscribe body for the account registration stamp, in directory mode.
///
/// Presence is the answer: the row is asserted the moment an account links
/// (`record_customer_status` at enroll), long before activation, so its
/// delta is the earliest "signed in" signal the profile branch carries.
fn account_registered_query_body() -> Result<JsValue, String> {
    let body = r#"{
      "predicate": { "with": { "registered_at": {
        "the": "xyz.tonk.account/registered-at", "as": "UnsignedInteger", "cardinality": "one"
      } } },
      "terms": { "this": { "?": { "name": "this" } }, "registered_at": { "?": { "name": "registered_at" } } }
    }"#;
    JSON::parse(body).map_err(|e| format!("query JSON parse: {e:?}"))
}

/// The subscribe body for the account display name, in directory mode
/// (`this` unbound — the profile branch carries at most one such row, and it
/// is keyed by the account subject, which this element does not know).
///
/// An inline predicate over the raw `xyz.tonk.account/display-name`
/// attribute, mirroring the FAB's `profile_name_query_body` — but over the
/// ACCOUNT name, because that row exists only once an account is linked:
/// its very presence is the "signed in" signal, and `converge_account_state`
/// keeps its value the shown name.
fn account_name_query_body() -> Result<JsValue, String> {
    let body = r#"{
      "predicate": { "with": { "name": {
        "the": "xyz.tonk.account/display-name", "as": "Text", "cardinality": "one"
      } } },
      "terms": { "this": { "?": { "name": "this" } }, "name": { "?": { "name": "name" } } }
    }"#;
    JSON::parse(body).map_err(|e| format!("query JSON parse: {e:?}"))
}

/// A subscription snapshot frame: the first conclusion's `name`. `None`
/// (no account linked yet) leaves the label at what the roster read
/// painted — an empty frame must not knock a linked label back.
fn read_name_from_frame(payload: &JsValue) -> Option<String> {
    let conclusions = js_sys::Array::from(payload);
    read_name_field(&conclusions.get(0))
}

/// An incremental `update` frame: `{ asserted, retracted }`. `name` is
/// cardinality-one, so the newest asserted row carries the current value; a
/// bare retract leaves the label where it is.
fn read_name_from_delta(payload: &JsValue) -> Option<String> {
    let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    read_name_field(&rows.get(rows.length().saturating_sub(1)))
}

/// Read `conclusion.fields.name` off a raw subscription row.
fn read_name_field(row: &JsValue) -> Option<String> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    Reflect::get(row, &"fields".into())
        .ok()
        .and_then(|fields| Reflect::get(&fields, &"name".into()).ok())
        .and_then(|v| v.as_string())
        .filter(|name| !name.trim().is_empty())
}

/// A live account name arrived: the profile is linked. Paint the name and
/// give the trigger its menu affordance back.
fn apply_account_name(this: &HtmlElement, name: &str) {
    set_text(this, "[data-account-label]", name);
    let _ = this.set_attribute("data-active-provider", "true");
    set_trigger_mode(this, false);
}

/// Enter the linking state: the account tab activates and the spaces stack
/// steps aside while the top page runs the ceremony seated under the bar.
/// The dialog's dismissal restores focus to the trigger, which is the
/// signal to put the page back (the `focusin` listener in
/// `connected_callback`).
fn enter_linking(this: &HtmlElement) {
    let _ = this.set_attribute("data-linking", "");
    if let Some(trigger) = account_trigger(this) {
        let _ = trigger.set_attribute("aria-current", "page");
    }
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
}

/// Put the page back after the ceremony settles (or is dismissed).
///
/// Not [`close_menu`]: that restores only from an OPEN menu (it keys on
/// `aria-expanded`, which the link-mode trigger does not carry).
fn leave_linking(this: &HtmlElement) {
    if !this.has_attribute("data-linking") {
        return;
    }
    let _ = this.remove_attribute("data-linking");
    if let Some(trigger) = account_trigger(this) {
        let _ = trigger.remove_attribute("aria-current");
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

/// Whether the settings section is the visible page under the bar.
fn settings_open(this: &HtmlElement) -> bool {
    this.query_selector("[data-settings-view]")
        .ok()
        .flatten()
        .and_then(|view| view.dyn_into::<HtmlElement>().ok())
        .is_some_and(|view| !view.hidden())
}

/// Swap between the account rows and the settings section — both are pages
/// of the account TAB, so the bar (and the trigger's current mark) stays.
fn show_settings(this: &HtmlElement, settings: bool) {
    if let Ok(Some(menu)) = this.query_selector("[data-account-menu]")
        && let Ok(menu) = menu.dyn_into::<HtmlElement>()
    {
        menu.set_hidden(settings);
    }
    if let Ok(Some(view)) = this.query_selector("[data-settings-view]")
        && let Ok(view) = view.dyn_into::<HtmlElement>()
    {
        view.set_hidden(!settings);
    }
}

/// Open the in-column settings section — a page of the account tab.
fn open_settings_view(this: &HtmlElement) {
    // Settings is a page of the account tab: make sure that tab's frame
    // (spaces stack aside, trigger current) is up before swapping to it.
    let expanded = account_trigger(this)
        .and_then(|trigger| trigger.get_attribute("aria-expanded"))
        .as_deref()
        == Some("true");
    if !expanded {
        open_menu(this);
    }
    show_settings(this, true);
    // The shared panel fills its own rows (see `ui_account_settings`).
    if let Ok(Some(panel)) = this.query_selector("ui-account-settings")
        && let Ok(panel) = panel.dyn_into::<HtmlElement>()
    {
        crate::ui_account_settings::refresh(&panel);
    }
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
    if let Ok(Some(view)) = this.query_selector("[data-settings-view]")
        && let Ok(view) = view.dyn_into::<HtmlElement>()
    {
        view.set_hidden(true);
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
        // Only a trigger that IS a menu button carries `aria-expanded`; in
        // the link-an-account mode `set_trigger_mode` stripped it, and this
        // must not stamp it back.
        if trigger.has_attribute("aria-haspopup") {
            let _ = trigger.set_attribute("aria-expanded", "false");
        }
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
    if let Ok(Some(view)) = this.query_selector("[data-settings-view]")
        && let Ok(view) = view.dyn_into::<HtmlElement>()
    {
        view.set_hidden(true);
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

/// Register `<ui-hub-account>`. Idempotent. Installs the prototype
/// `reset`/`update` method shims (forwarding to the per-instance
/// `__tonkReset`/`__tonkUpdate` delegates) so host subscription frames
/// reach the element — the same pattern `<ui-sync-status>` uses.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-hub-account").is_undefined() {
        UiHubAccount::define("ui-hub-account");
        install_frame_shims();
    }
}

/// Install `reset`/`update` on the element prototype, forwarding to the
/// per-instance delegates. On the prototype (not each instance) so
/// `this`-binding is correct.
fn install_frame_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("ui-hub-account");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = js_sys::Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let update_fn = js_sys::Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
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
    fn it_mounts_the_complete_header_with_settings_folded_away() {
        let host = account_element();

        assert!(
            host.query_selector("[data-account-menu]")
                .expect("valid selector")
                .is_some(),
            "the registered element must mount its account menu"
        );
        let settings: HtmlElement = host
            .query_selector("[data-settings-view]")
            .expect("valid selector")
            .expect("settings is a page of the account tab now")
            .dyn_into()
            .unwrap();
        assert!(settings.hidden(), "and it stays folded until asked for");
        assert_eq!(
            host.query_selector_all(".hubbar > *").unwrap().length(),
            2,
            "two cells: account and spaces — settings lives in the account view"
        );
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
        assert!(
            host.query_selector("[data-profile=\"primary\"]")
                .unwrap()
                .is_none(),
            "the current account is the tab itself, not a row"
        );
        let switches = host.query_selector_all("button[data-profile]").unwrap();
        assert_eq!(switches.length(), 2);
        let second = switches.item(0).unwrap().text_content().unwrap();
        assert!(second.contains("grace@example.com"));
        assert!(
            second.contains("switch account"),
            "a roster row carries its switch verb"
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
        assert!(
            host.query_selector("[data-account-menu] a[data-open-settings]")
                .unwrap()
                .is_some(),
            "settings is the account view's first way onward"
        );
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
        assert!(
            document
                .active_element()
                .unwrap()
                .has_attribute("data-open-settings"),
            "the settings row leads the account view"
        );

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
        assert!(
            document
                .active_element()
                .unwrap()
                .has_attribute("data-open-settings")
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
        assert!(
            document
                .active_element()
                .unwrap()
                .has_attribute("data-open-settings")
        );
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
        assert!(
            account.get_attribute("aria-haspopup").is_none(),
            "the link-an-account trigger is a plain action, not a menu button"
        );
        assert!(account.get_attribute("aria-expanded").is_none());
        assert!(account.get_attribute("aria-controls").is_none());
        assert!(
            account.query_selector(".g").unwrap().is_none(),
            "the account cell never draws a dropdown caret"
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
        assert!(
            account.get_attribute("aria-expanded").is_none(),
            "the close-menu pass must not stamp menu ARIA back onto a plain action trigger"
        );
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
            window().unwrap().location().href().unwrap(),
            original_url,
            "settings opens inside the hub, not on a page of its own"
        );
        let settings_view: HtmlElement = host
            .query_selector("[data-settings-view]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!settings_view.hidden(), "the settings section swaps in");
        assert!(stack.hidden(), "the spaces stack steps aside for it");
        let escape = KeyboardEventInit::new();
        escape.set_key("Escape");
        escape.set_bubbles(true);
        host.query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dispatch_event(
                &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &escape).unwrap(),
            )
            .unwrap();
        assert!(settings_view.hidden(), "Escape goes home");
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

    /// A live `xyz.tonk.account/display-name` frame is the login signal:
    /// it must flip the unlinked trigger to the member's name — and give
    /// it its menu affordance back — without any reload.
    #[wasm_bindgen_test]
    fn it_adopts_a_live_account_name_frame_without_a_reload() {
        let host = account_element();
        super::render_profiles(
            &host,
            &ProfilesResponse {
                active: "Local workspace".into(),
                profiles: vec![profile("Local workspace", None, None, None, true)],
            },
        );
        let trigger: HtmlElement = host
            .query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(trigger.get_attribute("aria-haspopup").is_none());

        // The shape a subscription `reset` frame delivers: conclusions with
        // a `fields.name`.
        let frame = js_sys::JSON::parse(r#"[{ "fields": { "name": "Ada Lovelace" } }]"#).unwrap();
        if let Some(name) = super::read_name_from_frame(&frame) {
            super::apply_account_name(&host, &name);
        }

        let label = host
            .query_selector("[data-account-label]")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap_or_default();
        assert_eq!(label, "Ada Lovelace");
        assert_eq!(
            host.get_attribute("data-active-provider").as_deref(),
            Some("true")
        );
        assert_eq!(
            trigger.get_attribute("aria-haspopup").as_deref(),
            Some("menu")
        );
        assert!(
            trigger.query_selector(".g").unwrap().is_none(),
            "linking brings the menu back, not a dropdown caret"
        );

        // A rename arrives as an `update` delta; the newest asserted row wins.
        let delta = js_sys::JSON::parse(
            r#"{ "asserted": [{ "fields": { "name": "Countess Lovelace" } }], "retracted": [] }"#,
        )
        .unwrap();
        assert_eq!(
            super::read_name_from_delta(&delta).as_deref(),
            Some("Countess Lovelace")
        );
        // A bare retract or an empty snapshot must leave the label alone.
        let bare_retract = js_sys::JSON::parse(r#"{ "asserted": [], "retracted": [{}] }"#).unwrap();
        assert!(super::read_name_from_delta(&bare_retract).is_none());
        let empty = js_sys::JSON::parse("[]").unwrap();
        assert!(super::read_name_from_frame(&empty).is_none());
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_opens_settings_inside_the_hub() {
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
        // Still an anchor: `/settings` is the no-handler fallback, but a
        // handled click stays on this page and raises the dialog instead.
        assert_eq!(settings_button.tag_name(), "A");
        assert_eq!(
            settings_button.get_attribute("href").as_deref(),
            Some("/settings")
        );
        assert!(
            host.query_selector("[data-settings-view]")
                .unwrap()
                .is_some(),
            "the settings section rides the hub markup"
        );
        let original_url = window().unwrap().location().href().unwrap();
        settings_button.click();
        assert_eq!(
            window().unwrap().location().href().unwrap(),
            original_url,
            "settings must not navigate the hub anywhere"
        );
        let settings_view: HtmlElement = host
            .query_selector("[data-settings-view]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(!settings_view.hidden(), "the settings section swaps in");
        assert!(stack.hidden(), "the spaces stack steps aside for it");
        // The rail: devices is a pane switch, and the account tab leads
        // back to the account rows.
        host.query_selector(".s-rail [data-pane=\"devices\"]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert!(
            !host
                .query_selector(".s-body [data-pane=\"devices\"]")
                .unwrap()
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap()
                .hidden()
        );
        let menu: HtmlElement = host
            .query_selector("[data-account-menu]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(menu.hidden(), "the account rows stepped aside for settings");
        host.query_selector("[data-account-trigger]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert!(
            settings_view.hidden() && !menu.hidden(),
            "the account tab leads from settings back to the account rows"
        );
        hubcol.remove();
    }
}
