//! The pending-email condition shared by the bar and its share stack.
//!
//! Driven by a subscription to the account's own `AccountCustomer` row,
//! not by polling `GET /api/customer`.
//!
//! The probe it replaces ran every 10 seconds for the life of the bar,
//! and on a device that never registered every one of them answered
//! `409 ROOT_REQUIRED` — the route wants a local passkey root to read
//! the record. The poll swallowed that and returned, so the condition it
//! meant to render was never right on exactly the devices that needed
//! it, while the console filled with conflicts.
//!
//! A subscription answers once, when the fact changes, which is also
//! precisely when the condition changes.

use js_sys::{JSON, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

const BANNER_ID: &str = "fabb-activation-banner";
const CLUSTER_ID: &str = "fabb-activation-cluster";

/// Distinguishes this element's account subscriptions from its others.
const SUB_TAG: &str = "fabb-activation";
/// The activation half, subscribed separately: registration and activation
/// are independent facts that resolve independently.
const ACTIVE_TAG: &str = "fabb-activation-active";

thread_local! {
    /// The live registration subscription, held here rather than in the
    /// watch because it is opened a microtask after `watch` returned.
    static OPEN: std::cell::RefCell<Option<Subscription>> =
        const { std::cell::RefCell::new(None) };
    /// The live activation subscription.
    static ACTIVE_OPEN: std::cell::RefCell<Option<Subscription>> =
        const { std::cell::RefCell::new(None) };
    /// The address the registration frame carried, latched so an
    /// activation frame (which carries no address) still renders it.
    static REGISTERED_EMAIL: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// Whether an activation frame has arrived.
    static ACTIVATED: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
}

pub(crate) struct ActivationWatch {
    _frames: Vec<Closure<dyn FnMut(JsValue, JsValue)>>,
}

impl Drop for ActivationWatch {
    fn drop(&mut self) {
        OPEN.with(|open| {
            if let Some(mut subscription) = open.borrow_mut().take() {
                subscription.cancel();
            }
        });
    }
}

pub(crate) fn watch(this: &HtmlElement) -> Option<ActivationWatch> {
    // The account row lives on the profile branch, not this space's.
    let _ = this.set_attribute("with", "main@profile:tonk");

    let mut frames = Vec::new();
    for method in ["reset", "update"] {
        let host = this.clone();
        let is_delta = method == "update";
        let frame =
            Closure::<dyn FnMut(JsValue, JsValue)>::new(move |payload: JsValue, _opts: JsValue| {
                apply(&host, &payload, is_delta);
            });
        if Reflect::set(this.as_ref(), &method.into(), frame.as_ref()).is_err() {
            return None;
        }
        frames.push(frame);
    }

    // Deferred a microtask, with a connected guard.
    //
    // `watch` runs from `connectedCallback`, and the custom-element
    // reaction queue delivers that to an element that is not yet in the
    // document — so subscribing here directly fails with
    // `no host claimed the event (connected=false)`. The host listens on
    // `document`, and a detached element's event never reaches it.
    //
    // The share row then never learned the account existed, and clicking
    // share waited for a link that was never going to arrive.
    // Two subscriptions, because registration and activation are two
    // independent facts that resolve independently: an enrolled account
    // has a registration row and no activation row, and one query
    // requiring both would resolve for neither.
    let query = JSON::parse(&crate::logic::account_customer_query_body()).ok()?;
    match consumer::subscribe(this, &query, Some(&SUB_TAG.into())) {
        Ok(subscription) => OPEN.with(|open| *open.borrow_mut() = Some(subscription)),
        Err(error) => tonk_common::log!("activation: could not watch: {error:?}"),
    }
    let active = JSON::parse(&crate::logic::account_active_query_body()).ok()?;
    match consumer::subscribe(this, &active, Some(&ACTIVE_TAG.into())) {
        Ok(subscription) => ACTIVE_OPEN.with(|open| *open.borrow_mut() = Some(subscription)),
        Err(error) => tonk_common::log!("activation: could not watch activation: {error:?}"),
    }

    // An account nobody has registered yet resolves to no row at all.
    // That absence IS the answer, and nothing else will say it, so paint
    // it now rather than waiting for a frame.
    render(this, None, false);

    Some(ActivationWatch { _frames: frames })
}

/// Render whichever row the frame carries.
///
/// `status` is cardinality-one on the account, so the newest asserted
/// row is the current answer; a bare retract leaves the condition alone.
fn apply(host: &HtmlElement, payload: &JsValue, is_delta: bool) {
    let rows = if is_delta {
        Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED)
    } else {
        payload.clone()
    };
    let rows = js_sys::Array::from(&rows);
    let Some(index) = rows.length().checked_sub(1) else {
        return;
    };
    let row = rows.get(index);
    if row.is_undefined() || row.is_null() {
        return;
    }
    let Ok(fields) = Reflect::get(&row, &"fields".into()) else {
        return;
    };
    let read = |name: &str| {
        Reflect::get(&fields, &name.into())
            .ok()
            .and_then(|value| value.as_string())
    };
    // A registration frame carries the address; an activation frame
    // carries `activated_at`, and its PRESENCE is the whole signal —
    // there is no provider on it any more. The reader once looked for
    // `provider` here after the query had moved on, so every activation
    // frame read as not-activated and the banner lingered for the life
    // of the page. [`crate::logic::ACTIVATION_FIELD`] is what the query
    // binds, pinned to this reader by a test.
    if let Some(email) = read("email") {
        REGISTERED_EMAIL.with(|cell| *cell.borrow_mut() = Some(email));
    }
    let activated_frame = Reflect::get(&fields, &crate::logic::ACTIVATION_FIELD.into())
        .is_ok_and(|value| !value.is_undefined() && !value.is_null());
    if activated_frame {
        ACTIVATED.with(|cell| *cell.borrow_mut() = true);
    }
    let email = REGISTERED_EMAIL.with(|cell| cell.borrow().clone());
    let activated = ACTIVATED.with(|cell| *cell.borrow());
    render(host, email.as_deref(), activated);
}

