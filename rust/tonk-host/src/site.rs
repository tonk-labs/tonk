//! `<tonk-site path="…">` — a routing element.
//!
//! Registers a per-tab site for `path` on a branch and renders the matched
//! route. The branch comes from the element's attributes (this first cut only
//! supports the profile via the `profile` flag; ancestor `<tonk-repository>` /
//! `<tonk-branch>` resolution and the sealed-iframe space variant come later).
//!
//! On connect it posts `path` to the per-branch `/site` endpoint
//! (`/api/profile/branch/{branch}/site`), which asserts the tab's `tonk:site`
//! on that branch's overlay (matching `path` against the branch's `route!`
//! table) and returns the `site:<client-id>` entity. The element then mounts
//! `<tonk-display entity={site} model=tonk:site>`, whose view nests the matched
//! `{concept}` — exactly the indirection the sealed `/space` route uses, but
//! driven by this element's `path` rather than the SW parsing the document URL.

use custom_elements::CustomElement;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlElement, window};

use crate::bridge;

/// The `<tonk-site>` element. Stateless beyond the DOM it renders.
#[derive(Default)]
pub(crate) struct TonkSite;

impl CustomElement for TonkSite {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["path", "branch", "profile"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        resolve_and_render(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // Re-resolve when `path` changes (a client-side navigation). The initial
        // value is handled by `connected_callback`, so skip the upgrade-time
        // callback (old == None on first set) to avoid a double render.
        if name == "path" && old.is_some() && old != new {
            resolve_and_render(this);
        }
    }
}

/// Read the element's `path` (defaulting to the document pathname), register the
/// site on its branch, and mount the matched route's display. Best-effort: a
/// failed registration leaves the element empty rather than throwing.
fn resolve_and_render(this: &HtmlElement) {
    let host = this.clone();
    let path = host
        .get_attribute("path")
        .filter(|p| !p.is_empty())
        .or_else(|| window().and_then(|w| w.location().pathname().ok()))
        .unwrap_or_else(|| "/".to_owned());
    let branch = host
        .get_attribute("branch")
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "meta".to_owned());
    // This first cut routes the profile only; the `profile` flag selects the
    // profile-as-repository endpoint. A named-repo `<tonk-site>` (ancestor
    // `<tonk-repository name=…>`) is a later step.
    let url = format!("/api/profile/branch/{branch}/site");

    spawn_local(async move {
        match bridge::ensure_site_on(&url, &path).await {
            Ok(site) => mount_display(&host, &site),
            Err(error) => {
                tonk_common::log!("tonk-site: register failed for {path}: {error:?}");
            }
        }
    });
}

/// Replace the element's children with `<tonk-display entity={site}
/// model=tonk:site>`, the same site indirection the sealed `/space` route uses.
fn mount_display(host: &HtmlElement, site: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(display) = document.create_element("tonk-display") else {
        return;
    };
    let _ = display.set_attribute("entity", site);
    let _ = display.set_attribute("model", "tonk:site");
    host.set_inner_html("");
    let node: web_sys::Node = display.into();
    let _ = host.append_child(&node);
}

/// Register `<tonk-site>`. Idempotent.
pub fn register() {
    if let Some(win) = window() {
        if win.custom_elements().get("tonk-site").is_undefined() {
            TonkSite::define("tonk-site");
        }
    }
}
