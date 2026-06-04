//! `/space/:space/layout/:workspace` route (`:space` is `{branch}@{name}`).
//!
//! Mounts a `<tonk-layout>` for the given workspace name. Unlike
//! the display route, no async pre-resolve is needed — the
//! workspace name is matched against the workspace concept's `name`
//! field by the element's own subscription, so this route is a thin
//! attribute-passing shim.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkLayoutParams {
    space: Option<String>,
    workspace: Option<String>,
}

/// Layout workspace route. Reads `:space` (`{branch}@{name}`) and
/// `:workspace` from the URL and mounts a `<tonk-layout>` with them as
/// attributes. The element handles all subscriptions, folding, and
/// reconciliation internally.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkLayoutView() -> impl IntoView {
    let params = use_params::<TonkLayoutParams>();

    let space_ref = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
            .and_then(|s| crate::components::route::parse_space(&s))
    });
    let space_name = Signal::derive_local(move || space_ref.get().map(|s| s.name));
    let branch_name = Signal::derive_local(move || space_ref.get().map(|s| s.branch));
    let workspace_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.workspace)
            .filter(|s| !s.is_empty())
    });

    // The slot + a single Effect: whenever the workspace signal
    // changes, rebuild the `<tonk-layout>` host so it picks up the
    // new attribute value. Space and branch come from the
    // surrounding routing elements; no attributes needed here.
    let mount: NodeRef<leptos::html::Div> = NodeRef::new();
    Effect::new(move |_| {
        let Some(slot) = mount.get() else {
            return;
        };
        let document = leptos::prelude::document();
        slot.set_inner_html("");
        let Ok(host) = document.create_element("tonk-layout") else {
            return;
        };
        if let Some(w) = workspace_name.get() {
            let _ = host.set_attribute("workspace", &w);
        }
        let _ = slot.append_child(&host);
    });

    view! {
        <header slot="main-header" class="space-banner">
            <h1 class="space-banner-title" title=move || workspace_name.get().unwrap_or_default()>
                { move || workspace_name.get().unwrap_or_default() }
            </h1>
        </header>
        <main class="wa-stack space-view layout-view">
            <tonk-repository name=move || space_name.get().unwrap_or_default()>
                <tonk-branch name=move || branch_name.get().unwrap_or_default()>
                    <div class="layout-view-slot" node_ref=mount></div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
