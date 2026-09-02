//! `<ui-account-settings>` — the account settings panel: a rail of panes
//! over the live account facts.
//!
//! One panel, two seats. The Hub's account tab embeds it as the in-column
//! settings page; the space route mounts it inside a `<tonk-dialog>` the
//! FAB's `settings` row raises. It injects its own markup and fills the
//! rows imperatively because the values are API reads, not branch facts:
//! the address is service-owned by design (the uniqueness key is never
//! mirrored into the account repository) and the device list derives from
//! delegation chains — a declarative view has nothing to subscribe to.
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

fn set_text(this: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = this.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

#[derive(Default)]
struct UiAccountSettings {
    click: Option<EventClosure>,
    change: Option<EventClosure>,
    keydown: Option<EventClosure>,
    dialog_open: Option<EventClosure>,
    /// The live ceremony-status subscription, held while connected.
    subscription: Rc<RefCell<Option<Subscription>>>,
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
        if this.query_selector(".s-rail").ok().flatten().is_none() {
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
            if let Some(pane) = target
                .closest(".s-rail [data-pane]")
                .ok()
                .flatten()
                .and_then(|button| button.get_attribute("data-pane"))
            {
                set_pane(&host, &pane);
                return;
            }
            // Removing a device's access asks in place — the word answers:
            // the first press arms the verb, the second revokes.
            if let Some(verb) = target
                .closest("[data-revoke-device]")
                .ok()
                .flatten()
                .and_then(|verb| verb.dyn_into::<HtmlElement>().ok())
            {
                event.prevent_default();
                revoke_device(&host, &verb);
                return;
            }
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
        let keydown: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            if key.key() != "Enter" {
                return;
            }
            let Some(input) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                .filter(|input| input.has_attribute("data-settings-name"))
            else {
                return;
            };
            key.prevent_default();
            let _ = input.blur();
        }));
        let _ = this.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        self.keydown = Some(keydown);

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
        let reset: FrameClosure = Closure::wrap(Box::new(move |payload: JsValue, _: JsValue| {
            on_ceremony_snapshot(&host, payload);
        }));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        let host = this.clone();
        let update: FrameClosure = Closure::wrap(Box::new(move |payload: JsValue, _: JsValue| {
            on_ceremony_delta(&host, payload);
        }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        self.frames = vec![reset, update];
        subscribe_ceremony(this, self.subscription.clone());

        refresh(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
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

/// Show one pane and mark its rail tab current.
fn set_pane(this: &HtmlElement, pane: &str) {
    if let Ok(tabs) = this.query_selector_all(".s-rail [data-pane]") {
        for index in 0..tabs.length() {
            if let Some(tab) = tabs
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                let current = tab.get_attribute("data-pane").as_deref() == Some(pane);
                let _ = tab.class_list().toggle_with_force("cur", current);
            }
        }
    }
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
/// fills in beats one that waits on two fetches.
pub(crate) fn refresh(this: &HtmlElement) {
    // `/settings/link?audience=&callback=&name=` is a terminal asking
    // for access; `/settings#devices` lands on the devices pane.
    match link_request() {
        Some(request) => {
            set_text(this, "[data-link-name]", &request.name);
            set_text(this, "[data-link-did]", &request.audience);
            set_pane(this, "link");
        }
        None => {
            let location = page_location();
            set_pane(
                this,
                if location.hash == "#devices" {
                    "devices"
                } else {
                    "account"
                },
            );
            // `tonk account delete` and `tonk account spots delete` open
            // this page with the review already asked for.
            if location.hash == "#delete-account" {
                open_delete_dialog(this);
            }
        }
    }
    prefill_name(this);
    load_summary(this);
    load_devices(this);
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
        audience,
        callback,
        name,
    })
}

/// Raise one of this panel's `<tonk-dialog>` clusters.
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

fn input_value(this: &HtmlElement, selector: &str) -> String {
    this.query_selector(selector)
        .ok()
        .flatten()
        .and_then(|field| field.dyn_into::<HtmlInputElement>().ok())
        .map(|field| field.value())
        .unwrap_or_default()
}

fn input_checked(this: &HtmlElement, selector: &str) -> bool {
    this.query_selector(selector)
        .ok()
        .flatten()
        .and_then(|field| field.dyn_into::<HtmlInputElement>().ok())
        .is_some_and(|field| field.checked())
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
    set_hidden(this, "[data-delete-error]", true);
    set_text(
        this,
        "[data-delete-scope]",
        "Loading what this deletes\u{2026}",
    );
    if let Ok(Some(list)) = this.query_selector("[data-delete-spaces]") {
        list.set_inner_html("");
    }
    show_dialog(this, "[data-delete-account-dialog]");
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
        let requested = requested_space_deletion();
        let spaces: Vec<_> = plan
            .spaces
            .iter()
            .filter(|space| {
                requested
                    .as_deref()
                    .is_none_or(|subject| space.subject == subject)
            })
            .collect();
        if let Some(subject) = &requested
            && spaces.is_empty()
        {
            set_text(
                &host,
                "[data-delete-scope]",
                &format!("{subject} is not an owned hosted space of this account."),
            );
            return;
        }
        let _ = host.set_attribute(
            "data-delete-space",
            requested.as_deref().unwrap_or_default(),
        );
        set_text(
            &host,
            "[data-delete-submit-label]",
            if requested.is_some() {
                "delete selected owned space"
            } else {
                "delete owned spaces and account"
            },
        );
        let owned = spaces.len();
        set_text(
            &host,
            "[data-delete-scope]",
            &if requested.is_some() {
                format!(
                    "This deletes the selected owned space's hosted content from Tonk services. Your account and every other space remain. {} joined space{} will be left intact.",
                    plan.joined_spaces,
                    if plan.joined_spaces == 1 { "" } else { "s" },
                )
            } else {
                format!(
                    "{owned} owned hosted space{} will be deleted. {} joined space{} will be left intact.",
                    if owned == 1 { "" } else { "s" },
                    plan.joined_spaces,
                    if plan.joined_spaces == 1 { "" } else { "s" },
                )
            },
        );
        let (Ok(Some(list)), Some(document)) = (
            host.query_selector("[data-delete-spaces]"),
            window().and_then(|win| win.document()),
        ) else {
            return;
        };
        for space in spaces {
            if let Ok(item) = document.create_element("li") {
                item.set_text_content(Some(space.name.as_deref().unwrap_or(&space.subject)));
                let _ = list.append_child(&item);
            }
        }
        let _ = host.set_attribute("data-delete-email-expected", &plan.email);
    });
}

