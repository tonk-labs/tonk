//! `<ui-account-settings>` — the account settings panel over live account
//! facts.
//!
//! One panel, two seats. The Hub's account tab embeds it as the in-column
//! settings page; the space route mounts it inside a `<tonk-dialog>` the
//! FAB's `settings` row raises. It injects its own markup and fills the
//! rows imperatively because the values are API reads, not branch facts:
//! the address is service-owned by design (the uniqueness key is never
//! mirrored into the account repository), so a declarative view has nothing
//! to subscribe to.
//! The display name commits imperatively too — `POST
//! /api/account/display-name`, the same worker route the /account page
//! uses — because event-to-command delegation belongs to `tonk-display`
//! templates, and this panel's markup is injected after preprocessing.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, JSON, Reflect};
use tonk_host::consumer::{self, Subscription};
use tonk_schema::{ceremony, ceremony_state};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, window};

type EventClosure = Closure<dyn FnMut(Event)>;
type FrameClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// The routing context the ceremony status lives in: the profile branch.
const PROFILE_WITH: &str = "main@profile:tonk";
/// The tag the ceremony-status subscription's frames arrive under.
const CEREMONY_TAG: &str = "ui-account-settings:ceremony";
/// The email row, read as a fact rather than fetched.
const ACCOUNT_TAG: &str = "ui-account-settings:account";
/// The passkey rows, likewise.
const PASSKEY_TAG: &str = "ui-account-settings:passkeys";
const DELETE_ACCOUNT_CONFIRMATION: &str = "delete account";

fn set_text(this: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = this.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

#[derive(Default)]
struct UiAccountSettings {
    custody_opened: Option<EventClosure>,
    custody_closed: Option<EventClosure>,
    position_change: Option<EventClosure>,
    position_observer: Option<web_sys::ResizeObserver>,
    position_callback: Option<FrameClosure>,
    click: Option<EventClosure>,
    change: Option<EventClosure>,
    keydown: Option<EventClosure>,
    input: Option<EventClosure>,
    dialog_open: Option<EventClosure>,
    /// The live ceremony-status subscription, held while connected.
    subscription: Rc<RefCell<Option<Subscription>>>,
    /// The account facts the email row renders.
    account_subscription: Rc<RefCell<Option<Subscription>>>,
    /// The passkey rows the panel lists.
    passkey_subscription: Rc<RefCell<Option<Subscription>>>,
    /// The frame delegates the host calls by name off the element.
    frames: Vec<FrameClosure>,
}

impl CustomElement for UiAccountSettings {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        if this.query_selector(".s-body").ok().flatten().is_none() {
            this.set_inner_html(include_str!("ui_account_settings.html"));
        }
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        if self.click.is_some() {
            return;
        }

        let host = this.clone();
        let click: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            let hit = |selector: &str| target.closest(selector).ok().flatten().is_some();
            if hit("[data-delete-account-open]") {
                open_delete_dialog(&host);
            } else if hit("[data-delete-account-submit]") {
                submit_delete(&host);
            } else if hit("[data-sign-out-open]") {
                show_dialog(&host, "[data-sign-out-dialog]");
            } else if hit("[data-sign-out-submit]") {
                sign_out(&host);
            } else if hit("[data-add-passkey]") {
                add_passkey(&host);
            } else if hit("[data-link-approve]") {
                approve_link(&host);
            } else if hit("[data-link-decline]") {
                decline_link(&host);
            }
        }));
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        self.click = Some(click);

        // The display name saves on commit (change = Enter or blur). The
        // roster subscription repaints the bar's account cell when the
        // write lands, which is the visible receipt.
        let host = this.clone();
        let change: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let Some(input) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                .filter(|input| input.has_attribute("data-settings-name"))
            else {
                return;
            };
            let name = input.value();
            if name.trim().is_empty() {
                prefill_name(&host);
                return;
            }
            spawn_local(async move {
                let body = serde_json::json!({ "name": name }).to_string();
                if let Err(error) = tonk_host::post_json("/api/account/display-name", &body).await {
                    tonk_common::log!("settings: display-name save failed: {error:?}");
                }
            });
        }));
        let _ = this.add_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        self.change = Some(change);

        // A plain text input does not commit on Enter by itself. End the edit
        // so the browser emits the same `change` event as a pointer blur and
        // the one save path above handles both gestures.
        let host_enter = this.clone();
        let keydown: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            if key.key() != "Enter" {
                return;
            }
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlElement>().ok())
            else {
                return;
            };
            // Enter in the arming field submits when armed, and never
            // breaks the line.
            if target.has_attribute("data-delete-confirm") {
                key.prevent_default();
                submit_delete(&host_enter);
                return;
            }
            let Some(input) = target
                .dyn_into::<HtmlInputElement>()
                .ok()
                .filter(|input| input.has_attribute("data-settings-name"))
            else {
                return;
            };
            key.prevent_default();
            let _ = input.blur();
        }));
        let _ = this.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        self.keydown = Some(keydown);
        // Every keystroke in the arming field re-judges the verb.
        let host = this.clone();
        let input: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let in_field = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| target.has_attribute("data-delete-confirm"));
            if in_field {
                arm_delete(&host);
            }
        }));
        let _ = this.add_event_listener_with_callback("input", input.as_ref().unchecked_ref());
        self.input = Some(input);

        // A `<tonk-dialog>` seat re-raises this panel long after connect;
        // the dialog's own `fabb-open` is the "fill it freshly" signal. The
        // event is composed and bubbles to the document, so one listener
        // covers whichever dialog this instance sits in.
        let host = this.clone();
        let dialog_open: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let reopened = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                .is_some_and(|target| target.contains(Some(&host)));
            if reopened {
                refresh(&host);
            }
        }));
        if let Some(document) = window().and_then(|window| window.document()) {
            let _ = document.add_event_listener_with_callback(
                "fabb-open",
                dialog_open.as_ref().unchecked_ref(),
            );
        }
        self.dialog_open = Some(dialog_open);
        // The ceremony status is a row on the profile overlay; the host
        // calls `reset` (snapshot) and `update` (delta) by name off this
        // element for every frame, so both delegates hang off it.
        let host = this.clone();
        let reset: FrameClosure = Closure::wrap(Box::new(
            move |payload: JsValue, opts: JsValue| match frame_tag(&opts).as_deref() {
                Some(ACCOUNT_TAG) => on_account_snapshot(&host, payload),
                Some(PASSKEY_TAG) => render_passkeys(&host, &js_sys::Array::from(&payload)),
                _ => on_ceremony_snapshot(&host, payload),
            },
        ));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        let host = this.clone();
        let update: FrameClosure = Closure::wrap(Box::new(
            move |payload: JsValue, opts: JsValue| match frame_tag(&opts).as_deref() {
                Some(ACCOUNT_TAG) => on_account_delta(&host, payload),
                Some(PASSKEY_TAG) => {
                    let asserted =
                        Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
                    render_passkeys(&host, &js_sys::Array::from(&asserted));
                }
                _ => on_ceremony_delta(&host, payload),
            },
        ));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        self.frames = vec![reset, update];
        subscribe_ceremony(this, self.subscription.clone());
        subscribe_account(this, self.account_subscription.clone());
        subscribe_passkeys(this, self.passkey_subscription.clone());

        refresh(this);
        let host = this.clone();
        let opened: EventClosure = Closure::wrap(Box::new(move |_: Event| {
            if host.has_attribute("data-passkey-screen") {
                return;
            }
            let _ = host.set_attribute("data-passkey-screen", "");
            publish_custody_seat(&host);
        }));
        if let Some(window) = window() {
            let _ = window.add_event_listener_with_callback(
                "tonk:custody-opened",
                opened.as_ref().unchecked_ref(),
            );
        }
        self.custody_opened = Some(opened);
        let host = this.clone();
        let closed: EventClosure = Closure::wrap(Box::new(move |_: Event| {
            let _ = host.remove_attribute("data-passkey-screen");
            let _ = host.remove_attribute("data-passkey-requested");
            // Closing the passkey UI is not a new command result. The worker
            // may already have published its refusal while this screen hid it.
            if !matches!(
                host.get_attribute("data-ceremony-state").as_deref(),
                Some(ceremony_state::REFUSED | ceremony_state::FAILED | ceremony_state::DONE)
            ) {
                show_status(&host, "");
            }
        }));
        if let Some(window) = window() {
            let _ = window.add_event_listener_with_callback(
                "tonk:custody-closed",
                closed.as_ref().unchecked_ref(),
            );
        }
        self.custody_closed = Some(closed);
        let host = this.clone();
        let position: EventClosure = Closure::wrap(Box::new(move |_: Event| {
            publish_custody_seat(&host);
        }));
        if let Some(window) = window() {
            for event in ["scroll", "resize"] {
                let _ = window.add_event_listener_with_callback_and_bool(
                    event,
                    position.as_ref().unchecked_ref(),
                    true,
                );
            }
        }
        self.position_change = Some(position);
        let host = this.clone();
        let callback: FrameClosure = Closure::wrap(Box::new(move |_, _| {
            publish_custody_seat(&host);
        }));
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(this);
            self.position_observer = Some(observer);
            self.position_callback = Some(callback);
        }
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(opened) = self.custody_opened.take()
            && let Some(window) = window()
        {
            let _ = window.remove_event_listener_with_callback(
                "tonk:custody-opened",
                opened.as_ref().unchecked_ref(),
            );
        }
        if let Some(closed) = self.custody_closed.take()
            && let Some(window) = window()
        {
            let _ = window.remove_event_listener_with_callback(
                "tonk:custody-closed",
                closed.as_ref().unchecked_ref(),
            );
        }
        if let Some(observer) = self.position_observer.take() {
            observer.disconnect();
        }
        self.position_callback.take();
        if let Some(position) = self.position_change.take()
            && let Some(window) = window()
        {
            for event in ["scroll", "resize"] {
                let _ = window.remove_event_listener_with_callback_and_bool(
                    event,
                    position.as_ref().unchecked_ref(),
                    true,
                );
            }
        }
        self.subscription.borrow_mut().take();
        self.frames.clear();
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        if let Some(change) = self.change.take() {
            let _ =
                this.remove_event_listener_with_callback("change", change.as_ref().unchecked_ref());
        }
        if let Some(keydown) = self.keydown.take() {
            let _ = this
                .remove_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        }
        if let Some(input) = self.input.take() {
            let _ =
                this.remove_event_listener_with_callback("input", input.as_ref().unchecked_ref());
        }
        if let Some(dialog_open) = self.dialog_open.take()
            && let Some(document) = window().and_then(|window| window.document())
        {
            let _ = document.remove_event_listener_with_callback(
                "fabb-open",
                dialog_open.as_ref().unchecked_ref(),
            );
        }
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

