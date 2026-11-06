//! `<ui-space-publication>` — whether a space is published, read live from
//! its own branch, for the bar to offer making it public or private.
//!
//! Headless, like `<ui-space-name headless>`: a subscription with an
//! attribute for an output. It reads the invitations the space records
//! with kind `public` (the worker's `space/publish` writes one, its
//! `space/unpublish` retracts it) and stamps `data-published` on the bar
//! while any is there. The bar's CSS shows "make space public" or "make
//! space private" from that attribute, and its click dispatches the
//! command ([`dispatch`]).

use std::cell::Cell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlElement;

use crate::logic::{publication_claim_json, publication_query_body};
use crate::subscribing;

const SUB_TAG: &str = "ui-space-publication";

/// The bar attribute present while the space is published.
const PUBLISHED_ATTR: &str = "data-published";

#[derive(Default)]
pub struct UiSpacePublicationElement {
    scaffold: subscribing::Scaffold,
    /// How many publications the subscription has delivered and not
    /// retracted since.
    count: Rc<Cell<u32>>,
}

struct PublicationBehaviour {
    count: Rc<Cell<u32>>,
}

impl subscribing::Subscribing for PublicationBehaviour {
    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        Ok(publication_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        self.count.set(js_sys::Array::from(payload).length());
        stamp(host, self.count.get());
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let rows = |key: &str| {
            Reflect::get(payload, &key.into())
                .map(|rows| js_sys::Array::from(&rows).length())
                .unwrap_or(0)
        };
        let count = (self.count.get() + rows("asserted")).saturating_sub(rows("retracted"));
        self.count.set(count);
        stamp(host, count);
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl CustomElement for UiSpacePublicationElement {
    /// Headless: it owns no DOM, only the bar attribute it stamps.
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.wire(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "space" || old == new {
            return;
        }
        self.scaffold.disconnect();
        self.wire(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

impl UiSpacePublicationElement {
    fn wire(&mut self, this: &HtmlElement) {
        self.count.set(0);
        stamp(this, 0);
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(PublicationBehaviour {
            count: self.count.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }
}

/// Stamp the bar this element feeds with whether the space is published.
fn stamp(host: &HtmlElement, count: u32) {
    let Some(bar) = host
        .closest("tonk-fab")
        .ok()
        .flatten()
        .and_then(|bar| bar.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    if count > 0 {
        let _ = bar.set_attribute(PUBLISHED_ATTR, "");
    } else {
        let _ = bar.remove_attribute(PUBLISHED_ATTR);
    }
}

/// Ask the worker to publish `space`, or to make it private again.
pub(crate) fn dispatch(space: &str, publish: bool) {
    crate::share::dispatch_claim(&publication_claim_json(space, publish, js_sys::Date::now()));
}

/// Register `<ui-space-publication>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiSpacePublicationElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
