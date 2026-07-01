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

    // The sub-path within the space — the part AFTER `/space/{space}`, which the
    // space's own `route!` table matches. `<tonk-site>` posts this to
    // `/api/repository/{space}/branch/{branch}/site` (the branch rides the URL),
    // so it carries only the rest, not the `/space/{space}` prefix. A bare space
    // root is `/`. From the params, not `window.location`, so a client-side
    // navigation resolves the new sub-path rather than the stale one.
    let rest = Signal::derive_local(move || {
        match params
            .get()
            .ok()
            .and_then(|p| p.subject)
            .filter(|s| !s.is_empty())
        {
            Some(subject) => format!("/{subject}"),
            None => "/".to_owned(),
        }
    });

    // Mount `<tonk-site>` scoped to the space repository: it registers the tab's
    // site on the space branch (matching `rest` against the space `route!`
    // table) and renders the matched route's view — `tonk:workspace/shell` for a
    // bare space root. This replaces the bespoke `ensure_site` +
    // `<tonk-display model=tonk:site>` with the data-driven router element, the
    // same one `/` uses, just pointed at a named repository instead of the
    // profile. The `.display-route` / `.display-view-slot` chain is the
    // fill-height layout contract.
    view! {
        <main class="display-route">
            <tonk-repository class="display-route" name=move || space_name.get().unwrap_or_default()>
                <tonk-branch name=move || branch_name.get().unwrap_or_default()>
                    <div class="display-view-slot">
                        <tonk-site path=move || rest.get()></tonk-site>
                    </div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