/// Show one account-flow pane.
fn set_pane(this: &HtmlElement, pane: &str) {
    if let Ok(panes) = this.query_selector_all(".s-body .pane") {
        for index in 0..panes.length() {
            if let Some(section) = panes
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            {
                section.set_hidden(section.get_attribute("data-pane").as_deref() != Some(pane));
            }
        }
    }
}

/// Fill every pane from live state, landing on the account pane.
///
/// The rows load AFTER the panel appears — a view that shows instantly and
/// fills in beats one that waits on a fetch.
pub(crate) fn refresh(this: &HtmlElement) {
    // `/settings/link?audience=&callback=&name=` is a terminal asking
    // for access. Every other settings URL lands on the account pane.
    match link_request() {
        Some(request) => {
            set_text(this, "[data-link-name]", &request.name);
            set_text(this, "[data-link-did]", &request.audience);
            set_text(
                this,
                "[data-link-account]",
                request
                    .expected_account
                    .as_deref()
                    .unwrap_or("your signed-in account"),
            );
            set_pane(this, "link");
        }
        None => {
            let location = page_location();
            set_pane(this, "account");
            // `tonk account delete` and `tonk account spots delete` open
            // this page with the review already asked for.
            if location.hash == "#delete-account" {
                open_delete_dialog(this);
            }
        }
    }
    prefill_name(this);
}

/// The one space `?delete-space=` names, when this page was opened to
/// delete one owned hosted space rather than the account.
fn requested_space_deletion() -> Option<String> {
    let location = page_location();
    let params = web_sys::UrlSearchParams::new_with_str(&location.search).ok()?;
    params
        .get("delete-space")
        .filter(|subject| !subject.trim().is_empty())
}

/// The page's real location, as the host forwards it into the guest.
///
/// A sealed guest's own `window.location` is `about:srcdoc`; the host
/// injects the real one into `window.tonk.context`. The top-page seat
/// (tests) falls back to `window.location`.
struct PageLocation {
    origin: String,
    path: String,
    search: String,
    hash: String,
}