/// Assert `tonk:delete-account`. The worker checks the address against
/// the account, asks the page for the passkey, and reports through the
/// ceremony row this panel watches.
fn submit_delete(this: &HtmlElement) {
    let email = input_value(this, "[data-delete-email]");
    let expected = this
        .get_attribute("data-delete-email-expected")
        .unwrap_or_default();
    let error = if email.trim().is_empty() || (!expected.is_empty() && email.trim() != expected) {
        Some("The confirmation email does not match this account.")
    } else if !input_checked(this, "[data-delete-understood]") {
        Some("Confirm that you understand the permanent consequences.")
    } else {
        None
    };
    if let Some(error) = error {
        set_text(this, "[data-delete-error]", error);
        set_hidden(this, "[data-delete-error]", false);
        return;
    }
    set_hidden(this, "[data-delete-error]", true);
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
    show_status(this, "Waiting for your passkey\u{2026}");
    transact(&claim(
        "Delete this account from every service and this device.",
        serde_json::json!({
            "email": { "the": "xyz.tonk.delete-account/email", "as": "Text" }
        }),
        serde_json::json!({ "email": email.trim() }),
    ));
}

/// Sign this device out: the account stays, this browser forgets it.
fn sign_out(this: &HtmlElement) {
    close_dialog(this, "[data-sign-out-dialog]");
    show_status(this, "Signing out\u{2026}");
    let host = this.clone();
    spawn_local(async move {
        match tonk_host::post_json("/api/account/unlink", "{}").await {
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
    transact(&claim(
        "Seal the account under another passkey.",
        serde_json::json!({
            "marker": { "the": "dom.event.current-target.dataset/add-passkey", "as": "Entity" }
        }),
        serde_json::json!({ "marker": "tonk:add-passkey" }),
    ));
}

/// Assert `tonk:authorize-device` for the terminal named in the URL.
fn approve_link(this: &HtmlElement) {
    let Some(request) = link_request() else {
        return;
    };
    show_status(this, "Waiting for your passkey\u{2026}");
    transact(&claim(
        "Delegate the account to a waiting terminal.",
        serde_json::json!({
            "audience": { "the": "xyz.tonk.authorize-device/audience", "as": "Entity" },
            "callback": { "the": "xyz.tonk.authorize-device/callback", "as": "Text" },
            "name": { "the": "xyz.tonk.authorize-device/name", "as": "Text" }
        }),
        serde_json::json!({
            "audience": request.audience,
            "callback": bs58::encode(request.callback.as_bytes()).into_string(),
            "name": request.name,
        }),
    ));
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
/// the profile branch this guest is mounted with.
fn transact(request: &serde_json::Value) {
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
        return;
    };
    if let Ok(body) = JSON::parse(&text) {
        let _ = transact.call1(&tonk, &body);
    }
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
            Ok(sub) => *subscription.borrow_mut() = Some(sub),
            Err(error) => tonk_common::log!("ui-account-settings: subscribe failed: {error:?}"),
        }
    });
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
    let _ = this.set_attribute("data-ceremony", &which);
    let _ = this.set_attribute("data-ceremony-state", &state);
    show_status(this, &text);
    if state == ceremony_state::DONE && which == ceremony::ADD_PASSKEY {
        load_summary(this);
    }
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

fn load_summary(this: &HtmlElement) {
    let host = this.clone();
    spawn_local(async move {
        match tonk_host::get_json("/api/account/summary").await {
            Ok(body) => {
                let summary: Option<tonk_worker_api::AccountSummary> =
                    serde_json::from_str(&body).ok();
                let summary = summary.unwrap_or(tonk_worker_api::AccountSummary {
                    email: None,
                    passkey: None,
                    display_name: None,
                });
                let email = summary
                    .email
                    .filter(|email| !email.trim().is_empty())
                    .unwrap_or_else(|| "Unavailable".to_string());
                set_text(&host, "[data-settings-email]", &email);
                match summary.passkey {
                    Some(passkey) => {
                        set_text(&host, "[data-settings-passkey-device]", &passkey.created_on);
                        let date = js_sys::Date::new(&JsValue::from_f64(
                            passkey.created_at as f64 * 1000.0,
                        ))
                        .to_locale_date_string("default", &JsValue::UNDEFINED);
                        set_text(
                            &host,
                            "[data-settings-passkey-created]",
                            &format!("created {}", String::from(date)),
                        );
                    }
                    None => {
                        set_text(&host, "[data-settings-passkey-device]", "Unavailable");
                        set_text(&host, "[data-settings-passkey-created]", "");
                    }
                }
            }
            Err(_) => {
                set_text(&host, "[data-settings-email]", "Unavailable");
                set_text(&host, "[data-settings-passkey-device]", "Unavailable");
            }
        }
    });
}

fn load_devices(this: &HtmlElement) {
    let host = this.clone();
    spawn_local(async move {
        let devices: Vec<tonk_worker_api::AccountDevice> =
            match tonk_host::get_json("/api/account/devices").await {
                Ok(body) => serde_json::from_str(&body).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
        // The list is the same on every device; "this device" is a
        // presentation mark (the wireframe's soft suffix on the name).
        let own = match tonk_host::get_json("/api/identify").await {
            Ok(body) => serde_json::from_str::<tonk_worker_api::IdentifyResponse>(&body)
                .map(|identity| identity.did)
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        let Some(list) = host
            .query_selector("[data-settings-devices]")
            .ok()
            .flatten()
        else {
            return;
        };
        let Some(document) = window().and_then(|window| window.document()) else {
            return;
        };
        list.set_inner_html("");
        if devices.is_empty() {
            let Ok(row) = document.create_element("div") else {
                return;
            };
            row.set_class_name("srowd");
            row.set_text_content(Some("No linked devices to list."));
            let _ = list.append_child(&row);
            return;
        }
        for device in devices {
            let Ok(row) = document.create_element("div") else {
                continue;
            };
            row.set_class_name("srowd");
            let Ok(name) = document.create_element("b") else {
                continue;
            };
            name.set_class_name("lft");
            name.set_text_content(Some(&device.name));
            if !own.is_empty() && device.did == own {
                if let Ok(marker) = document.create_element("span") {
                    marker.set_class_name("dev-self");
                    marker.set_text_content(Some(" \u{b7} this device"));
                    let _ = name.append_child(&marker);
                }
            }
            let _ = row.append_child(&name);
            let Ok(meta) = document.create_element("span") else {
                continue;
            };
            meta.set_class_name("dev-r");
            let Ok(when) = document.create_element("span") else {
                continue;
            };
            when.set_class_name("dev-when");
            let date = js_sys::Date::new(&JsValue::from_f64(device.created_at as f64 * 1000.0))
                .to_locale_date_string("default", &JsValue::UNDEFINED);
            when.set_text_content(Some(&format!("linked {}", String::from(date))));
            let _ = meta.append_child(&when);
            let Ok(verb) = document.create_element("button") else {
                continue;
            };
            verb.set_class_name("cta");
            let _ = verb.set_attribute("type", "button");
            let _ = verb.set_attribute("data-revoke-device", &device.did);
            verb.set_text_content(Some("remove access"));
            let _ = meta.append_child(&verb);
            let _ = row.append_child(&meta);
            let _ = list.append_child(&row);
        }
    });
}

/// Revoke one device's access, asking in place: the first press arms the
/// verb ("sure? remove"), the second sends the revocation and re-reads the
/// list.
fn revoke_device(this: &HtmlElement, verb: &HtmlElement) {
    let Some(did) = verb.get_attribute("data-revoke-device") else {
        return;
    };
    if !verb.has_attribute("data-armed") {
        let _ = verb.set_attribute("data-armed", "");
        verb.set_text_content(Some("sure? remove"));
        return;
    }
    verb.set_text_content(Some("removing\u{2026}"));
    let _ = verb.set_attribute("disabled", "");
    let host = this.clone();
    let verb = verb.clone();
    spawn_local(async move {
        let body = serde_json::json!({ "did": did }).to_string();
        match tonk_host::post_json("/api/account/devices/revoke", &body).await {
            Ok(_) => load_devices(&host),
            Err(_) => {
                let _ = verb.remove_attribute("disabled");
                let _ = verb.remove_attribute("data-armed");
                verb.set_text_content(Some("couldn\u{2019}t remove"));
            }
        }
    });
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
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{HtmlElement, HtmlInputElement, KeyboardEvent, KeyboardEventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn it_mounts_the_rail_and_switches_panes() {
        super::register();
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("ui-account-settings")
            .unwrap()
            .dyn_into()
            .unwrap();
        document.body().unwrap().append_child(&host).unwrap();

        let devices_tab: HtmlElement = host
            .query_selector(".s-rail [data-pane=\"devices\"]")
            .unwrap()
            .expect("devices tab")
            .dyn_into()
            .unwrap();
        let devices_pane: HtmlElement = host
            .query_selector(".s-body [data-pane=\"devices\"]")
            .unwrap()
            .expect("devices pane")
            .dyn_into()
            .unwrap();
        let account_pane: HtmlElement = host
            .query_selector(".s-body [data-pane=\"account\"]")
            .unwrap()
            .expect("account pane")
            .dyn_into()
            .unwrap();
        assert!(devices_pane.hidden(), "the account pane leads");
        assert!(!account_pane.hidden());

        devices_tab.click();
        assert!(!devices_pane.hidden(), "the rail switches panes");
        assert!(account_pane.hidden());
        assert!(devices_tab.class_list().contains("cur"));

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

    /// `/settings#devices` lands on the devices pane: a tab is a place.
    #[wasm_bindgen_test]
    fn it_lands_on_the_pane_the_fragment_names() {
        set_context("/settings", "", "#devices");
        let host = mount();
        assert!(
            !pane(&host, "devices").hidden(),
            "the fragment picks the pane"
        );
        assert!(pane(&host, "account").hidden());
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

    /// The deletion review refuses to go on without the retyped address
    /// and the acknowledgement, in that order.
    #[wasm_bindgen_test]
    fn it_arms_the_deletion_only_with_the_address_and_the_acknowledgement() {
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
        let submit: HtmlElement = host
            .query_selector("[data-delete-account-submit]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let error: HtmlElement = host
            .query_selector("[data-delete-error]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        submit.click();
        assert!(!error.hidden(), "an empty address refuses");
        assert!(
            error
                .text_content()
                .unwrap_or_default()
                .contains("does not match")
        );

        let email: HtmlInputElement = host
            .query_selector("[data-delete-email]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        email.set_value("goner@example.com");
        submit.click();
        assert!(
            error
                .text_content()
                .unwrap_or_default()
                .contains("understand"),
            "an unchecked acknowledgement refuses next"
        );
        assert!(native.open(), "a refused submit keeps the review up");
        host.remove();
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
