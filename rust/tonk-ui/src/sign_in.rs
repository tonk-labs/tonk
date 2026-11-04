//! Signing in, on the host's page.
//!
//! The persona kernel asks for a sign-in (its provisional persona asks to
//! merge), and the profile's host asks this page: `{ type: "sign-in" }`. A
//! space this page shows itself may need the account too, and asks here
//! directly. Either way this page asks the person who they are, looks the
//! address up, and runs the passkey ceremony in the click that asks for it:
//! a new passkey for an address with no account, the account's own for one
//! with. What the passkey derives goes to the profile's worker, which signs
//! the person in; the kernel never sees the address typed or the passkey.

use std::cell::RefCell;

use tonk_identity::custody::CustodyDenial;
use tonk_worker_api::{AccountCreation, CustodyIntent, DeviceLink};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, MessageEvent};

use crate::custody_relay::{self, CeremonyError};

const PANEL_ID: &str = "tonk-register";
const STYLES: &str = include_str!("sign_in.css");

/// The ceremony a sign-in runs, as the profile's worker names it.
const CREATE: &str = "create";
const LOG_IN: &str = "log-in";

/// Where the panel is.
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    /// Asks for an address, saying `says` under it.
    Address { email: String, says: Option<String> },
    /// Looks the address up.
    Checking { email: String },
    /// The address is free: asks what to call the account.
    Naming { email: String },
    /// An account has the address: asks for its passkey.
    LogIn { email: String },
    /// Waits for the passkey, and the profile's worker after it.
    Waiting,
    /// The account waits for its emailed link.
    Confirming { email: String },
    /// What went wrong, and a way to try again.
    Failed { message: String },
    /// Asks which Tonk holds the account, to sign in through it.
    Via,
}

impl Stage {
    /// What the page says the panel is at, for what reads it.
    fn name(&self) -> &'static str {
        match self {
            Stage::Address { .. } | Stage::Checking { .. } => "address",
            Stage::Naming { .. } => "naming",
            Stage::LogIn { .. } => "log-in",
            Stage::Waiting => "ceremony",
            Stage::Confirming { .. } => "confirming",
            Stage::Failed { .. } => "failed",
            Stage::Via => "via",
        }
    }
}

struct Panel {
    root: HtmlElement,
    stage: Stage,
    /// The ceremony the person got to, for the worker to hear if they
    /// leave without signing in.
    kind: &'static str,
    /// What the browser said of the last ceremony it refused.
    refused: Option<&'static str>,
    /// Whether the persona merged.
    signed_in: bool,
    /// Who waits on the sign-in.
    done: Vec<Box<dyn FnOnce(bool)>>,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// Answer the profile's worker when it asks this page to sign the person
/// in. A page inside another hears nothing: only the top page runs a
/// passkey.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let nested = window
        .parent()
        .ok()
        .flatten()
        .is_some_and(|parent| JsValue::from(parent) != JsValue::from(window.clone()));
    if nested {
        return;
    }
    let listener = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        let asked = js_sys::Reflect::get(&event.data(), &"type".into())
            .ok()
            .and_then(|kind| kind.as_string());
        if asked.as_deref() == Some("sign-in") {
            open(|_| {});
        }
    });
    let _ = window
        .navigator()
        .service_worker()
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    listener.forget();
}

/// Ask the person to sign in, and tell `done` whether they did. A panel
/// already up keeps its place, and tells `done` too.
pub fn open(done: impl FnOnce(bool) + 'static) {
    let opened = PANEL.with_borrow_mut(|panel| {
        if let Some(panel) = panel.as_mut() {
            panel.done.push(Box::new(done));
            return Ok(());
        }
        let root = create()?;
        *panel = Some(Panel {
            root,
            stage: Stage::Address {
                email: String::new(),
                says: None,
            },
            kind: LOG_IN,
            refused: None,
            signed_in: false,
            done: vec![Box::new(done)],
        });
        Err(())
    });
    if opened.is_err() {
        render();
    }
}