fn page_location() -> PageLocation {
    let context = window()
        .and_then(|win| Reflect::get(&win, &"tonk".into()).ok())
        .and_then(|tonk| Reflect::get(&tonk, &"context".into()).ok())
        .filter(|context| !context.is_undefined() && !context.is_null());
    let field = |key: &str| -> Option<String> {
        Reflect::get(context.as_ref()?, &key.into())
            .ok()
            .and_then(|value| value.as_string())
    };
    match field("origin").filter(|origin| !origin.is_empty()) {
        Some(origin) => PageLocation {
            origin,
            path: field("path").unwrap_or_default(),
            search: field("search").unwrap_or_default(),
            hash: field("hash").unwrap_or_default(),
        },
        None => {
            let location = window().map(|win| win.location());
            let read = |value: Option<Result<String, JsValue>>| value.and_then(Result::ok);
            PageLocation {
                origin: read(location.as_ref().map(|l| l.origin())).unwrap_or_default(),
                path: read(location.as_ref().map(|l| l.pathname())).unwrap_or_default(),
                search: read(location.as_ref().map(|l| l.search())).unwrap_or_default(),
                hash: read(location.as_ref().map(|l| l.hash())).unwrap_or_default(),
            }
        }
    }
}

/// What a waiting terminal asked for, when this is its approval page.
struct LinkRequest {
    expected_account: Option<String>,
    audience: String,
    callback: String,
    name: String,
}

fn link_request() -> Option<LinkRequest> {
    let location = page_location();
    if location.path != "/settings/link" {
        return None;
    }
    let params = web_sys::UrlSearchParams::new_with_str(&location.search).ok()?;
    let audience = params.get("audience")?;
    let callback = params.get("callback")?;
    let name = params
        .get("name")
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "terminal".to_string());
    Some(LinkRequest {
        expected_account: params.get("expectedAccount"),
        audience,
        callback,
        name,
    })
}

/// Raise one of this panel's `<tonk-dialog>` clusters, the confirm.
fn show_dialog(this: &HtmlElement, selector: &str) {
    let Some(dialog) = this.query_selector(selector).ok().flatten() else {
        return;
    };
    if let Some(show) = Reflect::get(dialog.as_ref(), &"show".into())
        .ok()
        .and_then(|show| show.dyn_into::<Function>().ok())
    {
        let _ = show.call0(dialog.as_ref());
    }
}

fn close_dialog(this: &HtmlElement, selector: &str) {
    let Some(dialog) = this.query_selector(selector).ok().flatten() else {
        return;
    };
    if let Some(close) = Reflect::get(dialog.as_ref(), &"close".into())
        .ok()
        .and_then(|close| close.dyn_into::<Function>().ok())
    {
        let _ = close.call0(dialog.as_ref());
    }
}

