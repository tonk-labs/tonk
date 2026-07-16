//! `<ui-space-name>` — a space's repository name, read live from its own branch.
//!
//! Host chrome, NOT space content: it renders the same chip regardless of what
//! the space asserts, so a space choosing wild UI can never redefine or break
//! it — unlike a stdlib `tonk:view/*` view, which lives on the space branch and
//! would need per-space seeding. The `ui-` prefix marks it a host UI primitive,
//! distinct from the `tonk-` data elements.
//!
//! Reads `xyz.tonk.repo/name` through an inline predicate (no concept named,
//! nothing seeded) on its own `with="main@{did}"`, exactly as
//! `<ui-sync-status>` reads sync state.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::JSON;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

use crate::logic::repo_name_query_body;
use crate::retry::RetryPolicy;

/// Shown before the first frame and for a repo with no name — matches the
/// existing "Untitled" fallback the seeded view rendered.
const UNTITLED: &str = "Untitled";

const SUB_TAG: &str = "ui-space-name";

#[derive(Default)]
pub struct UiSpaceNameElement {
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
}

impl CustomElement for UiSpaceNameElement {
    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_text_content(Some(UNTITLED));
    }

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Some(space) = this.get_attribute("space").filter(|s| !s.is_empty()) else {
            // No space yet (an unsubstituted `{id}` placeholder, say) — the
            // attribute callback re-runs this when it lands.
            return;
        };
        // Stamp our own routing context: `resolve_with` reads THIS element's
        // attribute and never walks ancestors.
        let _ = this.set_attribute("with", &crate::logic::space_with(&space));

        let subscription = self.subscription.clone();
        let retry = self.retry.clone();
        let host = this.clone();
        spawn_local(async move {
            if !host.is_connected() || subscription.borrow().is_some() {
                return;
            }
            subscribe_name(&host, &space, subscription, retry);
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.subscription.borrow_mut().take();
    }
}

fn subscribe_name(
    host: &HtmlElement,
    space: &str,
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
) {
    let body = match repo_name_query_body(space) {
        Ok(body) => body,
        Err(err) => {
            tonk_common::log!("ui-space-name: query build failed: {err}");
            return;
        }
    };
    let Ok(parsed) = JSON::parse(&body) else {
        tonk_common::log!("ui-space-name: query JSON parse failed");
        return;
    };
    let consumer_el: Element = host.clone().into();
    let tag = JsValue::from_str(SUB_TAG);
    match consumer::subscribe(&consumer_el, &parsed, Some(&tag)) {
        Ok(sub) => {
            retry.borrow_mut().reset();
            *subscription.borrow_mut() = Some(sub);
        }
        Err(err) => {
            // Bounded, unlike the host's default resubscribe loop.
            let delay = retry.borrow_mut().next_delay_ms();
            match delay {
                Some(_) => {
                    tonk_common::log!("ui-space-name: subscribe failed, will retry: {err:?}")
                }
                None => {
                    tonk_common::log!("ui-space-name: subscribe failed, giving up: {err:?}");
                    let _ = host.set_attribute("data-state", "unavailable");
                }
            }
        }
    }
}

/// Register `<ui-space-name>`. Idempotent.
pub fn register() {
    let registered = window()
        .map(|win| !win.custom_elements().get("ui-space-name").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    UiSpaceNameElement::define("ui-space-name");
}