/// The panel's element, in the page, hearing what is done in it.
fn create() -> Result<HtmlElement, ()> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or(())?;
    if document.get_element_by_id("tonk-register-styles").is_none() {
        let style = document.create_element("style").map_err(|_| ())?;
        style.set_id("tonk-register-styles");
        style.set_text_content(Some(STYLES));
        document
            .head()
            .ok_or(())?
            .append_child(&style)
            .map_err(|_| ())?;
    }
    let root = document
        .create_element("div")
        .map_err(|_| ())?
        .dyn_into::<HtmlElement>()
        .map_err(|_| ())?;
    root.set_id(PANEL_ID);
    root.set_class_name("tonk-ceremony");
    let _ = root.set_attribute("data-anchored", "");
    let _ = root.set_attribute("role", "dialog");
    let _ = root.set_attribute("aria-modal", "true");
    let _ = root.set_attribute("aria-label", "sign in");
    hear(&root, "click", clicked);
    hear(&root, "submit", submitted);
    hear(&root, "input", typed);
    hear(&root, "keydown", |event: &Event| {
        if event
            .dyn_ref::<KeyboardEvent>()
            .is_some_and(|key| key.key() == "Escape")
        {
            close();
        }
    });
    document
        .body()
        .ok_or(())?
        .append_child(&root)
        .map_err(|_| ())?;
    Ok(root)
}

fn hear(root: &HtmlElement, kind: &str, handler: fn(&Event)) {
    let listener = Closure::<dyn FnMut(Event)>::new(move |event: Event| handler(&event));
    let _ = root.add_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
    listener.forget();
}

/// Move the panel to `stage`.
fn show(stage: Stage) {
    PANEL.with_borrow_mut(|panel| {
        if let Some(panel) = panel.as_mut() {
            panel.stage = stage;
        }
    });
    render();
}

fn render() {
    let Some((root, stage, kind)) = PANEL.with_borrow(|panel| {
        panel
            .as_ref()
            .map(|panel| (panel.root.clone(), panel.stage.clone(), panel.kind))
    }) else {
        return;
    };
    let _ = root.set_attribute("data-registration", stage.name());
    let _ = root.set_attribute("data-kind", kind);
    root.set_inner_html(&markup(&stage));
    if let Ok(Some(field)) = root.query_selector("input:not([readonly])")
        && let Ok(field) = field.dyn_into::<HtmlInputElement>()
    {
        let _ = field.focus();
        enable(&root, &field);
    }
}

const HEAD: &str = r#"<div class="m-head mblk"><span class="fabb-task-disc" aria-hidden="true"></span><span class="fabb-task-title">sign in</span></div>"#;
const CANCEL: &str = r#"<button class="ghost" type="button" data-act="cancel">cancel</button>"#;