/// What the destructive confirmation field holds.
fn typed_confirmation(this: &HtmlElement) -> String {
    this.query_selector("[data-delete-confirm]")
        .ok()
        .flatten()
        .and_then(|field| field.dyn_into::<HtmlInputElement>().ok())
        .map(|field| field.value())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Arm the solid verb only while the requested destructive phrase is exact.
fn arm_delete(this: &HtmlElement) {
    let expected = this
        .get_attribute("data-delete-confirm-expected")
        .unwrap_or_default();
    let armed = !expected.is_empty() && typed_confirmation(this) == expected;
    if let Ok(Some(verb)) = this.query_selector("[data-delete-account-submit]") {
        if armed {
            let _ = verb.remove_attribute("disabled");
        } else {
            let _ = verb.set_attribute("disabled", "");
        }
    }
}

fn set_hidden(this: &HtmlElement, selector: &str, hidden: bool) {
    if let Ok(Some(element)) = this.query_selector(selector)
        && let Ok(element) = element.dyn_into::<HtmlElement>()
    {
        element.set_hidden(hidden);
    }
}

/// Show the reviewed scope, then the confirmation.
///
/// The plan is read from the account db by the worker: which listed
/// spaces this account provides, and how many it merely joined.
fn open_delete_dialog(this: &HtmlElement) {
    let requested = requested_space_deletion();
    let deleting_space = requested.is_some();
    let confirmation = if deleting_space {
        "delete space"
    } else {
        DELETE_ACCOUNT_CONFIRMATION
    };
    set_text(this, "[data-delete-confirm-label]", confirmation);
    set_text(
        this,
        "[data-delete-submit-label]",
        if deleting_space {
            "delete space"
        } else {
            "delete account"
        },
    );
    set_text(
        this,
        "[data-delete-scope]",
        "loading what this deletes\u{2026}",
    );
    if let Ok(Some(dialog)) = this.query_selector("[data-delete-account-dialog]") {
        let _ = dialog.set_attribute(
            "heading",
            if deleting_space {
                "delete this space?"
            } else {
                "confirm account deletion"
            },
        );
    }
    set_text(
        this,
        "[data-delete-question]",
        if deleting_space {
            "Everyone will lose access to this space through Tonk. Your account and other spaces will stay."
        } else {
            "are you sure you want to delete all data associated with this account?"
        },
    );
    set_text(
        this,
        "[data-delete-consequence]",
        if deleting_space {
            "This cannot be undone. Copies saved on other devices may remain, but they will no longer sync."
        } else {
            "this action is permanent. there is no option to recover your data."
        },
    );
    set_text(
        this,
        "[data-delete-passkey]",
        if deleting_space {
            ""
        } else {
            "your passkey will be asked for."
        },
    );
    if let Ok(Some(field)) = this.query_selector("[data-delete-confirm]")
        && let Ok(field) = field.dyn_into::<HtmlInputElement>()
    {
        field.set_value("");
    }
    let _ = this.remove_attribute("data-delete-confirm-expected");
    let _ = this.remove_attribute("data-delete-account-email");
    let _ = this.remove_attribute("data-delete-space");
    arm_delete(this);
    show_dialog(this, "[data-delete-account-dialog]");
    if let Ok(Some(field)) = this.query_selector("[data-delete-confirm]")
        && let Ok(field) = field.dyn_into::<HtmlElement>()
    {
        let _ = field.focus();
    }
    let host = this.clone();
    spawn_local(async move {
        let plan: Option<tonk_worker_api::AccountDeletionPlan> =
            match tonk_host::get_json("/api/account/deletion/plan").await {
                Ok(body) => serde_json::from_str(&body).ok(),
                Err(_) => None,
            };
        let Some(plan) = plan else {
            set_text(
                &host,
                "[data-delete-scope]",
                "The deletion scope could not be loaded. Check your connection and try again.",
            );
            return;
        };
        let spaces: Vec<_> = plan
            .spaces
            .iter()
            .filter(|space| {
                requested
                    .as_deref()
                    .is_none_or(|subject| space.subject == subject)
            })
            .collect();
        if requested.is_some() && spaces.is_empty() {
            set_text(
                &host,
                "[data-delete-scope]",
                "This space cannot be deleted from this account. Go back to your spaces and check that you are signed into the account that owns it.",
            );
            return;
        }
        let _ = host.set_attribute(
            "data-delete-space",
            requested.as_deref().unwrap_or_default(),
        );
        let owned = spaces.len();
        let names: Vec<&str> = spaces
            .iter()
            .map(|space| {
                space
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("unnamed space")
            })
            .collect();
        let listed = if names.is_empty() {
            String::new()
        } else {
            format!(": {}", names.join(", "))
        };
        let confirmation = if requested.is_some() {
            "delete space".to_owned()
        } else {
            DELETE_ACCOUNT_CONFIRMATION.to_owned()
        };
        set_text(&host, "[data-delete-confirm-label]", &confirmation);
        set_text(
            &host,
            "[data-delete-scope]",
            &if requested.is_some() {
                format!("Permanently delete {} from Tonk?", names[0])
            } else {
                format!(
                    "{owned} owned hosted space{} will be deleted{listed}. {} joined space{} will be left intact.",
                    if owned == 1 { "" } else { "s" },
                    plan.joined_spaces,
                    if plan.joined_spaces == 1 { "" } else { "s" },
                )
            },
        );
        let _ = host.set_attribute("data-delete-account-email", &plan.email);
        let _ = host.set_attribute("data-delete-confirm-expected", &confirmation);
        arm_delete(&host);
    });
}

/// Assert `tonk:delete-account`. The worker checks the address against
/// the account, asks the page for the passkey, and reports through the
/// ceremony row this panel watches.
fn submit_delete(this: &HtmlElement) {
    let confirmation = typed_confirmation(this);
    let expected_confirmation = this
        .get_attribute("data-delete-confirm-expected")
        .unwrap_or_default();
    // The verb is off until the phrase matches; a submit that arrives
    // anyway (keyboard, script) is answered the same way.
    if expected_confirmation.is_empty() || confirmation != expected_confirmation {
        arm_delete(this);
        return;
    }
    close_dialog(this, "[data-delete-account-dialog]");
    if let Some(subject) = this
        .get_attribute("data-delete-space")
        .filter(|subject| !subject.is_empty())
    {
        // One hosted space is deprovisioning: the worker signs
        // `/provider/remove` with this device's own authority, and no
        // passkey is involved.
        show_status(this, "Deleting the selected space\u{2026}");
        let host = this.clone();
        spawn_local(async move {
            let body = serde_json::json!({ "subject": subject }).to_string();
            match tonk_host::post_json("/api/account/spaces/delete", &body).await {
                Ok(_) => show_status(
                    &host,
                    "Owned space deleted from Tonk services. Your account and other spaces remain.",
                ),
                Err(error) => show_status(
                    &host,
                    &format!("The space was not deleted: {}", error.message),
                ),
            }
        });
        return;
    }
    let Some(email) = this.get_attribute("data-delete-account-email") else {
        arm_delete(this);
        return;
    };
    show_status(this, "Waiting for your passkey\u{2026}");
    transact(
        this,
        &claim(
            "Delete this account from every service and this device.",
            serde_json::json!({
                "email": { "the": "xyz.tonk.delete-account/email", "as": "Text" }
            }),
            serde_json::json!({ "email": email }),
        ),
    );
}

/// Sign this device out: the account stays, this browser forgets it.
fn sign_out(this: &HtmlElement) {
    close_dialog(this, "[data-sign-out-dialog]");
    show_status(this, "Signing out\u{2026}");
    let host = this.clone();
    spawn_local(async move {
        match tonk_host::delete_json("/api/account").await {
            // The worker's whole state changed hands; rebuilding the
            // page is what drops the subscriptions the old account owned.
            Ok(_) => tonk_host::reload_page(),
            Err(error) => show_status(
                &host,
                &format!("This device could not be signed out: {}", error.message),
            ),
        }
    });
}

/// Assert `tonk:add-passkey`: the worker asks the page for the passkey
/// that holds the account, then for the new one.
fn add_passkey(this: &HtmlElement) {
    show_status(this, "Waiting for your passkey\u{2026}");
    transact(
        this,
        &claim(
            "Seal the account under another passkey.",
            serde_json::json!({
                "account": { "the": "xyz.tonk.command.add-passkey/account", "as": "Entity" }
            }),
            serde_json::json!({ "account": "tonk:add-passkey" }),
        ),
    );
}

/// The passkey runs in the top document; reserve and publish its seat in
/// this sealed guest using the same page-effect relay as Hub registration.
fn publish_custody_seat(this: &HtmlElement) {
    let Some(seat) = this.query_selector("[data-custody-seat]").ok().flatten() else {
        return;
    };
    let rect = seat.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    tonk_host::request_registration(
        &serde_json::json!({
            "reason": "custody-anchor",
            "anchor": { "left": rect.left(), "bottom": rect.top() - 7.0, "width": rect.width() }
        })
        .to_string(),
    );
}

/// Assert `tonk:authorize-device` for the terminal named in the URL.
fn approve_link(this: &HtmlElement) {
    let Some(request) = link_request() else {
        return;
    };
    let _ = this.set_attribute("data-passkey-requested", "");
    show_status(this, "Waiting for your passkey\u{2026}");
    let mut fields = serde_json::json!({
        "audience": request.audience,
        "callback": bs58::encode(request.callback.as_bytes()).into_string(),
        "name": request.name,
    });
    let mut attributes = serde_json::json!({
        "audience": { "the": "xyz.tonk.authorize-device/audience", "as": "Entity" },
        "callback": { "the": "xyz.tonk.authorize-device/callback", "as": "Text" },
        "name": { "the": "xyz.tonk.authorize-device/name", "as": "Text" }
    });
    if let Some(expected) = request.expected_account {
        attributes["expectedAccount"] = serde_json::json!({ "the": "xyz.tonk.authorize-device/expected-account", "as": "Entity" });
        fields["expectedAccount"] = expected.into();
    }
    transact(
        this,
        &claim(
            "Delegate the account to a waiting terminal.",
            attributes,
            fields,
        ),
    );
}

/// Tell the waiting terminal no, and come back here.
fn decline_link(this: &HtmlElement) {
    let Some(request) = link_request() else {
        return;
    };
    let redirect = format!("{}/settings", page_location().origin);
    match tonk_worker_api::callback::delivery_url(
        &request.callback,
        &[("deny", "declined in the browser"), ("redirect", &redirect)],
    ) {
        Ok(target) => tonk_host::navigate_to(&target),
        Err(error) => show_status(this, &error),
    }
}

fn show_status(this: &HtmlElement, text: &str) {
    set_text(this, "[data-ceremony-status]", text);
    set_hidden(this, "[data-ceremony-status]", text.is_empty());
    publish_custody_seat(this);
}

/// A transient claim for `window.tonk.transact`: the concept inline,
/// so the worker decodes the same attributes the handler matches on.
fn claim(
    description: &str,
    with: serde_json::Value,
    parameters: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": { "description": description, "with": with }
                },
                "parameters": parameters
            }
        }]
    })
}

