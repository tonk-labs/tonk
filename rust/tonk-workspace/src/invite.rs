//! `<tonk-invite>` — generates an artifact-scoped invite link.
//!
//! The empty-artifact canvas hands a freshly created sheet to an agent
//! via a link that targets that specific artifact:
//!
//! ```text
//! http://<host>/space/<repo>/<artifact-entity>@<concept>?access=…#…
//! ```
//!
//! On click the element mints an invite through the existing repo-invite
//! endpoint (`POST /api/repository/{repo}/invite`), supplying an
//! artifact-targeted `base_url` so the returned URL points at the
//! artifact's view. It then renders the link inline. Nothing is stored:
//! the invite (whose fragment is a private-key seed) lives only in the
//! HTTP response and in this element's DOM for the session — re-clicking
//! mints a fresh one. See `plan/agent-invite.md`.
//!
//! It mirrors [`super::share`] (resolve the repo from a
//! `<tonk-repository>` ancestor; reuse its reskin attributes) but
//! resolves the link itself rather than dispatching intent, because the
//! result is a secret that must not cross the data model.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};
use web_sys::{Element, Event, HtmlElement, Request, RequestInit, Response, window};

use crate::ancestors::repo_from_ancestor;

/// A retained click-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type ClickClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

/// Default concept the link targets when `concept` is absent.
const DEFAULT_CONCEPT: &str = "tonk:artifact";

/// Per-element state. Holds the click closure so it lives as long as
/// the element and drops on disconnect.
#[derive(Default)]
pub(crate) struct TonkInvite {
    click: ClickClosure,
}

impl CustomElement for TonkInvite {
    fn shadow() -> bool {
        // Light DOM: the consuming view styles the button + result, and
        // the element must see its `<tonk-repository>` ancestor.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        ensure_button(this);
        install_click(this, &self.click);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.click.borrow_mut().take();
    }
}

/// Default CSS class the consuming view styles (the empty-artifact
/// canvas reuses the share-button class via `button-class`).
const BUTTON: &str = "workspace__share";
/// The button's icon slot — a `<wa-icon>` whose `name` is swapped for
/// the copy / success / error visual states, like `<wa-copy-button>`.
const ICON: &str = "tonk-invite__icon";
/// The container the resolved link renders into (the copy-failed
/// fallback).
const RESULT: &str = "tonk-invite__result";
const URL_FIELD: &str = "tonk-invite__url";

/// Web Awesome system-icon names for the three states, matching
/// `<wa-copy-button>`. The visible label plus the icon swap are the
/// feedback; no tooltip (it would just restate the label).
const ICON_IDLE: &str = "copy";
const ICON_SUCCESS: &str = "check";
const ICON_ERROR: &str = "xmark";
/// How long the success / error icon shows before reverting, matching
/// `<wa-copy-button>`'s 1s `feedbackDuration`.
const FEEDBACK_MS: i32 = 1000;

/// Build the trigger button as the element's only child: a `<wa-icon>`
/// (the copy glyph) plus an optional `label` span — modeled on
/// `<wa-copy-button>` so it reads as a copy control with success /
/// error feedback. Honors the `button-class` / `label` reskin
/// attributes. Idempotent.
fn ensure_button(this: &HtmlElement) -> Option<Element> {
    let document = window().and_then(|w| w.document())?;
    let button_class = this
        .get_attribute("button-class")
        .unwrap_or_else(|| BUTTON.to_owned());
    let selector = format!(
        ":scope > .{}",
        button_class.split_whitespace().next().unwrap_or(BUTTON)
    );
    if let Ok(Some(existing)) = this.query_selector(&selector) {
        return Some(existing);
    }
    let label = this.get_attribute("label");

    let button = document.create_element("button").ok()?;
    let _ = button.set_attribute("class", &button_class);
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("part", "button");
    let aria = label.as_deref().unwrap_or("Copy agent link");
    let _ = button.set_attribute("aria-label", aria);

    let icon = document.create_element("wa-icon").ok()?;
    let _ = icon.set_attribute("class", ICON);
    let _ = icon.set_attribute("library", "system");
    let _ = icon.set_attribute("name", ICON_IDLE);
    let _ = button.append_child(&icon);

    if let Some(text) = &label {
        let span = document.create_element("span").ok()?;
        span.set_text_content(Some(text));
        let _ = button.append_child(&span);
    }
    let _ = this.append_child(&button);
    Some(button)
}