fn markup(stage: &Stage) -> String {
    match stage {
        Stage::Address { email, says } => {
            let says = says.as_deref().map(escape).unwrap_or_else(|| {
                r#"Enter your email to continue. We’ll check whether you already have a Tonk account. Your account is on another Tonk? <a href="" data-act="via">Sign in through it</a>."#.to_owned()
            });
            address(
                email,
                &says,
                r#"<button class="obtn" id="tonk-register-action" type="submit" disabled>continue</button>"#,
                "",
            )
        }
        Stage::Checking { email } => address(
            email,
            "Checking this address…",
            r#"<button class="obtn wait" id="tonk-register-action" type="button" disabled aria-busy="true">checking</button>"#,
            " readonly",
        ),
        Stage::Naming { email } => format!(
            r#"<form class="ocol" data-form="naming"><div class="ostack">{HEAD}<div class="orow mblk"><span class="k">email</span><span class="v">{email}</span></div><div class="orow mblk editing"><span class="k">display name</span><span class="v"><input class="ed" name="name" type="text" enterkeyhint="go" autocomplete="name" aria-label="display name" required maxlength="50"></span></div></div><div class="oexp mblk"><p aria-live="polite">What should people see when you share a space? Your passkey comes next.</p></div>{CANCEL}<button class="obtn" id="tonk-register-action" type="submit" disabled>create a passkey</button></form>"#,
            email = escape(email),
        ),
        Stage::LogIn { email } => format!(
            r#"<div class="ocol"><div class="ostack">{HEAD}<div class="orow mblk"><span class="k">email</span><span class="v">{email}</span></div></div><div class="oexp mblk"><p aria-live="polite">This address has a Tonk account. Use its passkey to sign in.</p></div>{CANCEL}<button class="obtn" id="tonk-register-action" type="button" data-act="log-in">log in with passkey</button></div>"#,
            email = escape(email),
        ),
        Stage::Waiting => format!(
            r#"<div class="ocol"><div class="ostack">{HEAD}</div><div class="oexp mblk"><p aria-live="polite">Waiting for your passkey…</p></div>{CANCEL}<button class="obtn wait" id="tonk-register-action" type="button" disabled aria-busy="true">waiting for device</button></div>"#
        ),
        Stage::Confirming { .. } => format!(
            r#"<div class="ocol"><div class="ostack">{HEAD}<div class="orow mblk"><span class="k">email</span><span class="v">awaiting confirmation</span></div></div><div class="oexp mblk"><p aria-live="polite">Open the confirmation link we emailed you to finish. You can leave this page open.</p></div><button class="ghost" type="button" data-act="close">close</button><button class="obtn" id="tonk-register-action" type="button" hidden></button></div>"#
        ),
        Stage::Failed { message } => format!(
            r#"<div class="ocol"><div class="ostack">{HEAD}</div><div class="oexp mblk"><p aria-live="polite">{message}</p></div>{CANCEL}<button class="obtn" id="tonk-register-action" type="button" data-act="retry">try again</button></div>"#,
            message = escape(message),
        ),
        Stage::Via => format!(
            r#"<form class="ocol" data-form="via"><div class="ostack">{HEAD}<div class="orow mblk editing"><span class="k">tonk address</span><span class="v"><input class="ed" name="via" type="text" inputmode="url" enterkeyhint="go" autocomplete="url" spellcheck="false" aria-label="tonk address" placeholder="tonk.network" required></span></div></div><div class="oexp mblk"><p aria-live="polite">Enter the address of the Tonk that holds your account. You’ll approve this browser there with your passkey.</p></div>{CANCEL}<button class="obtn" id="tonk-register-action" type="submit" disabled>continue</button></form>"#
        ),
    }
}

fn address(email: &str, says: &str, action: &str, readonly: &str) -> String {
    format!(
        r#"<form class="ocol" data-form="address"><div class="ostack">{HEAD}<div class="orow mblk editing"><span class="k">email address</span><span class="v"><input class="ed" name="email" type="email" inputmode="email" enterkeyhint="go" autocomplete="email" aria-label="email address" value="{email}" required{readonly}></span></div></div><div class="oexp mblk"><p aria-live="polite">{says}</p></div>{CANCEL}{action}</form>"#,
        email = escape(email),
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Offer the step once what it needs is typed.
fn enable(root: &HtmlElement, field: &HtmlInputElement) {
    if let Ok(Some(action)) = root.query_selector("form #tonk-register-action")
        && !action.class_list().contains("wait")
    {
        let ready = !field.value().trim().is_empty() && field.check_validity();
        let _ = action.toggle_attribute_with_force("disabled", !ready);
    }
}

fn typed(event: &Event) {
    let Some(field) = event
        .target()
        .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };
    if let Some(root) = PANEL.with_borrow(|panel| panel.as_ref().map(|panel| panel.root.clone())) {
        enable(&root, &field);
    }
}

fn clicked(event: &Event) {
    let Some(act) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|target| target.closest("[data-act]").ok().flatten())
        .and_then(|target| target.get_attribute("data-act"))
    else {
        return;
    };
    event.prevent_default();
    match act.as_str() {
        "cancel" | "close" => close(),
        "retry" => show(Stage::Address {
            email: String::new(),
            says: None,
        }),
        "via" => show(Stage::Via),
        "log-in" => {
            let email = PANEL.with_borrow(|panel| match panel.as_ref().map(|panel| &panel.stage) {
                Some(Stage::LogIn { email }) => email.clone(),
                _ => String::new(),
            });
            log_in(&email);
        }
        _ => {}
    }
}