/// Call `window.tonk.transact(request)`: routeless, so the claim lands on
/// the profile branch this guest is mounted with. A refusal is shown
/// where the ask was made: a command that never committed reports
/// nothing else.
fn transact(this: &HtmlElement, request: &serde_json::Value) {
    let Ok(text) = serde_json::to_string(request) else {
        return;
    };
    let Some((tonk, transact)) = window()
        .and_then(|win| Reflect::get(&win, &"tonk".into()).ok())
        .and_then(|tonk| {
            Reflect::get(&tonk, &"transact".into())
                .ok()
                .and_then(|f| f.dyn_into::<Function>().ok())
                .map(|f| (tonk, f))
        })
    else {
        show_status(this, "This page cannot reach the worker.");
        return;
    };
    let Ok(body) = JSON::parse(&text) else {
        return;
    };
    let host = this.clone();
    spawn_local(async move {
        let outcome = match transact.call1(&tonk, &body) {
            Ok(answer) => match answer.dyn_into::<js_sys::Promise>() {
                Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise)
                    .await
                    .map(|_| ()),
                Err(_) => Ok(()),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = outcome {
            let reason = Reflect::get(&error, &"message".into())
                .ok()
                .and_then(|value| value.as_string())
                .or_else(|| error.as_string())
                .unwrap_or_else(|| format!("{error:?}"));
            tonk_common::log!("ui-account-settings: transact refused: {reason}");
            show_status(&host, &format!("The worker refused the request: {reason}"));
        }
    });
}

/// Subscribe to the ceremony-status row on the profile overlay.
fn subscribe_ceremony(this: &HtmlElement, subscription: Rc<RefCell<Option<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        if !host.is_connected() || subscription.borrow().is_some() {
            return;
        }
        if host.get_attribute("with").is_none() {
            let _ = host.set_attribute("with", PROFILE_WITH);
        }
        let consumer: Element = host.clone().into();
        let body = r#"{
          "predicate": { "with": {
            "ceremony": { "the": "xyz.tonk.ceremony/ceremony", "as": "Text", "cardinality": "one" },
            "state": { "the": "xyz.tonk.ceremony/state", "as": "Text", "cardinality": "one" },
            "detail": { "the": "xyz.tonk.ceremony/detail", "as": "Text", "cardinality": "one" }
          } },
          "terms": {
            "this": "state:ceremony",
            "ceremony": { "?": { "name": "ceremony" } },
            "state": { "?": { "name": "state" } },
            "detail": { "?": { "name": "detail" } }
          }
        }"#;
        let Ok(body) = JSON::parse(body) else {
            return;
        };
        let tag = JsValue::from_str(CEREMONY_TAG);
        match consumer::subscribe(&consumer, &body, Some(&tag)) {
            Ok(sub) => {
                tonk_common::log!("ui-account-settings: watching the ceremony status");
                *subscription.borrow_mut() = Some(sub)
            }
            Err(error) => tonk_common::log!("ui-account-settings: subscribe failed: {error:?}"),
        }
    });
}

/// Read a subscription frame's tag, so two subscriptions can share one
/// pair of delegates.
fn frame_tag(opts: &JsValue) -> Option<String> {
    Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
}

/// Watch the account facts the email and passkey rows render.
///
/// These were fetched from `/api/account/summary`, whose handler answers
/// by reading these very attributes off this very branch. A subscription
/// gets them without the round trip AND follows them: an address
/// enrolled or a passkey added on another device lands here rather than
/// waiting for the next mount.
fn subscribe_account(this: &HtmlElement, subscription: Rc<RefCell<Option<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        if !host.is_connected() || subscription.borrow().is_some() {
            return;
        }
        if host.get_attribute("with").is_none() {
            let _ = host.set_attribute("with", PROFILE_WITH);
        }
        let consumer: Element = host.clone().into();
        // Directory mode (`this` unbound), like the Hub cell's own name
        // subscription: this element does not know the account subject,
        // and asking the worker for it would be another round trip to
        // learn something the branch is about to tell us anyway.
        //
        // A browser that has held more than one account can carry more
        // than one row, and `render_account` takes the first. That is
        // the same exposure the Hub cell has had; binding the subject
        // here means threading it in, which is worth doing once for
        // both rather than differently in each.
        let body = r#"{
          "predicate": { "with": {
            "email": { "the": "xyz.tonk.account/customer-email", "as": "Text", "cardinality": "one" }
          } },
          "terms": {
            "this": { "?": { "name": "this" } },
            "email": { "?": { "name": "email" } }
          }
        }"#;
        let Ok(body) = JSON::parse(body) else {
            return;
        };
        let tag = JsValue::from_str(ACCOUNT_TAG);
        match consumer::subscribe(&consumer, &body, Some(&tag)) {
            Ok(sub) => *subscription.borrow_mut() = Some(sub),
            Err(error) => {
                tonk_common::log!("ui-account-settings: account subscribe failed: {error:?}")
            }
        }
    });
}

/// Watch the passkey rows the settings panel lists.
///
/// `RecoveryPasskey` carries everything shown — the creation label and
/// its timestamp — so this reads the concept directly.
///
/// The worker enumerates an account's passkeys by joining through the
/// `SecretMessage` whose sender is the account, because the passkey row
/// deliberately holds no second copy of the account it belongs to. That
/// join answers "whose passkey is this", which a DISPLAY on this branch
/// does not have to ask: passkey rows are written to the profile's own
/// branch beside the account's envelope, and a profile branch carries
/// one account. Every row here is this account's.
fn subscribe_passkeys(this: &HtmlElement, subscription: Rc<RefCell<Option<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        if !host.is_connected() || subscription.borrow().is_some() {
            return;
        }
        if host.get_attribute("with").is_none() {
            let _ = host.set_attribute("with", PROFILE_WITH);
        }
        let consumer: Element = host.clone().into();
        let body = r#"{
          "predicate": { "with": {
            "created_on": { "the": "xyz.tonk.recovery/created-on", "as": "Text", "cardinality": "one" },
            "created_at": { "the": "xyz.tonk.recovery/created-at", "as": "UnsignedInteger", "cardinality": "one" }
          } },
          "terms": {
            "this": { "?": { "name": "this" } },
            "created_on": { "?": { "name": "created_on" } },
            "created_at": { "?": { "name": "created_at" } }
          }
        }"#;
        let Ok(body) = JSON::parse(body) else {
            return;
        };
        let tag = JsValue::from_str(PASSKEY_TAG);
        match consumer::subscribe(&consumer, &body, Some(&tag)) {
            Ok(sub) => *subscription.borrow_mut() = Some(sub),
            Err(error) => {
                tonk_common::log!("ui-account-settings: passkey subscribe failed: {error:?}")
            }
        }
    });
}

