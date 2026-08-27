//! The pending-email condition shared by the bar and its share stack.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

const BANNER_ID: &str = "fabb-activation-banner";
const CLUSTER_ID: &str = "fabb-activation-cluster";
const POLL_MS: i32 = 10_000;

pub(crate) struct ActivationWatch {
    interval: i32,
    _tick: Closure<dyn FnMut()>,
}

impl Drop for ActivationWatch {
    fn drop(&mut self) {
        if let Some(window) = window() {
            window.clear_interval_with_handle(self.interval);
        }
    }
}

pub(crate) fn watch(this: &HtmlElement) -> Option<ActivationWatch> {
    poll(this);
    let host = this.clone();
    let tick = Closure::<dyn FnMut()>::new(move || poll(&host));
    let interval = window()?
        .set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            POLL_MS,
        )
        .ok()?;
    Some(ActivationWatch {
        interval,
        _tick: tick,
    })
}

pub(crate) fn poll(this: &HtmlElement) {
    let host = this.clone();
    spawn_local(async move {
        let Ok(body) = tonk_host::get_json("/api/customer").await else {
            return;
        };
        let Ok(state) = serde_json::from_str::<serde_json::Value>(&body) else {
            return;
        };
        render(&host, state["status"].as_str(), state["email"].as_str());
    });
}

fn render(this: &HtmlElement, status: Option<&str>, email: Option<&str>) {
    if !this.is_connected() {
        return;
    }
    let status = status.unwrap_or_default();
    let _ = this.set_attribute("data-customer-status", status);
    if let Some(email) = email {
        let _ = this.set_attribute("data-customer-email", email);
    } else {
        let _ = this.remove_attribute("data-customer-email");
    }
    if let Ok(Some(row)) = this.query_selector("[data-share-link]") {
        if status == "Registered" {
            let _ = row.set_attribute("data-activation-blocked", "");
        } else {
            let _ = row.remove_attribute("data-activation-blocked");
        }
    }

    if status != "Registered" {
        retire_condition();
        crate::bar::refresh_sync_condition(this);
        return;
    }

    if let Some(document) = window().and_then(|window| window.document()) {
        if let Some(connect) = document.get_element_by_id(crate::bar::CONNECT_BANNER_ID) {
            connect.remove();
        }
    }
    ensure_banner(this, email.unwrap_or("your email address"));
    repaint_cluster(email.unwrap_or("your email address"));
}

fn ensure_banner(this: &HtmlElement, email: &str) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(banner) = document.get_element_by_id(BANNER_ID) {
        set_banner_copy(&banner, email);
        if let Some(mode) = this.get_attribute("mode") {
            let _ = banner.set_attribute("mode", &mode);
        } else {
            let _ = banner.remove_attribute("mode");
        }
        return;
    }
    let Ok(banner) = document.create_element("tonk-banner") else {
        return;
    };
    banner.set_id(BANNER_ID);
    if let Some(mode) = this.get_attribute("mode") {
        let _ = banner.set_attribute("mode", &mode);
    }
    let Ok(message) = document.create_element("span") else {
        return;
    };
    let _ = message.set_attribute("data-activation-message", "");
    let Ok(door) = document.create_element("span") else {
        return;
    };
    let _ = door.set_attribute("slot", "door");
    door.set_text_content(Some("activate"));
    let _ = banner.append_child(&message);
    let _ = banner.append_child(&door);
    set_banner_copy(&banner, email);

    let host = this.clone();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        open_cluster(&host);
        poll(&host);
    });
    let _ = banner.add_event_listener_with_callback("fabb-open", on_open.as_ref().unchecked_ref());
    on_open.forget();
    if let Some(body) = document.body() {
        let _ = body.append_child(&banner);
    }
}

fn set_banner_copy(banner: &Element, email: &str) {
    if let Ok(Some(message)) = banner.query_selector("[data-activation-message]") {
        message.set_text_content(Some(&format!(
            "{email} is not activated yet — nothing syncs until it is"
        )));
    }
}