fn submitted(event: &Event) {
    event.prevent_default();
    let Some(form) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return;
    };
    let value = |name: &str| {
        form.query_selector(&format!("input[name={name}]"))
            .ok()
            .flatten()
            .and_then(|field| field.dyn_into::<HtmlInputElement>().ok())
            .map(|field| field.value().trim().to_owned())
            .unwrap_or_default()
    };
    match form.get_attribute("data-form").as_deref() {
        Some("address") => {
            let email = value("email");
            if !email.is_empty() {
                look_up(email);
            }
        }
        Some("naming") => {
            let name = value("name");
            let email = PANEL.with_borrow(|panel| match panel.as_ref().map(|panel| &panel.stage) {
                Some(Stage::Naming { email }) => email.clone(),
                _ => String::new(),
            });
            if !name.is_empty() && !email.is_empty() {
                create_account(&email, name);
            }
        }
        Some("via") => {
            let via = value("via");
            if let Some(origin) = via_origin(&via)
                && let Some(window) = web_sys::window()
            {
                let target = format!(
                    "/settings/link?via={}",
                    js_sys::encode_uri_component(&origin)
                );
                close();
                let _ = window.location().assign(&target);
            }
        }
        _ => {}
    }
}

/// The origin of the Tonk `typed` names: a bare host is an `https` one.
fn via_origin(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let address = if typed.contains("://") {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    };
    web_sys::Url::new(&address).ok().map(|url| url.origin())
}

/// What an address answers: whether an account has it.
#[derive(Debug, PartialEq)]
enum Lookup {
    Free,
    /// An account has it, and is served.
    Active,
    /// An account has it, and waits for its emailed link.
    Pending,
    Suspended,
    Invalid,
    Unavailable,
}

/// Split an address into the `(domain, local)` pair the lookup path
/// names, or `None` when it is not one.
fn split_address(email: &str) -> Option<(String, String)> {
    let trimmed = email.trim();
    if trimmed.len() > 254 {
        return None;
    }
    let (local, domain) = trimmed.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') || local.contains('/') {
        return None;
    }
    Some((domain.to_ascii_lowercase(), local.to_ascii_lowercase()))
}

/// What the access service's lookup says of an address, by its status.
fn lookup_of(status: u16) -> Lookup {
    match status {
        200 => Lookup::Active,
        202 => Lookup::Pending,
        404 => Lookup::Free,
        410 => Lookup::Suspended,
        _ => Lookup::Unavailable,
    }
}

async fn lookup(email: &str) -> Lookup {
    let Some((domain, local)) = split_address(email) else {
        return Lookup::Invalid;
    };
    let Some(origin) = web_sys::window().and_then(|window| window.location().origin().ok()) else {
        return Lookup::Unavailable;
    };
    let url = format!(
        "{origin}/customer/{}/{}/did.json",
        js_sys::encode_uri_component(&domain),
        js_sys::encode_uri_component(&local)
    );
    match reqwest::get(url).await {
        Ok(answer) => lookup_of(answer.status().as_u16()),
        Err(_) => Lookup::Unavailable,
    }
}

