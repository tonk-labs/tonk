//! `<ui-site-request>` — the bar performs what its tab was asked to do.
//!
//! The bar's own acts (add an account, copy a share link, view members,
//! connect an agent) are commands like any other, so anything that can
//! transact can ask for them: the command palette, an agent, another view.
//! Their handlers cannot open a panel or start a ceremony, so each records
//! the request on the asking tab's site (`xyz.tonk.site/request` and
//! `xyz.tonk.site/request-time`, in the profile's session overlay; see the
//! worker's `site_request`). This headless child subscribes to its own
//! tab's site and, on a request newer than any it has seen, presses the
//! control that performs it — so the act runs exactly as a click would,
//! through the one implementation the bar already has.
//!
//! Headless like `<ui-sync-status>`: unslotted, so it renders nothing.

use std::cell::Cell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlElement;

use crate::subscribing;

const SUB_TAG: &str = "ui-site-request";

/// The profile branch's routing context when the bar names none.
const PROFILE_WITH: &str = "main@profile:tonk";

/// Which control performs each request.
const CONTROLS: [(&str, &str); 4] = [
    ("account", ".login"),
    ("share", "[data-panel=share]"),
    ("members", "[data-panel=members]"),
    ("agent", "[data-panel=agent]"),
];

#[derive(Default)]
pub struct UiSiteRequestElement {
    scaffold: Rc<subscribing::Scaffold>,
}

/// The subscription: this tab's site request, on the profile branch.
struct SiteRequestBehaviour {
    site: String,
    /// The newest request seen. `None` until the first frame, which only
    /// sets it: a request made before this bar connected is not replayed.
    seen: Rc<Cell<Option<f64>>>,
}

impl subscribing::Subscribing for SiteRequestBehaviour {
    fn resolve_with(&self, this: &HtmlElement) -> Option<String> {
        // The bar reads the profile through its own `with`; follow it, so a
        // profile on another branch is still the one asked.
        let with = this
            .closest("tonk-fab")
            .ok()
            .flatten()
            .and_then(|bar| bar.get_attribute("with"))
            .filter(|with| !with.is_empty() && !with.contains('{'));
        Some(with.unwrap_or_else(|| PROFILE_WITH.to_owned()))
    }

    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        Ok(serde_json::json!({
            "predicate": { "with": {
                "request": { "the": "xyz.tonk.site/request", "as": "Text", "cardinality": "one" },
                "time": { "the": "xyz.tonk.site/request-time", "as": "Float", "cardinality": "one" }
            } },
            "terms": {
                "this": self.site,
                "request": { "?": { "name": "request" } },
                "time": { "?": { "name": "time" } }
            }
        })
        .to_string())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let rows = js_sys::Array::from(payload);
        self.consider(host, &rows.get(rows.length().saturating_sub(1)));
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        let rows = js_sys::Array::from(&asserted);
        self.consider(host, &rows.get(rows.length().saturating_sub(1)));
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl SiteRequestBehaviour {
    /// Act on `row` if it is a request newer than any seen.
    fn consider(&self, host: &HtmlElement, row: &JsValue) {
        let request = read(row);
        let Some(seen) = self.seen.get() else {
            self.seen
                .set(Some(request.as_ref().map_or(0.0, |(_, time)| *time)));
            return;
        };
        let Some((request, time)) = request else {
            return;
        };
        if time <= seen {
            return;
        }
        self.seen.set(Some(time));
        perform(host, &request);
    }
}

/// `{fields: {request, time}}` off a subscription row.
fn read(row: &JsValue) -> Option<(String, f64)> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let request = Reflect::get(&fields, &"request".into()).ok()?.as_string()?;
    let time = Reflect::get(&fields, &"time".into()).ok()?.as_f64()?;
    Some((request, time))
}

/// Press the bar's control for `request`, unfolding the bar first, unless
/// the bar is not offering it right now (a hidden control: "add an account"
/// once there is one).
fn perform(host: &HtmlElement, request: &str) {
    let Some(bar) = host.closest("tonk-fab").ok().flatten() else {
        return;
    };
    let Some(control) = CONTROLS
        .iter()
        .find(|(name, _)| *name == request)
        .and_then(|(_, selector)| bar.shadow_root()?.query_selector(selector).ok().flatten())
    else {
        tonk_common::log!("{SUB_TAG}: no control for {request:?}");
        return;
    };
    if control.has_attribute("hidden") {
        tonk_common::log!("{SUB_TAG}: {request:?} is not offered right now");
        return;
    }
    if let Ok(expand) = Reflect::get(&bar, &"expand".into())
        && let Ok(expand) = expand.dyn_into::<js_sys::Function>()
    {
        let _ = expand.call0(&bar);
    }
    if let Ok(control) = control.dyn_into::<HtmlElement>() {
        control.click();
    }
}

impl CustomElement for UiSiteRequestElement {
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host = this.clone();
        let scaffold = self.scaffold.clone();
        spawn_local(async move {
            // The tab's site is `site:<client>`, the entity a command's
            // handler derives from the tab that asked. A page that has not
            // registered one yet (a bar in a sealed guest whose host never
            // did) registers now: the service worker answers with it.
            let mut site = tonk_host::bridge::site_id();
            if site.is_empty() {
                let path = tonk_host::bridge::context_field("path").unwrap_or_else(|| "/".into());
                match tonk_host::bridge::ensure_site(&path).await {
                    Ok(assigned) => site = assigned,
                    Err(error) => {
                        tonk_common::log!("{SUB_TAG}: no site for this tab: {}", error.message);
                    }
                }
            }
            if site.is_empty() || !host.is_connected() {
                return;
            }
            let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(SiteRequestBehaviour {
                site,
                seen: Rc::new(Cell::new(None)),
            });
            scaffold.connect(&host, behaviour);
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// Register `<ui-site-request>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiSiteRequestElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
