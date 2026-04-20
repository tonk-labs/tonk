//! Index route view.
//!
//! When the repo list is empty, renders the first-run modal. When the
//! list is non-empty, renders a "pick a repo" prompt instead — the
//! modal is only a first-run artifact and shouldn't interrupt a user
//! with existing repos.

use leptos::prelude::*;

use crate::components::{RepoListResource, TonkFirstRunModal};

/// Index route. Gates the first-run modal on the sidebar's repo list.
#[component]
pub fn TonkEmpty() -> impl IntoView {
    let repos = use_context::<RepoListResource>()
        .expect("RepoListResource provided by TonkSidebar")
        .0;

    view! {
        <section class="empty">
            {move || match repos.get() {
                // Still loading — render nothing to avoid a flash of modal.
                None => ().into_any(),
                Some(Ok(entries)) if entries.is_empty() => view! {
                    <TonkFirstRunModal />
                }.into_any(),
                Some(Ok(_)) => view! {
                    <p class="pick">"Pick a repo from the sidebar."</p>
                }.into_any(),
                Some(Err(msg)) => view! { <p class="error">{msg}</p> }.into_any(),
            }}
        </section>
    }
}