fn look_up(email: String) {
    show(Stage::Checking {
        email: email.clone(),
    });
    wasm_bindgen_futures::spawn_local(async move {
        let found = lookup(&email).await;
        let says = |said: &str| Stage::Address {
            email: email.clone(),
            says: Some(said.to_owned()),
        };
        match found {
            Lookup::Free => show(Stage::Naming { email }),
            // The click that asked may still count: then the passkey is
            // asked for on it, as the person expects, and otherwise on
            // the next.
            Lookup::Active | Lookup::Pending if activated() => log_in(&email),
            Lookup::Active | Lookup::Pending => show(Stage::LogIn { email }),
            Lookup::Suspended => show(says(
                "This account is suspended, so it cannot be used on this device. Contact support to restore it.",
            )),
            Lookup::Invalid => show(says("That is not an email address Tonk can look up.")),
            Lookup::Unavailable => show(says(
                "Tonk could not check this address. Try again in a moment.",
            )),
        }
    });
}

/// Whether this document holds a live user activation.
fn activated() -> bool {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window.navigator(), &"userActivation".into()).ok())
        .and_then(|activation| js_sys::Reflect::get(&activation, &"isActive".into()).ok())
        .and_then(|active| active.as_bool())
        .unwrap_or(false)
}

/// What this device is called in the account's device list.
fn device_name() -> String {
    let Some(navigator) = web_sys::window().map(|window| window.navigator()) else {
        return String::new();
    };
    tonk_common::device_label::from_navigator(
        &navigator.user_agent().unwrap_or_default(),
        &navigator.platform().unwrap_or_default(),
        navigator.max_touch_points(),
    )
}

/// The `/ucan/` endpoint of this deployment, which an account syncs
/// through.
fn remote() -> String {
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default();
    format!("{origin}/ucan/")
}

fn create_account(email: &str, name: String) {
    let device = device_name();
    let remote = remote();
    let intent = CustodyIntent::CreateAccount(AccountCreation {
        email: email.to_owned(),
        display_name: Some(name),
        device_name: device.clone(),
        remote: remote.clone(),
        provider: remote,
        created_on: Some(device),
    });
    run(CREATE, "createPasskey", intent, email.to_owned());
}

fn log_in(email: &str) {
    let remote = remote();
    let intent = CustodyIntent::Login(DeviceLink {
        device_name: device_name(),
        endpoint: remote.clone(),
        provider: remote,
    });
    run(LOG_IN, "usePasskey", intent, email.to_owned());
}

/// Run the passkey ceremony `method` for `intent`, now, inside the click
/// that asked, and wait for the profile's worker to sign the person in
/// with what it derives.
fn run(kind: &'static str, method: &'static str, intent: CustodyIntent, email: String) {
    PANEL.with_borrow_mut(|panel| {
        if let Some(panel) = panel.as_mut() {
            panel.kind = kind;
        }
    });
    let mediation = custody_relay::begin(method, intent);
    show(Stage::Waiting);
    wasm_bindgen_futures::spawn_local(async move {
        let finished = match mediation {
            Ok(mediation) => mediation.finish().await,
            Err(error) => Err(error),
        };
        match finished {
            Ok(()) if kind == CREATE => {
                signed_in();
                confirm(email);
            }
            Ok(()) => {
                signed_in();
                close();
            }
            Err(error) => failed(&error, email),
        }
    });
}

/// How often the panel asks whether the emailed link was opened.
const CONFIRM_EVERY_MS: i32 = 4_000;

async fn sleep(ms: i32) {
    let waited = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(waited).await;
}

/// Wait for the account at `email` to be served: its emailed link can be
/// opened on another device, which this page hears of only from the
/// service. The panel goes away once it is, or once the person puts it
/// away.
fn confirm(email: String) {
    show(Stage::Confirming {
        email: email.clone(),
    });
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            sleep(CONFIRM_EVERY_MS).await;
            let waiting = PANEL.with_borrow(|panel| {
                matches!(
                    panel.as_ref().map(|panel| &panel.stage),
                    Some(Stage::Confirming { .. })
                )
            });
            if !waiting {
                return;
            }
            if lookup(&email).await == Lookup::Active {
                close();
                return;
            }
        }
    });
}

