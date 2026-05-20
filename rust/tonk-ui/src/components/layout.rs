//! `/space/:space/branch/:branch/layout/:workspace` route.
//!
//! Mounts a `<tonk-layout>` element — the tiling window manager —
//! for the named workspace. Unlike the `<tonk-display>` route,
//! there is no name resolution: `:workspace` is a plain string
//! forwarded straight onto the element's `workspace` attribute,
//! and the element resolves the matching workspace entity on the
//! branch itself.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkLayoutParams {
    space: Option<String>,
    branch: Option<String>,
    workspace: Option<String>,
}

/// Window-manager route. Mounts a `<tonk-layout>` for the path's
/// `:space` / `:branch` / `:workspace`.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkLayoutView() -> impl IntoView {
    let params = use_params::<TonkLayoutParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
    });
    let branch_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.branch)
            .filter(|s| !s.is_empty())
    });
    let workspace_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.workspace)
            .filter(|s| !s.is_empty())
    });

    // A mount node plus one Effect: when any path signal changes
    // the Effect rebuilds the `<tonk-layout>` host with current
    // attributes. We mount imperatively (rather than writing the
    // tag in `view!`) so the attributes land verbatim on the
    // custom element. Same shape as the `<tonk-display>` route.
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
        if let Some(space) = space_name.get() {
            let _ = host.set_attribute("space", &space);
        }
        if let Some(branch) = branch_name.get() {
            let _ = host.set_attribute("branch", &branch);
        }
        if let Some(workspace) = workspace_name.get() {
            let _ = host.set_attribute("workspace", &workspace);
        }
        let _ = slot.append_child(&host);
    });

    // No header — the window manager owns the whole viewport, so
    // there is no workspace-title banner. The current workspace
    // is already in the URL.
    view! {
        <main class="layout-view">
            <div class="layout-view-slot" node_ref=mount></div>
        </main>
    }
}