/// List every passkey the account has, newest first.
///
/// Rows arrive unordered, so they are sorted here. The panel ships one
/// row of markup as its template; the rest are cloned from it, and the
/// whole list is rebuilt on each frame rather than diffed -- a handful
/// of passkeys is not worth reconciling.
fn render_passkeys(this: &HtmlElement, rows: &js_sys::Array) {
    let Some(first) = this
        .query_selector("[data-settings-passkey-device]")
        .ok()
        .flatten()
        .and_then(|device| device.parent_element())
    else {
        return;
    };
    let Some(parent) = first.parent_element() else {
        return;
    };

    let mut passkeys: Vec<(f64, String)> = Vec::new();
    for row in rows.iter() {
        let Ok(fields) = Reflect::get(&row, &"fields".into()) else {
            continue;
        };
        let created_on = Reflect::get(&fields, &"created_on".into())
            .ok()
            .and_then(|value| value.as_string());
        let created_at = Reflect::get(&fields, &"created_at".into())
            .ok()
            .and_then(|value| value.as_f64());
        if let (Some(on), Some(at)) = (created_on, created_at) {
            passkeys.push((at, on));
        }
    }
    passkeys.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Drop every row this rebuild replaces, keeping the first as the
    // template to clone from.
    while let Some(extra) = first.next_element_sibling().filter(|sibling| {
        sibling
            .query_selector("[data-settings-passkey-device]")
            .ok()
            .flatten()
            .is_some()
    }) {
        let _ = parent.remove_child(&extra);
    }

    if passkeys.is_empty() {
        set_text(this, "[data-settings-passkey-device]", "Unavailable");
        set_text(this, "[data-settings-passkey-created]", "");
        return;
    }

    for (index, (created_at, created_on)) in passkeys.iter().enumerate() {
        let row = if index == 0 {
            first.clone()
        } else {
            let Ok(clone) = first.clone_node_with_deep(true) else {
                break;
            };
            let Ok(clone) = clone.dyn_into::<Element>() else {
                break;
            };
            let _ = parent.insert_before(&clone, first.next_sibling().as_ref());
            clone
        };
        if let Ok(Some(device)) = row.query_selector("[data-settings-passkey-device]") {
            device.set_text_content(Some(created_on));
        }
        if let Ok(Some(created)) = row.query_selector("[data-settings-passkey-created]") {
            let date = js_sys::Date::new(&JsValue::from_f64(created_at * 1000.0))
                .to_locale_date_string("default", &JsValue::UNDEFINED);
            created.set_text_content(Some(&format!("created {}", String::from(date))));
        }
    }
}

/// A snapshot frame carrying the account row.
fn on_account_snapshot(this: &HtmlElement, payload: JsValue) {
    let rows = js_sys::Array::from(&payload);
    render_account(this, &rows.get(0));
}

/// A delta frame: the newest asserted row carries the current value.
fn on_account_delta(this: &HtmlElement, payload: JsValue) {
    let asserted = Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    if rows.length() > 0 {
        render_account(this, &rows.get(rows.length() - 1));
    }
}

/// Paint the email row from a subscription row.
///
/// An absent value leaves the row alone rather than blanking it: an
/// empty frame means the fact has not arrived, not that the account has
/// no address.
fn render_account(this: &HtmlElement, row: &JsValue) {
    let email = Reflect::get(row, &"fields".into())
        .ok()
        .and_then(|fields| Reflect::get(&fields, &"email".into()).ok())
        .and_then(|value| value.as_string())
        .filter(|email| !email.trim().is_empty());
    // Always writes: the markup ships "loading…" as its placeholder, so
    // returning early on an empty frame leaves that word on screen for
    // good. An account with no address recorded reads "Unavailable",
    // which is what the fetch this replaced said.
    set_text(
        this,
        "[data-settings-email]",
        email.as_deref().unwrap_or("Unavailable"),
    );
}

/// A snapshot frame: the row as it stands, or nothing yet.
fn on_ceremony_snapshot(this: &HtmlElement, payload: JsValue) {
    let rows = js_sys::Array::from(&payload);
    if rows.length() > 0 {
        render_ceremony(this, &rows.get(rows.length() - 1));
    }
}

/// A delta frame: `{ asserted, retracted }`, the newest asserted row wins.
fn on_ceremony_delta(this: &HtmlElement, payload: JsValue) {
    let asserted = Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    if rows.length() > 0 {
        render_ceremony(this, &rows.get(rows.length() - 1));
    }
}

/// Say where the ceremony got to, in words that say what to do next.
fn render_ceremony(this: &HtmlElement, row: &JsValue) {
    let field = |name: &str| {
        Reflect::get(row, &"fields".into())
            .ok()
            .and_then(|fields| Reflect::get(&fields, &name.into()).ok())
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    };
    let (which, state, detail) = (field("ceremony"), field("state"), field("detail"));
    tonk_common::log!("ui-account-settings: ceremony {which} {state} {detail}");
    let subject = match which.as_str() {
        ceremony::DELETE_ACCOUNT => "Deleting the account",
        ceremony::AUTHORIZE_DEVICE => "Approving the terminal",
        ceremony::ADD_PASSKEY => "Adding the passkey",
        _ => return,
    };
    let text = match state.as_str() {
        ceremony_state::PENDING_CEREMONY => format!("{subject}: waiting for your passkey\u{2026}"),
        ceremony_state::WORKING => format!("{subject}\u{2026}"),
        ceremony_state::DONE => match which.as_str() {
            ceremony::DELETE_ACCOUNT => "Account deleted.".to_string(),
            ceremony::AUTHORIZE_DEVICE => {
                "Approved. Handing the terminal its access\u{2026}".to_string()
            }
            _ => "Done.".to_string(),
        },
        ceremony_state::REFUSED | ceremony_state::FAILED => {
            format!("{subject} did not finish: {detail}")
        }
        _ => return,
    };
    if which == ceremony::AUTHORIZE_DEVICE {
        if state == ceremony_state::PENDING_CEREMONY || state == ceremony_state::WORKING {
            let _ = this.set_attribute("data-passkey-requested", "");
        } else {
            let _ = this.remove_attribute("data-passkey-requested");
        }
    }
    let _ = this.set_attribute("data-ceremony", &which);
    let _ = this.set_attribute("data-ceremony-state", &state);
    show_status(this, &text);
    // No refresh on ADD_PASSKEY: the passkey rows subscribe to
    // `RecoveryPasskey`, so a new one lands on its own commit rather
    // than on a re-read this ceremony has to remember to trigger.
}

