//! `/space/:space` — the sealed-iframe space view (spike).
//!
//! Renders the space's default view (`<tonk-display model=tonk/space>`)
//! **inside a sealed opaque-origin iframe** via `<tonk-portal runtime>`. The
//! outer document provides only the routing context and the data plane:
//!
//! ```text
//! <tonk-host>                       ← IO owner: queries → SW → DB
//!   <tonk-repository name={space}>  ← the space; annotates space on queries
//!     <tonk-branch name=main>       ← annotates branch
//!       <tonk-portal runtime        ← sealed iframe + window.tonk bridge +
//!         content="<tonk-display       injected element runtime
//!           model=tonk/space>">
//! ```
//!
//! Inside the guest, a real `<tonk-display>` dispatches its query/subscribe
//! events to the proxy `<tonk-host>` (registered by the guest runtime),
//! which relays them over `window.tonk` to the outer portal, which bubbles
//! them to the real `<tonk-host>` → SW. The space/branch annotation happens
//! outer-side from the `<tonk-repository>`/`<tonk-branch>` ancestors, so the
//! guest only needs the `model`.

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
    let space_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.name).unwrap_or_default());
    let branch_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.branch).unwrap_or_default());

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

    // The guest content is just the route's mount slot
    // (`.display-view-slot > tonk-display`) under the proxy `<tonk-host>`.
    // The app stylesheet anchors the bare-display-route fill-height layout
    // on `.display-view-slot` (not the `.display-route > tonk-branch`
    // routing path), so the view fills the iframe with no routing ancestors
    // inside the guest — the iframe body is the route's viewport box and
    // `100dvh` resolves against it.
    //
    // The proxy `<tonk-host>` (registered by the injected runtime) is the IO
    // owner: `<tonk-display>` dispatches query/subscribe events to it and it
    // relays them over `window.tonk` to the outer bridge. It collapses to
    // `display: contents` (the app CSS `tonk-host:has(> .display-view-slot)`
    // rule).
    //
    // The space renders the tab's SITE. The fixed `model=tonk:site` resolves the
    // `tonk:site` instance the SW stamped on this tab's `site:<uuid>` entity; its
    // view nests into the matched route's `{concept}` on the same entity, which
    // resolves and renders. `data-tonk-entity="site"` is the seam — the guest
    // proxy `<tonk-host>` fills `entity` with this tab's site id (minted inside
    // the host, unknown when this markup is built).
    const CONTENT: &str = "<tonk-host>\
<div class=\"display-view-slot\">\
<tonk-display model=tonk:site data-tonk-entity=\"site\"></tonk-display>\
</div>\
</tonk-host>";

    // No inner `<tonk-host>`: the launcher wraps the whole router in one
    // (the IO owner). The portal's relayed query/subscribe events bubble up
    // through these repository/branch ancestors to that host.
    //
    // The `.display-route` + `.display-view-slot` chain is the bare-route
    // layout contract: `display: contents` collapses the routing-context
    // elements and the slot fills the viewport (`min-height: 100dvh`). The
    // sealed `<tonk-portal>` fills that slot via inline flex sizing (it
    // carries no app CSS itself).
    view! {
        <main class="display-route">
            <tonk-repository
                class="display-route"
                name=move || space_name.get()
            >
                <tonk-branch name=move || branch_name.get()>
                    <div class="display-view-slot">
                        // Gate the sealed portal on the site registration so the
                        // guest never queries an unstamped site. `ensure_site`
                        // also populates `site_id()`, which the portal threads
                        // into the guest context.
                        {move || site.get().flatten().map(|_site| view! {
                            <tonk-portal
                                runtime
                                content=CONTENT
                                style="display:flex; flex-direction:column; flex:1 1 auto; min-height:100dvh;"
                            ></tonk-portal>
                        })}
                    </div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
