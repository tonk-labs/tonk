//! Repo list sidebar. Reads `/api/repositories` (not yet implemented) and
//! renders one row per repo the profile has access to. The empty-state
//! modal handles the zero-repos case — this component just shows the list.
//!
//! For now the list is always empty; wiring up [`api::repositories`] lands
//! alongside the create-repo and claim-invite flows.

use leptos::prelude::*;

/// Left-hand repo list. Always visible; the empty-state modal is rendered
/// separately by the index route.
#[component]
pub fn TonkSidebar() -> impl IntoView {
    view! {
        <aside class="sidebar">
            <ul class="repos">
                // TODO: populate from /api/repositories
            </ul>
        </aside>
    }
}