/// The persona merged: whoever waits on the sign-in hears so.
fn signed_in() {
    let done = PANEL.with_borrow_mut(|panel| {
        panel
            .as_mut()
            .map(|panel| {
                panel.signed_in = true;
                std::mem::take(&mut panel.done)
            })
            .unwrap_or_default()
    });
    for done in done {
        done(true);
    }
}

fn failed(error: &CeremonyError, email: String) {
    if let Some(CustodyDenial::AwaitingActivation) = error.denial {
        signed_in();
        confirm(email);
        return;
    }
    let message = match &error.denial {
        Some(CustodyDenial::Suspended(_)) => {
            "This account is suspended, so it cannot be used on this device. Contact support to restore it."
                .to_owned()
        }
        Some(CustodyDenial::NotProvisioned(_)) => {
            "This account is not set up for syncing yet. Finish creating it on the browser that holds its passkey, then try again."
                .to_owned()
        }
        Some(_) => error.message.clone(),
        None => match error.refusal.map(custody_relay::refusal_name) {
            Some(name) => {
                PANEL.with_borrow_mut(|panel| {
                    if let Some(panel) = panel.as_mut() {
                        panel.refused = Some(name);
                    }
                });
                refused(name).to_owned()
            }
            None => error.message.clone(),
        },
    };
    show(Stage::Failed { message });
}

/// What a passkey ceremony the browser refused says to the person, by the
/// browser's `name` for the refusal.
fn refused(name: &str) -> &'static str {
    match name {
        "NotAllowedError" => "The passkey prompt was closed. Try again when you are ready.",
        "InvalidStateError" => {
            "This device already has a passkey for that account. Log in with it instead."
        }
        "NotSupportedError" => "This browser cannot use passkeys.",
        "SecurityError" => "Passkeys are not available on this page.",
        _ => "The passkey did not finish. Try again.",
    }
}

/// Put the panel away. A person who leaves without signing in is told to
/// the profile's worker, so the kernel that asked stops waiting, and to
/// whoever waits here.
fn close() {
    let Some(panel) = PANEL.with_borrow_mut(Option::take) else {
        return;
    };
    panel.root.remove();
    if panel.signed_in {
        return;
    }
    custody_relay::tell_refused(panel.kind, panel.refused.unwrap_or("AbortError"));
    for done in panel.done {
        done(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn it_reads_every_status_the_lookup_answers() {
        assert_eq!(lookup_of(200), Lookup::Active);
        assert_eq!(lookup_of(202), Lookup::Pending, "enrolled, link unopened");
        assert_eq!(lookup_of(404), Lookup::Free);
        assert_eq!(lookup_of(410), Lookup::Suspended);
        assert_eq!(lookup_of(429), Lookup::Unavailable, "not an answer");
    }

    #[wasm_bindgen_test]
    fn it_looks_up_only_what_splits_into_a_lookup_path() {
        assert_eq!(
            split_address(" Someone@Example.com "),
            Some(("example.com".to_owned(), "someone".to_owned()))
        );
        assert_eq!(split_address("no-at-sign"), None);
        assert_eq!(split_address("a/b@example.com"), None);
    }

    #[wasm_bindgen_test]
    fn it_signs_in_through_the_origin_of_a_bare_host() {
        assert_eq!(
            via_origin("tonk.network").as_deref(),
            Some("https://tonk.network")
        );
        assert_eq!(
            via_origin("http://localhost:8080/path").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(via_origin("  "), None);
    }

    #[wasm_bindgen_test]
    fn it_escapes_what_was_typed() {
        assert!(
            markup(&Stage::Naming {
                email: "<b>@x".to_owned()
            })
            .contains("&lt;b&gt;@x")
        );
    }
}