fn open_cluster(this: &HtmlElement) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(banner) = document.get_element_by_id(BANNER_ID) {
        let _ = banner.set_attribute("hidden", "");
    }
    if let Some(cluster) = document.get_element_by_id(CLUSTER_ID) {
        let _ = cluster.remove_attribute("hidden");
        return;
    }
    let email = activation_email(this);
    let Ok(cluster) = document.create_element("tonk-cluster") else {
        return;
    };
    cluster.set_id(CLUSTER_ID);
    if let Some(mode) = this.get_attribute("mode") {
        let _ = cluster.set_attribute("mode", &mode);
    }
    cluster.set_inner_html(
        r#"<p slot="statement">activate sync for this account</p>
<tonk-field noun="email" settled changeable data-activation-email></tonk-field>
<p slot="narrator" data-activation-narrator>Open the link in your activation email. <button data-resend-activation>resend activation email</button></p>
<tonk-button slot="run" variant="primary" solid data-check-activation>check activation</tonk-button>
<span slot="ghost">back to your space</span>"#,
    );
    style_narrator_verb(&cluster);
    if let Ok(Some(field)) = cluster.query_selector("[data-activation-email]") {
        let _ = field.set_attribute("value", &email);
    }

    let ceremony = cluster.clone();
    let on_bail = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        ceremony.remove();
        if let Some(banner) = window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(BANNER_ID))
        {
            let _ = banner.remove_attribute("hidden");
        }
    });
    let _ = cluster.add_event_listener_with_callback("fabb-bail", on_bail.as_ref().unchecked_ref());
    on_bail.forget();

    let on_change = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        tonk_host::navigate_to("/account");
    });
    let _ = cluster
        .add_event_listener_with_callback("fabb-change-noun", on_change.as_ref().unchecked_ref());
    on_change.forget();

    if let Ok(Some(resend)) = cluster.query_selector("[data-resend-activation]") {
        let ceremony = cluster.clone();
        let on_resend = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            event.prevent_default();
            let ceremony = ceremony.clone();
            spawn_local(async move {
                let result =
                    tonk_host::post_json("/api/customer/enroll", r#"{"email":null,"deposits":[]}"#)
                        .await;
                if let Ok(Some(narrator)) = ceremony.query_selector("[data-activation-narrator]") {
                    narrator.set_text_content(Some(if result.is_ok() {
                        "Sent — open the link in your activation email."
                    } else {
                        "The activation email could not be sent. Try again from account settings."
                    }));
                }
            });
        });
        let _ =
            resend.add_event_listener_with_callback("click", on_resend.as_ref().unchecked_ref());
        on_resend.forget();
    }

    if let Ok(Some(check)) = cluster.query_selector("[data-check-activation]") {
        let host = this.clone();
        let on_check = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| poll(&host));
        let _ =
            check.add_event_listener_with_callback("fabb-press", on_check.as_ref().unchecked_ref());
        on_check.forget();
    }

    if let Some(body) = document.body() {
        let _ = body.append_child(&cluster);
    }
}

fn activation_email(this: &HtmlElement) -> String {
    this.get_attribute("data-customer-email")
        .unwrap_or_else(|| "your email address".to_owned())
}

fn repaint_cluster(email: &str) {
    let Some(cluster) = window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(CLUSTER_ID))
    else {
        return;
    };
    if let Ok(Some(field)) = cluster.query_selector("[data-activation-email]") {
        let _ = field.set_attribute("value", email);
    }
}

fn style_narrator_verb(cluster: &Element) {
    if let Ok(Some(button)) = cluster.query_selector("[data-resend-activation]") {
        let Some(button) = button.dyn_ref::<HtmlElement>() else {
            return;
        };
        let style = button.style();
        let _ = style.set_property("all", "unset");
        let _ = style.set_property("cursor", "pointer");
        let _ = style.set_property("color", "var(--fabb-ink, currentColor)");
        let _ = style.set_property("text-decoration", "underline");
        let _ = style.set_property("text-underline-offset", "2px");
    }
}

fn retire_condition() {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(cluster) = document.get_element_by_id(CLUSTER_ID) {
        cluster.remove();
    }
    if let Some(banner) = document.get_element_by_id(BANNER_ID) {
        let retire = js_sys::Reflect::get(&banner, &"retire".into())
            .ok()
            .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
        if let Some(retire) = retire {
            let _ = retire.call0(&banner);
        } else {
            banner.remove();
        }
    }
}

pub(crate) fn remove() {
    retire_condition();
}

#[cfg(test)]
mod tests {
    #[test]
    fn registration_poll_is_deliberately_slow_and_non_spinning() {
        assert_eq!(super::POLL_MS, 10_000);
    }
}