fn render(this: &HtmlElement, email: Option<&str>, activated: bool) {
    if !this.is_connected() {
        return;
    }
    // Which of the share stack's two rows shows — "log in to share" or
    // the copy row — is the same question this subscription already
    // answers, so it is answered here rather than by a second probe.
    // `element.rs` used to fetch `/api/account` once on connect for it,
    // which went stale the moment someone registered: the bar kept
    // offering to log in to an account that now existed.
    // An account exists as soon as it has registered; being SERVED is
    // the separate activation fact.
    crate::element::apply_account_ready(this, email.is_some());
    let _ = this.set_attribute(
        "data-customer-status",
        if email.is_none() {
            ""
        } else if activated {
            "Active"
        } else {
            "Registered"
        },
    );
    if let Some(email) = email {
        let _ = this.set_attribute("data-customer-email", email);
    } else {
        let _ = this.remove_attribute("data-customer-email");
    }
    if let Ok(Some(row)) = this.query_selector("[data-share-link]") {
        if email.is_some() && !activated {
            let _ = row.set_attribute("data-activation-blocked", "");
        } else {
            let _ = row.remove_attribute("data-activation-blocked");
        }
    }

    // The banner is for exactly one state: registered and waiting.
    if email.is_none() || activated {
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
        crate::shadow::set_mode(&banner, this.get_attribute("mode").as_deref());
        return;
    }
    let Ok(banner) = document.create_element("tonk-banner") else {
        return;
    };
    banner.set_id(BANNER_ID);
    crate::shadow::set_mode(&banner, this.get_attribute("mode").as_deref());
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
    // No refresh on open: the subscription is already current, which is
    // the point of it.
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        open_cluster(&host);
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
            "{email} is waiting for email confirmation — nothing syncs until you confirm it"
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
    crate::shadow::set_mode(&cluster, this.get_attribute("mode").as_deref());
    cluster.set_inner_html(
        r#"<p slot="statement">activate sync for this account</p>
<tonk-field noun="email" settled changeable data-activation-email></tonk-field>
<p slot="narrator" data-activation-narrator>Open the link in your activation email. <button data-resend-activation>resend activation email</button></p>
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
    /// The condition is read from the account's own row, not fetched.
    ///
    /// This replaces a test pinning a 10-second poll interval. The poll
    /// it guarded answered `409 ROOT_REQUIRED` on every tick for a
    /// device that had never registered, and swallowed it — so the
    /// condition was never right on the devices that needed it. What is
    /// worth pinning now is that the read is a query over raw attribute
    /// URIs, which nothing seeded can break.
    #[dialog_common::test]
    fn it_reads_the_condition_from_the_account_row() {
        let body = crate::logic::account_customer_query_body();
        assert!(body.contains("xyz.tonk.account/registered-at"));
        assert!(body.contains("xyz.tonk.account/customer-email"));
        // A concept name would need the profile to have been seeded with
        // a matching definition; the raw URIs need nothing.
        assert!(!body.contains("tonk:account"));
    }
}