/// Seed the display-name editable with what the roster resolved, so the
/// field is never blank while the member HAS a name. A Hub seat can read it
/// off the surrounding `<ui-hub-account>`; a dialog seat asks the roster.
fn prefill_name(this: &HtmlElement) {
    let Some(name) = name_input(this) else {
        return;
    };
    if !name.value().trim().is_empty() {
        return;
    }
    if let Some(active) = this
        .closest("ui-hub-account")
        .ok()
        .flatten()
        .and_then(|hub| hub.get_attribute("data-active-name"))
        .filter(|active| !active.trim().is_empty())
    {
        name.set_value(&active);
        return;
    }
    let host = this.clone();
    spawn_local(async move {
        let Ok(body) = tonk_host::get_json("/api/profiles").await else {
            return;
        };
        let Ok(response) = serde_json::from_str::<tonk_worker_api::ProfilesResponse>(&body) else {
            return;
        };
        let active = response
            .profiles
            .iter()
            .find(|profile| profile.active || profile.profile_name == response.active)
            .and_then(|profile| profile.display_name.clone())
            .filter(|name| !name.trim().is_empty());
        if let (Some(active), Some(name)) = (active, name_input(&host))
            && name.value().trim().is_empty()
        {
            name.set_value(&active);
        }
    });
}

fn name_input(this: &HtmlElement) -> Option<HtmlInputElement> {
    this.query_selector("[data-settings-name]")
        .ok()
        .flatten()
        .and_then(|field| field.dyn_into().ok())
}

/// Register `<ui-account-settings>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win
        .custom_elements()
        .get("ui-account-settings")
        .is_undefined()
    {
        UiAccountSettings::define("ui-account-settings");
        install_frame_shim(&win);
    }
}

