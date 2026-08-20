//! `<tonk-invite-link>` — turn a pasted invite URL into a local `/join`.
//!
//! An invite link carries everything a join needs in its query and
//! fragment — the delegation chain, the space's sync remote, the
//! revocation relay — so the origin it was minted on is only a doorway.
//! Pasting a link minted elsewhere (say, production) into this deployment
//! joins the space here while its blocks keep coming from the remote the
//! invite names — which is exactly what local debugging against real
//! data wants.
//!
//! Static notation can't read an input, rewrite a URL, or navigate, so
//! this dumb element bridges the gap the same way [`super::default_remote`]
//! does for the origin: it sits inside the paste `<form>`, intercepts its
//! `submit`, swaps the pasted link's origin for this page's own `/join`
//! route (keeping query + fragment verbatim), and hands the result to the
//! ordinary navigation path. All join policy stays in the worker's claim
//! pipeline.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Event, HtmlElement, window};

/// A retained submit-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type SubmitClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

/// The form-control `name` read when the element sets no `field`.
const DEFAULT_FIELD: &str = "url";

/// Attribute the element stamps with the outcome of the last submit —
/// `invalid` when the pasted text isn't an invite URL — so the template
/// can style an error state without any script of its own.
const STATE_ATTR: &str = "data-state";

#[derive(Default)]
pub(crate) struct TonkInviteLink {
    submit: SubmitClosure,
}

impl CustomElement for TonkInviteLink {
    fn shadow() -> bool {
        // Light DOM: the element must see its `<form>` ancestor.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Ok(Some(form)) = this.closest("form") else {
            return;
        };
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            // The sealed guest must never fall back to a native form
            // navigation, so the default is cancelled unconditionally
            // and validity only decides whether we navigate.
            event.prevent_default();
            submit(&host);
        }) as Box<dyn FnMut(Event)>);
        let _ = form.add_event_listener_with_callback("submit", listener.as_ref().unchecked_ref());
        *self.submit.borrow_mut() = Some(listener);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.submit.borrow_mut().take();
    }
}

fn submit(this: &HtmlElement) {
    let field = this
        .get_attribute("field")
        .unwrap_or_else(|| DEFAULT_FIELD.to_string());
    let pasted = read_field(this, &field).unwrap_or_default();
    match local_join_href(pasted.trim()) {
        Some(href) => {
            let _ = this.remove_attribute(STATE_ATTR);
            navigate(&href);
        }
        None => {
            let _ = this.set_attribute(STATE_ATTR, "invalid");
        }
    }
}

/// Read the `value` property of the form control named `field` within the
/// closest `<form>`. Property, not attribute: `<wa-input>` is a
/// form-associated custom element and carries its live value there.
fn read_field(this: &HtmlElement, field: &str) -> Option<String> {
    let form = this.closest("form").ok()??;
    let input = form.query_selector(&format!("[name=\"{field}\"]")).ok()??;
    js_sys::Reflect::get(input.as_ref(), &JsValue::from_str("value"))
        .ok()?
        .as_string()
}

/// Rewrite a pasted invite URL into this deployment's `/join` route,
/// keeping the query and fragment — where the invite's chain, remote,
/// and seed actually live — byte for byte. The pasted origin and path
/// are deliberately discarded: they only say where the link was minted.
fn local_join_href(pasted: &str) -> Option<String> {
    if pasted.is_empty() {
        return None;
    }
    let url = web_sys::Url::new(pasted).ok()?;
    let search = url.search();
    let hash = url.hash();
    // A bare origin (or any URL with neither query nor fragment) carries
    // no invite; refusing here keeps the error visible at the form
    // instead of bouncing through /join to its failure screen.
    if search.is_empty() && hash.is_empty() {
        return None;
    }
    Some(format!("/join{search}{hash}"))
}

/// Navigate through the guest's bridge when sealed (`window.tonk.navigate`),
/// falling back to a plain location assignment at the top-level shell.
fn navigate(href: &str) {
    let Some(win) = window() else { return };
    let bridged = js_sys::Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|tonk| js_sys::Reflect::get(&tonk, &JsValue::from_str("navigate")).ok())
        .and_then(|function| function.dyn_into::<js_sys::Function>().ok())
        .and_then(|function| {
            function
                .call1(&JsValue::NULL, &JsValue::from_str(href))
                .ok()
        });
    if bridged.is_none() {
        let _ = win.location().assign(href);
    }
}

/// Register `<tonk-invite-link>`. Idempotent.
pub(crate) fn register() {
    let Some(elements) = window().map(|w| w.custom_elements()) else {
        return;
    };
    if elements.get("tonk-invite-link").is_undefined() {
        TonkInviteLink::define("tonk-invite-link");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// The rewrite keeps query + fragment verbatim and discards the
    /// minting origin; inputs carrying no invite material are refused.
    #[dialog_common::test]
    async fn it_rewrites_a_foreign_invite_onto_the_local_join_route() {
        assert_eq!(
            local_join_href("https://staging.tonk.xyz/join?ucan=abc#seed"),
            Some("/join?ucan=abc#seed".to_string()),
        );
        assert_eq!(
            local_join_href("https://tonk.network/join#onlyfragment"),
            Some("/join#onlyfragment".to_string()),
        );
        assert_eq!(local_join_href(""), None, "empty paste is refused");
        assert_eq!(
            local_join_href("https://staging.tonk.xyz/"),
            None,
            "a bare origin carries no invite"
        );
        assert_eq!(local_join_href("not a url"), None);
    }

    /// Submitting the form navigates nowhere on an invalid paste and
    /// stamps the error state the template styles.
    #[dialog_common::test]
    async fn it_marks_an_invalid_paste_instead_of_navigating() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let form = document.create_element("form").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("name", "url").unwrap();
        input
            .set_attribute("value", "definitely not an invite")
            .unwrap();
        let element = document.create_element("tonk-invite-link").unwrap();
        form.append_child(&input).unwrap();
        form.append_child(&element).unwrap();
        body.append_child(&form).unwrap();

        let event = web_sys::Event::new_with_event_init_dict(
            "submit",
            web_sys::EventInit::new().cancelable(true),
        )
        .unwrap();
        let _ = form.dispatch_event(&event);

        assert!(
            event.default_prevented(),
            "the sealed guest must never run a native form submit"
        );
        assert_eq!(
            element.get_attribute(STATE_ATTR).as_deref(),
            Some("invalid"),
        );

        form.remove();
    }
}
