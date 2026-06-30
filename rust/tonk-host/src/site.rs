//! `<tonk-site path="…">` — a routing element.
//!
//! Registers a per-tab site for `path` on the branch named by its **ancestor**
//! `<tonk-repository>` / `<tonk-branch>` context, and renders the matched route.
//! The repository, branch, and the `profile` flag come from those ancestors (the
//! same routing-context elements every other consumer reads), not from the
//! element's own attributes — so a `<tonk-site>` is always scoped by the
//! `<tonk-repository>` it lives inside.
//!
//! On connect it posts `path` to the per-branch `/site` endpoint
//! (`/api/{repository/{repo}|profile}/branch/{branch}/site`), which asserts the
//! tab's `tonk:site` on that branch's overlay (matching `path` against the
//! branch's `route!` table) and returns the `site:<client-id>` entity. The
//! element then mounts `<tonk-display entity={site} model=tonk:site>`, whose view
//! nests the matched `{concept}`. Because the display is a descendant of the same
//! `<tonk-repository>`/`<tonk-branch>`, its own queries inherit the routing
//! context — no synthesized wrapper needed.

use custom_elements::CustomElement;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use crate::bridge;
use crate::ops::read_context_from_ancestors;

/// The `<tonk-site>` element. Stateless beyond the DOM it renders.
#[derive(Default)]
pub(crate) struct TonkSite;

impl CustomElement for TonkSite {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["path"]
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
        // Re-resolve on a client-side navigation (`path` change). The initial
        // value is handled by `connected_callback`, so skip the upgrade-time
        // callback (old == None on first set) to avoid a double render.
        if name == "path" && old.is_some() && old != new {
            resolve_and_render(this);
        }
    }
}

/// Read the element's `path`, resolve its `(repository, branch, profile)` from
/// ancestor routing-context elements, register the site on that branch, and
/// mount the matched route's display. Best-effort: a failed registration leaves
/// the element empty rather than throwing.
fn resolve_and_render(this: &HtmlElement) {
    let host = this.clone();
    let path = host
        .get_attribute("path")
        .filter(|p| !p.is_empty())
        .or_else(|| window().and_then(|w| w.location().pathname().ok()))
        .unwrap_or_else(|| "/".to_owned());

    // The repository + branch + profile flag come from ancestor
    // `<tonk-repository>` / `<tonk-branch>` elements, the same context the
    // host's query routing reads. A `<tonk-repository profile>` (or no
    // repository at all) targets the profile-as-repository endpoint.
    let element: Element = host.clone().into();
    let (repo, branch, profile) = read_context_from_ancestors(&element);
    let profile = profile || repo.is_none();
    let default_branch = if profile { "meta" } else { "main" };
    let branch = branch.unwrap_or_else(|| default_branch.to_owned());
    let url = if profile {
        format!("/api/profile/branch/{branch}/site")
    } else {
        let repo = repo.unwrap_or_default();
        format!("/api/repository/{repo}/branch/{branch}/site")
    };

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
/// model=tonk:site>`. The display inherits its routing context from the
/// `<tonk-repository>`/`<tonk-branch>` ancestors `<tonk-site>` itself lives
/// under, so no wrapper is synthesized here.
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
