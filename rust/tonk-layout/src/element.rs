//! `<tonk-layout>` custom-element implementation.
//!
//! Step-1 skeleton: observes `workspace` / `space` / `branch`,
//! reflects `data-state="loading"` on connect, mounts an empty
//! `<div class="niri-strip">` placeholder, and restarts on
//! attribute change. Subscriptions, reconciliation, and
//! interaction land in later steps.

use custom_elements::CustomElement;
use web_sys::{Element, HtmlElement, window};

use crate::state::{self, State};

/// The custom element.
#[derive(Default)]
pub struct TonkLayout;

impl CustomElement for TonkLayout {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["workspace", "space", "branch"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        mount_skeleton(&host);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        // Same teardown/restart discipline as `<tonk-display>`:
        // wipe the host and re-mount from scratch on any observed
        // attribute change. Once subscriptions land, this will also
        // abort them.
        host.set_inner_html("");
        mount_skeleton(&host);
    }
}

/// Set `data-state="loading"` and mount the empty strip container.
/// The strip stays empty until the read path (step 5) opens
/// subscriptions and feeds frames through the reconciler.
fn mount_skeleton(host: &Element) {
    state::set(host, State::Loading);
    if let Some(document) = window().and_then(|w| w.document())
        && let Ok(strip) = document.create_element("div")
    {
        let _ = strip.set_attribute("class", "niri-strip");
        let _ = host.append_child(&strip);
    }
}

/// Public entry point — registers the element with the page.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkLayout::define("tonk-layout");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-layout").is_undefined()
}