/// Install the click listener: mint the invite and copy it to the
/// clipboard. The mint is async (an HTTP round-trip), but
/// `clipboard.writeText` after an `await` has lost the click's user
/// activation and would be rejected. So we hand the clipboard a
/// *pending promise* synchronously inside the click — a
/// `ClipboardItem` whose `text/plain` is the mint promise — and the
/// browser resolves it while the gesture is still valid. On failure
/// (unsupported browser, mint error) we fall back to revealing the URL
/// so the user can copy it manually.
fn install_click(this: &HtmlElement, slot: &ClickClosure) {
    let host = this.clone();
    let listener = Closure::wrap(Box::new(move |_event: Event| {
        copy_invite(&host);
    }) as Box<dyn FnMut(Event)>);

    let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    *slot.borrow_mut() = Some(listener);
}

/// Read the element's `artifact` / `concept` / repo context. `None`
/// when the artifact attribute or repository ancestor is missing.
fn link_context(host: &HtmlElement) -> Option<(String, String, String)> {
    let artifact = host.get_attribute("artifact").filter(|a| !a.is_empty())?;
    let concept = host
        .get_attribute("concept")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| DEFAULT_CONCEPT.to_owned());
    let repo = repo_from_ancestor(host)?;
    Some((repo, artifact, concept))
}

/// Synchronously hand the clipboard a `ClipboardItem` backed by the
/// pending mint promise, so the copy happens under the click's user
/// activation. If `clipboard.write` rejects (e.g. a browser without
/// promise-valued `ClipboardItem`), reveal the URL as a fallback.
fn copy_invite(host: &HtmlElement) {
    let Some((repo, artifact, concept)) = link_context(host) else {
        tonk_common::log!("tonk-invite: missing artifact or repository context");
        return;
    };
    let Some(clipboard) = window().map(|w| w.navigator().clipboard()) else {
        return;
    };

    // The mint promise, resolving to the invite URL string. Held by the
    // ClipboardItem; also reused for the success/failure UI below.
    let mint_repo = repo.clone();
    let url_promise: Promise = future_to_promise(async move {
        match mint(&mint_repo, &artifact, &concept).await {
            Some(url) => Ok(JsValue::from_str(&url)),
            None => Err(JsValue::from_str("invite mint failed")),
        }
    });

    // ClipboardItem({ "text/plain": <url promise> }). web-sys exposes no
    // constructor, so build it from the global.
    let Some(item) = clipboard_item(&url_promise) else {
        // No ClipboardItem support — fall back immediately.
        reveal_fallback(host, &url_promise);
        return;
    };
    let items = Array::of1(&item);

    let host_ok = host.clone();
    let host_err = host.clone();
    let promise_for_fallback = url_promise.clone();
    let write = clipboard.write(&items);
    let on_ok = Closure::once(move |_v: JsValue| flash_copied(&host_ok));
    let on_err = Closure::once(move |_e: JsValue| {
        tonk_common::log!("tonk-invite: clipboard.write rejected; revealing URL");
        flash_feedback(&host_err, ICON_ERROR);
        reveal_fallback(&host_err, &promise_for_fallback);
    });
    let _ = write.then2(&on_ok, &on_err);
    // One-shot promise callbacks; release the Rust handles and let the JS
    // wrappers live until the promise settles.
    on_ok.forget();
    on_err.forget();
}

/// Build `new ClipboardItem({ "text/plain": promise })`, or `None`
/// when the constructor is unavailable.
fn clipboard_item(text_promise: &Promise) -> Option<Object> {
    let win = window()?;
    let ctor = Reflect::get(win.as_ref(), &JsValue::from_str("ClipboardItem")).ok()?;
    if !ctor.is_function() {
        return None;
    }
    let dict = Object::new();
    Reflect::set(&dict, &JsValue::from_str("text/plain"), text_promise).ok()?;
    let args = Array::of1(&dict);
    let item = Reflect::construct(ctor.unchecked_ref::<js_sys::Function>(), &args).ok()?;
    item.dyn_into::<Object>().ok()
}