/// Install `reset` / `update` / `error` on the element prototype,
/// forwarding to the per-instance closures. The host calls them by name
/// off the element for every subscription frame; on the prototype (not
/// each instance) so `this`-binding is correct, the same pattern
/// `<ui-sync-status>` uses.
fn install_frame_shim(win: &web_sys::Window) {
    let constructor = win.custom_elements().get("ui-account-settings");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    for (method, delegate) in [
        ("reset", "__tonkReset"),
        ("update", "__tonkUpdate"),
        ("error", "__tonkError"),
    ] {
        let forward = Function::new_with_args(
            "payload, opts",
            &format!("if (typeof this.{delegate} === 'function') this.{delegate}(payload, opts);"),
        );
        let _ = Reflect::set(&proto, &method.into(), &forward);
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{HtmlElement, HtmlInputElement, KeyboardEvent, KeyboardEventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn it_mounts_only_account_settings() {
        super::register();
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("ui-account-settings")
            .unwrap()
            .dyn_into()
            .unwrap();
        document.body().unwrap().append_child(&host).unwrap();

        let account_pane: HtmlElement = host
            .query_selector(".s-body [data-pane=\"account\"]")
            .unwrap()
            .expect("account pane")
            .dyn_into()
            .unwrap();
        assert!(!account_pane.hidden());
        assert!(
            host.query_selector(".s-rail").unwrap().is_none(),
            "a single account surface needs no section header"
        );
        assert!(
            host.query_selector("[data-pane=\"devices\"]")
                .unwrap()
                .is_none(),
            "settings has no devices tab or pane"
        );

        host.remove();
    }

    #[wasm_bindgen_test]
    fn custody_screen_replaces_approval_and_restores_on_dismiss() {
        let document = window().unwrap().document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(include_str!("../../tonk-ui/styles.css")));
        document.body().unwrap().append_child(&style).unwrap();
        let host = mount();
        super::set_pane(&host, "link");
        host.set_attribute("data-passkey-requested", "").unwrap();
        super::show_status(&host, "Waiting for your passkey…");
        let approval = pane(&host, "link");
        let top = approval.get_bounding_client_rect().top();
        let window = window().unwrap();
        window
            .dispatch_event(&web_sys::Event::new("tonk:custody-opened").unwrap())
            .unwrap();
        assert_eq!(
            window
                .get_computed_style(&approval)
                .unwrap()
                .unwrap()
                .get_property_value("display")
                .unwrap(),
            "none"
        );
        let status = host
            .query_selector("[data-ceremony-status]")
            .unwrap()
            .unwrap();
        assert_eq!(
            window
                .get_computed_style(&status)
                .unwrap()
                .unwrap()
                .get_property_value("display")
                .unwrap(),
            "none"
        );
        let seat = host.query_selector("[data-custody-seat]").unwrap().unwrap();
        assert_eq!(seat.get_bounding_client_rect().top(), top);
        window
            .dispatch_event(&web_sys::Event::new("tonk:custody-closed").unwrap())
            .unwrap();
        assert!(!host.has_attribute("data-passkey-screen"));
        assert!(!host.has_attribute("data-passkey-requested"));
        assert_eq!(
            window
                .get_computed_style(&approval)
                .unwrap()
                .unwrap()
                .get_property_value("display")
                .unwrap(),
            "flex"
        );
        host.remove();
        style.remove();
    }

    #[wasm_bindgen_test]
    fn custody_close_preserves_the_handoff_refusal() {
        let host = mount();
        super::set_pane(&host, "link");
        host.set_attribute("data-passkey-screen", "").unwrap();
        let row = js_sys::JSON::parse(
            &serde_json::json!({
                "fields": {
                    "ceremony": tonk_schema::ceremony::AUTHORIZE_DEVICE,
                    "state": tonk_schema::ceremony_state::REFUSED,
                    "detail": "this handoff requires account did:key:expected"
                }
            })
            .to_string(),
        )
        .unwrap();
        super::render_ceremony(&host, &row);
        let status = host
            .query_selector("[data-ceremony-status]")
            .unwrap()
            .unwrap();
        assert!(
            status
                .text_content()
                .unwrap()
                .contains("this handoff requires account")
        );
        window()
            .unwrap()
            .dispatch_event(&web_sys::Event::new("tonk:custody-closed").unwrap())
            .unwrap();
        assert!(
            status
                .text_content()
                .unwrap()
                .contains("this handoff requires account"),
            "closing the passkey screen must retain the worker's refusal"
        );
        assert!(!status.has_attribute("hidden"));
        assert!(!host.has_attribute("data-passkey-screen"));
        host.remove();
    }

    fn mount() -> HtmlElement {
        tonk_fab::register();
        super::register();
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("ui-account-settings")
            .unwrap()
            .dyn_into()
            .unwrap();
        document.body().unwrap().append_child(&host).unwrap();
        host
    }

    fn set_context(path: &str, search: &str, hash: &str) {
        let window = window().unwrap();
        let tonk = js_sys::Object::new();
        let context = js_sys::Object::new();
        for (key, value) in [
            ("origin", "https://tonk.test"),
            ("path", path),
            ("search", search),
            ("hash", hash),
        ] {
            js_sys::Reflect::set(&context, &key.into(), &value.into()).unwrap();
        }
        js_sys::Reflect::set(&tonk, &"context".into(), &context).unwrap();
        js_sys::Reflect::set(&window, &"tonk".into(), &tonk).unwrap();
    }

    fn clear_context() {
        js_sys::Reflect::set(
            &window().unwrap(),
            &"tonk".into(),
            &wasm_bindgen::JsValue::UNDEFINED,
        )
        .unwrap();
    }

    fn pane(host: &HtmlElement, name: &str) -> HtmlElement {
        host.query_selector(&format!(".s-body [data-pane=\"{name}\"]"))
            .unwrap()
            .expect("pane")
            .dyn_into()
            .unwrap()
    }

    /// Old device-pane links now land safely on the only settings pane.
    #[wasm_bindgen_test]
    fn it_lands_legacy_device_links_on_account_settings() {
        set_context("/settings", "", "#devices");
        let host = mount();
        assert!(!pane(&host, "account").hidden());
        assert!(
            host.query_selector("[data-pane=\"devices\"]")
                .unwrap()
                .is_none()
        );
        host.remove();
        clear_context();
    }

    /// `/settings/link?audience=&callback=&name=` is a terminal asking:
    /// the page shows who, and offers approve or decline.
    #[wasm_bindgen_test]
    fn it_shows_the_terminal_asking_for_access() {
        set_context(
            "/settings/link",
            "?audience=did%3Akey%3Az6MkTerminal&callback=http%3A%2F%2F127.0.0.1%3A4321%2F&name=e2e%20terminal",
            "",
        );
        let host = mount();
        assert!(!pane(&host, "link").hidden(), "the approval pane leads");
        assert!(pane(&host, "account").hidden());
        assert_eq!(
            host.query_selector("[data-link-name]")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default(),
            "e2e terminal"
        );
        assert_eq!(
            host.query_selector("[data-link-did]")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default(),
            "did:key:z6MkTerminal"
        );
        assert!(
            host.query_selector("[data-link-approve]")
                .unwrap()
                .is_some()
        );
        assert!(
            host.query_selector("[data-link-decline]")
                .unwrap()
                .is_some()
        );
        host.remove();
        clear_context();
    }

    /// The deletion verb stays off until the prompt's phrase is typed
    /// into the arming field, and comes on the moment it is.
    #[wasm_bindgen_test]
    fn it_arms_the_deletion_only_once_the_prompt_is_typed() {
        clear_context();
        let host = mount();
        host.query_selector("[data-delete-account-open]")
            .unwrap()
            .expect("the delete row")
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        let dialog: HtmlElement = host
            .query_selector("[data-delete-account-dialog]")
            .unwrap()
            .expect("the deletion dialog")
            .dyn_into()
            .unwrap();
        let native: web_sys::HtmlDialogElement = dialog
            .shadow_root()
            .expect("dialog shadow root")
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(native.open(), "the row raises the review");
        let verb: HtmlElement = host
            .query_selector("[data-delete-account-submit]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(
            verb.has_attribute("disabled"),
            "nothing typed, nothing armed"
        );

        host.set_attribute(
            "data-delete-confirm-expected",
            super::DELETE_ACCOUNT_CONFIRMATION,
        )
        .unwrap();
        let field: HtmlInputElement = host
            .query_selector("[data-delete-confirm]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let typed = |text: &str| {
            field.set_value(text);
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            let event = web_sys::Event::new_with_event_init_dict("input", &init).unwrap();
            field.dispatch_event(&event).unwrap();
        };
        typed("delete");
        assert!(verb.has_attribute("disabled"), "a partial phrase stays off");
        typed(super::DELETE_ACCOUNT_CONFIRMATION);
        assert!(!verb.has_attribute("disabled"), "the exact phrase arms it");
        typed("delete accounts");
        assert!(
            verb.has_attribute("disabled"),
            "and editing it away disarms"
        );
        assert!(native.open(), "arming never closes the review");
        host.remove();
    }

    #[wasm_bindgen_test]
    fn it_gives_owned_space_deletion_its_own_consequences() {
        set_context(
            "/settings",
            "?delete-space=did%3Akey%3AzOwned",
            "#delete-account",
        );
        let host = mount();
        let dialog = host
            .query_selector("[data-delete-account-dialog]")
            .unwrap()
            .unwrap();
        assert_eq!(
            dialog.get_attribute("heading").as_deref(),
            Some("delete this space?")
        );
        let native: web_sys::HtmlDialogElement = dialog
            .shadow_root()
            .unwrap()
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(
            native.open(),
            "the deletion URL opens the dialog on arrival"
        );
        assert_eq!(
            host.query_selector("[data-delete-confirm-label]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("delete space")
        );
        assert!(
            host.query_selector("[data-delete-question]")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("Everyone will lose access")
        );
        assert!(
            host.query_selector("[data-delete-consequence]")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("Copies saved on other devices may remain")
        );
        assert_eq!(
            host.query_selector("[data-delete-passkey]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("")
        );
        host.remove();
        clear_context();
    }

    #[wasm_bindgen_test]
    fn deletion_copy_wraps_long_space_names() {
        clear_context();
        let document = window().unwrap().document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(include_str!("../../tonk-ui/styles.css")));
        document.body().unwrap().append_child(&style).unwrap();
        let host = mount();
        super::open_delete_dialog(&host);
        super::set_text(&host, "[data-delete-scope]", &"long-space-name".repeat(30));
        let dialog: HtmlElement = host
            .query_selector("[data-delete-account-dialog]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let body: HtmlElement = dialog
            .shadow_root()
            .unwrap()
            .query_selector("[part=body]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(body.client_width() > 0);
        assert!(
            body.scroll_width() <= body.client_width(),
            "long names must wrap within the dialog"
        );
        host.remove();
        style.remove();
    }

    #[wasm_bindgen_test]
    fn enter_ends_a_display_name_edit() {
        super::register();
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("ui-account-settings")
            .unwrap()
            .dyn_into()
            .unwrap();
        document.body().unwrap().append_child(&host).unwrap();
        let input: HtmlInputElement = host
            .query_selector("[data-settings-name]")
            .unwrap()
            .expect("display-name input")
            .dyn_into()
            .unwrap();
        input.focus().expect("focus display name");
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&input))),
            "the edit must begin focused",
        );

        let init = KeyboardEventInit::new();
        init.set_key("Enter");
        init.set_bubbles(true);
        let enter = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        input.dispatch_event(&enter).unwrap();

        assert!(
            document
                .active_element()
                .is_none_or(|active| !active.is_same_node(Some(&input))),
            "Enter must blur the field so its existing change-save path runs",
        );
        host.remove();
    }
}
