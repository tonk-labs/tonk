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

use crate::components::route::parse_space;

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
    let space_name = Signal::derive_local(move || space_ref.get().map(|s| s.name).unwrap_or_default());
    let branch_name =
        Signal::derive_local(move || space_ref.get().map(|s| s.branch).unwrap_or_default());

    // The guest content: a real `<tonk-display>` UNDER the guest-side proxy
    // `<tonk-host>` (registered by the injected runtime). `<tonk-display>`
    // dispatches its query/subscribe events to that ancestor, which relays
    // them over `window.tonk` to the outer bridge. Only `model` is needed —
    // space/branch are annotated outer-side from the routing context.
    //
    // A flex column so the display fills the iframe (the guest base CSS
    // gives `<body>` full height + column flex).
    const CONTENT: &str = "<tonk-host style=\"display:flex;flex-direction:column;flex:1 1 auto\">\
<tonk-display model=tonk/space style=\"display:flex;flex-direction:column;flex:1 1 auto\"></tonk-display>\
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
                        <tonk-portal
                            runtime
                            content=CONTENT
                            style="display:flex; flex-direction:column; flex:1 1 auto; min-height:100dvh;"
                        ></tonk-portal>
                    </div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