/// Flash the success state: swap the icon to a check, reverting after
/// the feedback window — the `<wa-copy-button>` behaviour.
fn flash_copied(host: &HtmlElement) {
    flash_feedback(host, ICON_SUCCESS);
}

/// Swap the button icon to `icon_name`, then revert to the idle copy
/// glyph after `FEEDBACK_MS`.
fn flash_feedback(host: &HtmlElement, icon_name: &str) {
    let Some(icon) = host.query_selector(&format!(".{ICON}")).ok().flatten() else {
        return;
    };
    let _ = icon.set_attribute("name", icon_name);

    // Revert after the feedback window.
    if let Some(win) = window() {
        let revert = Closure::once_into_js(move || {
            let _ = icon.set_attribute("name", ICON_IDLE);
        });
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            revert.unchecked_ref(),
            FEEDBACK_MS,
        );
    }
}

/// Fallback when the clipboard write can't happen: once the mint
/// promise resolves, render the URL + a copy control so the user can
/// copy it by hand.
fn reveal_fallback(host: &HtmlElement, url_promise: &Promise) {
    let host = host.clone();
    let fut = JsFuture::from(url_promise.clone());
    spawn_local(async move {
        if let Ok(value) = fut.await
            && let Some(url) = value.as_string()
        {
            render_result(&host, &url);
        }
    });
}

/// The artifact-targeted base URL the invite is minted onto: the
/// display route for `{entity}@{concept}` under the space. The worker
/// appends `?access=…#…` to this (`Invite::to_url`).
fn artifact_base_url(origin: &str, repo: &str, artifact: &str, concept: &str) -> String {
    format!("{origin}/space/{repo}/{artifact}@{concept}")
}

/// Pull the minted URL out of a `CreateInviteResponse` JSON body. The
/// response is an internally-tagged enum; both `open` and `scoped`
/// variants carry a `url` field.
fn url_from_response(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value.get("url")?.as_str().map(str::to_owned)
}

/// POST the existing repo-invite endpoint with an artifact-targeted
/// `base_url` and read the minted URL from the response. The worker is
/// origin-agnostic; the client owns the origin (same as the repo share
/// button — see `tonk-ui`'s `api::create_invite`).
async fn mint(repo: &str, artifact: &str, concept: &str) -> Option<String> {
    tonk_host::ready::wait().await;
    let win = window()?;
    let origin = win.location().origin().ok()?;
    let base_url = artifact_base_url(&origin, repo, artifact, concept);
    let endpoint = format!("{origin}/api/repository/{repo}/invite");
    let body = serde_json::json!({ "base_url": base_url }).to_string();

    let init = RequestInit::new();
    init.set_method("POST");
    let headers = web_sys::Headers::new().ok()?;
    let _ = headers.set("content-type", "application/json");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(&body));
    let request = Request::new_with_str_and_init(&endpoint, &init).ok()?;

    let resp: Response = JsFuture::from(win.fetch_with_request(&request))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !resp.ok() {
        tonk_common::log!("tonk-invite: POST {endpoint} -> {}", resp.status());
        return None;
    }
    let text = JsFuture::from(resp.text().ok()?).await.ok()?.as_string()?;
    url_from_response(&text)
}

/// Fallback render: replace the button with the resolved link + a copy
/// control, for when the clipboard write can't happen (unsupported
/// browser, lost activation). The happy path copies silently and never
/// shows the URL.
fn render_result(host: &HtmlElement, url: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    // Clear any prior content (the button or a stale result).
    host.set_inner_html("");

    let Ok(result) = document.create_element("div") else {
        return;
    };
    let _ = result.set_attribute("class", RESULT);

    let Ok(field) = document.create_element("code") else {
        return;
    };
    let _ = field.set_attribute("class", URL_FIELD);
    field.set_text_content(Some(url));
    let _ = result.append_child(&field);

    // Web Awesome copy button copies the literal value.
    if let Ok(copy) = document.create_element("wa-copy-button") {
        let _ = copy.set_attribute("value", url);
        let _ = result.append_child(&copy);
    }

    let _ = host.append_child(&result);
}

