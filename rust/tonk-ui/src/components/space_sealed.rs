//! `/space/:space` — the declarative workspace-shell space route.
//!
//! The composition lives in YAML, not here. This route registers the tab's
//! SITE with the service worker and then mounts a single fixed
//! `<tonk-display model=tonk:site entity=site:…>` over the data-plane host.
//! The `tonk:site` view nests `{concept}` (the route model the SW stamped on
//! the site — now `tonk:workspace/shell`) on the SAME site entity, so the
//! declarative shell view renders and owns the page:
//!
//! ```text
//! <tonk-host>                          ← IO owner: queries → SW → DB
//!   <tonk-display model=tonk:site      ← site → {concept} → workspace/shell
//!     entity=site:…>                      → <tonk-repository><tonk-branch>
//!                                            <tonk-portal runtime …>
//! ```
//!
//! The shell's CONTENT portal is a sealed opaque-origin iframe whose guest
//! renders the space content (`model=tonk:space/route` → `tonk/space`),
//! relaying its query/subscribe events over `window.tonk` to this outer host.
//! The FAB portal is a later task. Moving the repository/branch/portal markup
//! into the shell view (`core.yaml`) is the point: the Leptos route carries no
//! composition, only the site registration + the fixed site display.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

use tonk_schema::parse_space;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct SealedSpaceParams {
    space: Option<String>,
}

/// The sealed-iframe space route. `:space` is `{branch}@{name}` (branch
/// defaults to `main`).
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpaceSealed() -> impl IntoView {
    let params = use_params::<SealedSpaceParams>();
    let space_ref = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
            .and_then(|s| parse_space(&s))
    });

    // Run background sync for the space while it's shown — without this the
    // sealed `/space` route never swept (the controller was only mounted by
    // the non-sealed `display.rs`), so nothing pushed/pulled and the sync
    // chip never updated. Same `Option<name>` source the display route uses.
    let sync_source =
        Signal::derive_local(move || space_ref.get().map(|s| s.name).filter(|s| !s.is_empty()));
    crate::sync_controller::mount(sync_source);

    // The route's own path — `/space/{space}` from the param, NOT
    // `window.location`. On a client-side navigation the resource below fires
    // before the router has committed the new URL, so reading `window.location`
    // would carry the previous path and the SW would resolve the wrong route.
    let route_path = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
            .map(|space| format!("/space/{space}"))
    });

    // Register this page's SITE with the service worker before mounting the
    // sealed view. The navigation that loaded this page predates the SW, so the
    // SW never saw it — the page must announce itself via `POST /api/site`. That
    // asserts the tab's `tonk:site` (so the display has something to resolve) and
    // returns the `site:<client-id>` entity the SW keys it on, which the host
    // caches for the guest context. We gate the portal mount on it so the guest's
    // `<tonk-display model=tonk:site>` never queries an unstamped site. The
    // resource tracks `route_path`, so a client-side navigation re-registers the
    // site for the new path.
    let site = LocalResource::new(move || {
        let path = route_path.get();
        async move {
            // `tonk_host::bridge` is wasm-only (it talks to the service
            // worker); on native the route has no SW to register with, so
            // the resource resolves to `None` and the portal stays ungated.
            #[cfg(target_arch = "wasm32")]
            {
                match path {
                    Some(path) => tonk_host::bridge::ensure_site(&path).await.ok(),
                    None => None,
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = path;
                None::<String>
            }
        }
    });

    // Mount the fixed site display, gated on the site registration so it never
    // queries an unstamped site. `<tonk-display model=tonk:site entity=site:…>`
    // resolves the `tonk:site` instance the SW stamped on this tab's
    // `site:<uuid>` entity; its view nests the matched route's `{concept}` —
    // `tonk:workspace/shell` — on the SAME entity, so the declarative shell
    // renders and owns the page (repository/branch/content-portal in YAML).
    //
    // The launcher wraps the whole router in the one real `<tonk-host>` (the IO
    // owner), so this display dispatches its query/subscribe events up to it.
    // The site entity is the value `ensure_site` returned (outer-side there is
    // no guest host to fill `data-tonk-entity="site"`, so `entity` is set
    // explicitly). The `.display-view-slot` chain is the bare-route fill-height
    // layout contract: the slot fills the viewport and the shell's portal fills
    // the slot via its own inline flex sizing.
    view! {
        <main class="display-route">
            <div class="display-view-slot">
                {move || site.get().flatten().map(|site_id| view! {
                    <tonk-display model="tonk:site" entity=site_id></tonk-display>
                })}
            </div>
        </main>
    }
}
