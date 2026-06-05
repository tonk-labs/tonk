//! `<tonk-share>` — a share-this-repo control for the workspace top
//! bar.
//!
//! Deliberately dumb: it renders a single icon button and, on click,
//! emits *intent*. It walks up to the nearest `<tonk-repository
//! name="…">` ancestor (the display route wraps the whole view in one,
//! see `tonk-ui`'s `display.rs`), reads its `name`, and dispatches a
//! bubbling, composed `tonk:share` `CustomEvent` carrying `{ repo }`.
//!
//! It knows nothing about invites, UCANs, or the dialog — the app
//! shell owns that policy and listens for `tonk:share` on the window.
//! This mirrors the `<tonk-sheet-binder>` → `activate` pattern (see
//! [`super::binder`]): the element dispatches an event, the consumer
//! decides what it means.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CustomEvent, CustomEventInit, Element, Event, HtmlElement, window};

use crate::ancestors::repo_from_ancestor;

/// A retained click-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type ClickClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

/// The event name the shell listens for to open the invite dialog.
/// The first `tonk:`-prefixed window bridge from a workspace element.
const SHARE_EVENT: &str = "tonk:share";

/// Per-element state. Holds the click closure so it lives as long as
/// the element and drops on disconnect.
#[derive(Default)]
pub(crate) struct TonkShare {
    click: ClickClosure,
}

impl CustomElement for TonkShare {
    fn shadow() -> bool {
        // Light DOM: the consuming workspace view styles the button
        // (`.workspace__share`) and the element must see its
        // `<tonk-repository>` ancestor via `closest`.
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

/// CSS class the consuming workspace view styles.
const BUTTON: &str = "workspace__share";

/// Inline share-nodes glyph (three connected nodes), drawn with
/// `currentColor` so it follows the button's text colour.
const GLYPH: &str = r#"<svg class="workspace__share-glyph" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="12" r="3"></circle><circle cx="18" cy="5" r="3"></circle><circle cx="18" cy="19" r="3"></circle><line x1="8.6" y1="10.6" x2="15.4" y2="6.4"></line><line x1="8.6" y1="13.4" x2="15.4" y2="17.6"></line></svg>"#;

/// Find or create the share button as the element's only child.
/// Idempotent — a reconnect reuses the existing button.
fn ensure_button(this: &HtmlElement) -> Option<Element> {
    let document = window().and_then(|w| w.document())?;
    if let Ok(Some(existing)) = this.query_selector(&format!(":scope > .{BUTTON}")) {
        return Some(existing);
    }
    let button = document.create_element("button").ok()?;
    let _ = button.set_attribute("class", BUTTON);
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("part", "button");
    let _ = button.set_attribute("aria-label", "Share this repo");
    let _ = button.set_attribute("title", "Share this repo");
    button.set_inner_html(GLYPH);
    let _ = this.append_child(&button);
    Some(button)
}

/// Install the click listener: resolve the repo from the nearest
/// `<tonk-repository>` ancestor and dispatch `tonk:share`.
fn install_click(this: &HtmlElement, slot: &ClickClosure) {
    let host = this.clone();
    let listener = Closure::wrap(Box::new(move |_event: Event| {
        let repo = repo_from_ancestor(&host).unwrap_or_default();
        dispatch_share(&host, &repo);
    }) as Box<dyn FnMut(Event)>);

    let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    *slot.borrow_mut() = Some(listener);
}

/// Dispatch `tonk:share` with `detail = { repo }`, bubbling + composed
/// so it crosses any light-DOM boundary up to the window, where the
/// shell listens.
fn dispatch_share(host: &HtmlElement, repo: &str) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("repo"),
        &JsValue::from_str(repo),
    );
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(event) = CustomEvent::new_with_event_init_dict(SHARE_EVENT, &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// Register `<tonk-share>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkShare::define("tonk-share");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-share").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::CustomEvent;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_dispatches_tonk_share_with_the_ancestor_repo() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        // <tonk-repository name="pictures"><tonk-share></tonk-share></tonk-repository>
        // mirrors the display route, which wraps the view in a named
        // `<tonk-repository>`.
        let repo = document.create_element("tonk-repository").unwrap();
        repo.set_attribute("name", "pictures").unwrap();
        let share = document.create_element("tonk-share").unwrap();
        repo.append_child(&share).unwrap();
        // Appending an already-defined custom element runs its
        // connectedCallback synchronously, so the button is injected by
        // the time `append_child` returns.
        body.append_child(&repo).unwrap();

        // Capture the next `tonk:share` event's `detail.repo` off the
        // window — the bubbling + composed event must reach it.
        let captured: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let sink = captured.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let event = event.dyn_into::<CustomEvent>().unwrap();
            let repo = js_sys::Reflect::get(&event.detail(), &JsValue::from_str("repo"))
                .ok()
                .and_then(|value| value.as_string());
            *sink.borrow_mut() = repo;
        }) as Box<dyn FnMut(Event)>);
        window()
            .unwrap()
            .add_event_listener_with_callback("tonk:share", listener.as_ref().unchecked_ref())
            .unwrap();

        let button = share
            .query_selector(".workspace__share")
            .unwrap()
            .expect("share button injected on connect");
        button.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(captured.borrow().as_deref(), Some("pictures"));

        window()
            .unwrap()
            .remove_event_listener_with_callback("tonk:share", listener.as_ref().unchecked_ref())
            .unwrap();
        repo.remove();
    }

    #[dialog_common::test]
    async fn it_dispatches_an_empty_repo_with_no_repository_ancestor() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        // No `<tonk-repository>` ancestor — the element falls back to an
        // empty repo, leaving the shell to resolve it from the route.
        let share = document.create_element("tonk-share").unwrap();
        body.append_child(&share).unwrap();

        let captured: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let sink = captured.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let event = event.dyn_into::<CustomEvent>().unwrap();
            let repo = js_sys::Reflect::get(&event.detail(), &JsValue::from_str("repo"))
                .ok()
                .and_then(|value| value.as_string());
            *sink.borrow_mut() = repo;
        }) as Box<dyn FnMut(Event)>);
        window()
            .unwrap()
            .add_event_listener_with_callback("tonk:share", listener.as_ref().unchecked_ref())
            .unwrap();

        let button = share
            .query_selector(".workspace__share")
            .unwrap()
            .expect("share button injected on connect");
        button.dyn_ref::<HtmlElement>().unwrap().click();

        assert_eq!(captured.borrow().as_deref(), Some(""));

        window()
            .unwrap()
            .remove_event_listener_with_callback("tonk:share", listener.as_ref().unchecked_ref())
            .unwrap();
        share.remove();
    }
}