/// Register `<tonk-invite>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkInvite::define("tonk-invite");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-invite").is_undefined()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_builds_the_artifact_base_url() {
        let url = artifact_base_url(
            "http://127.0.0.1:8080",
            "home",
            "did:key:z6MkAeH7CbaZwurC1jswhPeaiwcwTLpBEkfwNiNbY86oNNfc",
            "tonk:artifact",
        );
        assert_eq!(
            url,
            "http://127.0.0.1:8080/space/home/did:key:z6MkAeH7CbaZwurC1jswhPeaiwcwTLpBEkfwNiNbY86oNNfc@tonk:artifact"
        );
    }

    #[dialog_common::test]
    fn it_reads_the_url_from_an_open_invite_response() {
        let body = r#"{"kind":"open","url":"http://h/space/home/did:key:zX@tonk:artifact?access=AAA#BBB"}"#;
        assert_eq!(
            url_from_response(body).as_deref(),
            Some("http://h/space/home/did:key:zX@tonk:artifact?access=AAA#BBB")
        );
    }

    #[dialog_common::test]
    fn it_reads_the_url_from_a_scoped_invite_response() {
        let body = r#"{"kind":"scoped","url":"http://h/x?access=AAA","audience":"did:key:zAud"}"#;
        assert_eq!(
            url_from_response(body).as_deref(),
            Some("http://h/x?access=AAA")
        );
    }

    #[dialog_common::test]
    fn it_returns_none_for_a_response_without_a_url() {
        assert!(url_from_response(r#"{"kind":"error"}"#).is_none());
        assert!(url_from_response("not json").is_none());
    }

    /// The element projects a copy button — a copy icon plus the
    /// `label` — on connect, under the custom button class.
    #[dialog_common::test]
    async fn it_projects_a_copy_button() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let el = document.create_element("tonk-invite").unwrap();
        el.set_attribute("artifact", "did:key:zX").unwrap();
        el.set_attribute("button-class", "empty-artifact__share")
            .unwrap();
        el.set_attribute("label", "copy").unwrap();
        body.append_child(&el).unwrap();

        let button = el
            .query_selector(".empty-artifact__share")
            .unwrap()
            .expect("button projected on connect");
        assert!(
            button.text_content().unwrap_or_default().contains("copy"),
            "labelled button should carry the label text",
        );
        let icon = el
            .query_selector(&format!(".{ICON}"))
            .unwrap()
            .expect("copy icon");
        assert_eq!(icon.get_attribute("name").as_deref(), Some(ICON_IDLE));
        el.remove();
    }

    /// `flash_copied` swaps the icon to a check (the success feedback).
    /// It reverts on a timer, which we don't wait for here — we assert
    /// the flashed state.
    #[dialog_common::test]
    async fn it_flashes_copied_feedback() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let el = document.create_element("tonk-invite").unwrap();
        el.set_attribute("label", "copy").unwrap();
        body.append_child(&el).unwrap();
        let el_html: HtmlElement = el.clone().dyn_into().unwrap();

        flash_copied(&el_html);

        let icon = el.query_selector(&format!(".{ICON}")).unwrap().unwrap();
        assert_eq!(icon.get_attribute("name").as_deref(), Some(ICON_SUCCESS));
        el.remove();
    }

    /// The fallback renderer (used when the clipboard write can't run)
    /// replaces the button with the link + a copy button.
    #[dialog_common::test]
    async fn it_reveals_the_link_as_a_fallback() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let el = document.create_element("tonk-invite").unwrap();
        body.append_child(&el).unwrap();
        let el_html: HtmlElement = el.clone().dyn_into().unwrap();

        let url = "http://h/space/home/did:key:zX@tonk:artifact?access=AAA#BBB";
        render_result(&el_html, url);

        let field = el
            .query_selector(&format!(".{URL_FIELD}"))
            .unwrap()
            .expect("url field rendered");
        assert_eq!(field.text_content().as_deref(), Some(url));
        assert!(
            el.query_selector("wa-copy-button").unwrap().is_some(),
            "a copy control is rendered",
        );
        assert!(
            el.query_selector(".empty-artifact__share")
                .unwrap()
                .is_none(),
            "the trigger button is replaced by the result",
        );
        el.remove();
    }
}
