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
//! The display name is the exception, and it commits declaratively through
//! the seeded `profile/rename` command on the editable's `onchange`.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, window};

type EventClosure = Closure<dyn FnMut(Event)>;

fn set_text(this: &HtmlElement, selector: &str, value: &str) {
    if let Ok(Some(element)) = this.query_selector(selector) {
        element.set_text_content(Some(value));
    }
}

#[derive(Default)]
struct UiAccountSettings {
    click: Option<EventClosure>,
    dialog_open: Option<EventClosure>,
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
            }
        }));
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        self.click = Some(click);

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

        refresh(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
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
    set_pane(this, "account");
    prefill_name(this);
    load_summary(this);
    load_devices(this);
}

/// Seed the display-name editable with what the roster resolved, so the
/// field is never blank while the member HAS a name. A Hub seat can read it
/// off the surrounding `<ui-hub-account>`; a dialog seat asks the roster.
fn prefill_name(this: &HtmlElement) {
    let Ok(Some(name)) = this.query_selector("[data-settings-name]") else {
        return;
    };
    if !name.text_content().unwrap_or_default().trim().is_empty() {
        return;
    }
    if let Some(active) = this
        .closest("ui-hub-account")
        .ok()
        .flatten()
        .and_then(|hub| hub.get_attribute("data-active-name"))
        .filter(|active| !active.trim().is_empty())
    {
        name.set_text_content(Some(&active));
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
        if let (Some(active), Ok(Some(name))) =
            (active, host.query_selector("[data-settings-name]"))
            && name.text_content().unwrap_or_default().trim().is_empty()
        {
            name.set_text_content(Some(&active));
        }
    });
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
    use web_sys::{HtmlElement, window};

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
}
