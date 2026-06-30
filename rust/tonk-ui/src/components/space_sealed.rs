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
//!   <tonk-repository name=… >          ← routing context for the site query
//!     <tonk-branch name=… >               (the space branch `/api/site` stamped)
//!       <tonk-display model=tonk:site  ← site → {concept} → workspace/shell
//!         entity=site:…>                  → <tonk-repository><tonk-branch>
//!                                            <tonk-portal runtime …>
//! ```
//!
//! The route still carries no *shell composition* — only the routing context
//! the outer `tonk:site` query needs to reach the space branch. Without the
//! `<tonk-repository>`/`<tonk-branch>` annotation the host falls back to the
//! bare `/query` path, which the SW only rewrites for sealed guest iframes; a
//! top-level page hitting it gets a 405. The shell view supplies its OWN
//! repository/branch/portal for the sealed content, and inner-most-wins leaves
//! that nesting intact.
//!
//! The shell's CONTENT portal is a sealed opaque-origin iframe whose guest
//! renders the space content (`model=tonk:space/route` → `tonk/space`),
//! relaying its query/subscribe events over `window.tonk` to this outer host.
//! The FAB portal is a later task.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

use tonk_schema::parse_space;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct SealedSpaceParams {
    space: Option<String>,
    /// The remaining path after `/space/{space}` (e.g. `inspector`,
    /// `id:x@trip`). Captured as a wildcard so an entity URI containing `/`
    /// is taken whole; threaded into the registered site path so the SW's
    /// route table matches the full Level-1 path. Absent for a bare space URL.
    subject: Option<String>,
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

    // The outer `tonk:site` display queries the space branch where `/api/site`
    // stamped this tab's `site:<client-id>` (see `register_site`). Without a
    // `<tonk-repository>` / `<tonk-branch>` ancestor the host has no context and
    // builds the bare `/query` fallback, which the SW only rewrites for guest
    // iframes — from a top-level page it falls through to the network as a 405.
    // So annotate the routing context here, exactly as the non-sealed display
    // route does (`display.rs`). The shell view nests its own
    // `<tonk-repository>` for the sealed content; inner-most-wins keeps that
    // intact.
    let space_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.name).filter(|s| !s.is_empty()));
    let branch_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.branch).filter(|s| !s.is_empty()));

    // The route's own path — `/space/{space}` plus any `*subject` tail, from the
    // params, NOT `window.location`. On a client-side navigation the resource
    // below fires before the router has committed the new URL, so reading
    // `window.location` would carry the previous path and the SW would resolve
    // the wrong route. The SW splits off the `/space/{space}` prefix (Level 0)
    // and matches the remaining `/{subject}` against the route table (Level 1),
    // so the full path must be registered, sub-route included.
    let route_path = Signal::derive_local(move || {
        let params = params.get().ok()?;
        let space = params.space.filter(|s| !s.is_empty())?;
        match params.subject.filter(|s| !s.is_empty()) {
            Some(subject) => Some(format!("/space/{space}/{subject}")),
            None => Some(format!("/space/{space}")),
        }
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
            <tonk-repository class="display-route" name=move || space_name.get().unwrap_or_default()>
                <tonk-branch name=move || branch_name.get().unwrap_or_default()>
                    <div class="display-view-slot">
                        {move || site.get().flatten().map(|site_id| view! {
                            <tonk-display model="tonk:site" entity=site_id></tonk-display>
                        })}
                    </div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
