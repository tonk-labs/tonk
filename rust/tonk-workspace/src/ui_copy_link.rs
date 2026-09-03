//! `<ui-copy-link>` — a verb that copies a URL and answers in place.
//!
//! The word IS the feedback: it becomes "copied" (or "couldn't copy") for a
//! moment and then goes back to offering. No toast, no icon swap — the same
//! word-answers grammar the FAB's share row uses.
//!
//! ## What it copies, and what it does not
//!
//! The `url` it is given, verbatim. On a Hub row that is the space's own
//! address, which is a bookmark and a pointer for people who are ALREADY
//! members — it is not an invite, and it grants nobody access. Minting an
//! invite is the bar's `share`, which delegates authority and can be refused;
//! this cannot fail in that way because it asks for nothing.
//!
//! Relative URLs are resolved against the page so what lands on the clipboard
//! is something that can be pasted somewhere else and still work.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, HtmlElement, window};

/// How long the answer stands before the verb goes back to offering.
const ANSWER_MS: i32 = 1_200;

/// The class this element renders on its button, for the host view to style.
const VERB_CLASS: &str = "copy-verb";

/// The resting label. Overridable with `label`, so the same element can read
/// as "copy link" in one place and something else in another.
const DEFAULT_LABEL: &str = "copy link";

type ClickClosure = Closure<dyn FnMut(web_sys::Event)>;

/// Per-element state.
#[derive(Default)]
pub(crate) struct UiCopyLink {
    click: Option<ClickClosure>,
    /// The pending revert, so a second click restarts the timer instead of
    /// letting the first one snap the word back mid-answer.
    revert: Rc<RefCell<Option<i32>>>,
}

impl CustomElement for UiCopyLink {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["label"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        // Reuse an existing control rather than appending a second one:
        // `inject_children` runs again whenever the element is re-created or
        // re-parented, and an unguarded append stacks duplicates. Mirrors
        // `ui_sync_status::paint`.
        if button_of(this).is_some() {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(button) = document.create_element("button") else {
            return;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("class", VERB_CLASS);
        button.set_text_content(Some(&label_of(this)));
        let _ = this.append_child(&button);
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        if self.click.is_some() {
            return;
        }
        let host = this.clone();
        let revert = self.revert.clone();
        let click: ClickClosure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            // The row around this verb is a link to the space. Copying is
            // not opening it.
            event.stop_propagation();
            event.prevent_default();
            copy(&host, &revert);
        }));
        if let Some(button) = button_of(this) {
            let _ =
                button.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        self.click = Some(click);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.click = None;
        if let Some(handle) = self.revert.borrow_mut().take()
            && let Some(win) = window()
        {
            win.clear_timeout_with_handle(handle);
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "label" || old == new {
            return;
        }
        // Only while resting — relabelling mid-answer would wipe "copied".
        if self.revert.borrow().is_none() {
            say(this, &label_of(this));
        }
    }
}

fn button_of(this: &HtmlElement) -> Option<Element> {
    this.query_selector(&format!(".{VERB_CLASS}"))
        .ok()
        .flatten()
}

fn label_of(this: &HtmlElement) -> String {
    this.get_attribute("label")
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| DEFAULT_LABEL.to_owned())
}

fn say(this: &HtmlElement, word: &str) {
    if let Some(button) = button_of(this) {
        button.set_text_content(Some(word));
    }
}

/// Resolve `url` against the page, so what is copied works when pasted.
fn absolute(this: &HtmlElement) -> Option<String> {
    let raw = this.get_attribute("url").filter(|url| !url.is_empty())?;
    let base = window()?.location().href().ok()?;
    let resolved = web_sys::Url::new_with_base(&raw, &base)
        .ok()
        .map(|url| url.href())
        .unwrap_or(raw);
    Some(tonk_analytics::launch::space_route_referral_url(&resolved).unwrap_or(resolved))
}

fn copy(this: &HtmlElement, revert: &Rc<RefCell<Option<i32>>>) {
    let Some(url) = absolute(this) else { return };
    let Some(clipboard) = window().map(|w| w.navigator().clipboard()) else {
        return;
    };

    let host = this.clone();
    let pending = revert.clone();
    spawn_local(async move {
        let ok = JsFuture::from(clipboard.write_text(&url)).await.is_ok();
        // Only claim success when the write actually resolved. A refusal —
        // an insecure context, a denied permission — has to say so, or the
        // user walks away believing they hold a link they do not.
        say(&host, if ok { "copied" } else { "couldn't copy" });
        schedule_revert(&host, &pending);
    });
}

/// Put the resting word back after the answer has been read.
fn schedule_revert(this: &HtmlElement, revert: &Rc<RefCell<Option<i32>>>) {
    let Some(win) = window() else { return };
    if let Some(previous) = revert.borrow_mut().take() {
        win.clear_timeout_with_handle(previous);
    }
    let host = this.clone();
    let pending = revert.clone();
    let back = Closure::once_into_js(move || {
        *pending.borrow_mut() = None;
        say(&host, &label_of(&host));
    });
    if let Ok(handle) =
        win.set_timeout_with_callback_and_timeout_and_arguments_0(back.unchecked_ref(), ANSWER_MS)
    {
        *revert.borrow_mut() = Some(handle);
    }
}

/// Register `<ui-copy-link>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-copy-link").is_undefined() {
        UiCopyLink::define("ui-copy-link");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn copied_space_link_carries_organic_and_hashed_space_attribution() {
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("ui-copy-link")
            .unwrap()
            .unchecked_into();
        let raw_space = "did:key:z6MkCopiedSpace";
        host.set_attribute("url", &format!("https://tonk.network/space/{raw_space}"))
            .unwrap();

        let copied = absolute(&host).expect("space link resolves");
        let url = web_sys::Url::new(&copied).unwrap();
        assert_eq!(
            url.search_params()
                .get(tonk_analytics::launch::CHANNEL_PARAMETER)
                .as_deref(),
            Some("reshare")
        );
        assert_eq!(
            url.search_params()
                .get(tonk_analytics::launch::SPACE_PARAMETER)
                .as_deref(),
            Some(tonk_analytics::anonymize(raw_space).as_str())
        );
    }
}
