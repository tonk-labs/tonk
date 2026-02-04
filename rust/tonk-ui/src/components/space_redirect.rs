//! Space redirect component - redirects from / to the first space.

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::api;

/// Redirects from the root path to the first available space.
///
/// When the user navigates to `/`, this component fetches the list of spaces
/// and redirects to the first one (sorted alphabetically by DID).
#[component]
pub fn SpaceRedirect() -> impl IntoView {
    let navigate = use_navigate();

    // Fetch space list and redirect to first
    let spaces = LocalResource::new(|| async { api::list_spaces().await });

    Effect::new(move |_| {
        if let Some(Ok(list)) = spaces.get() {
            if let Some(first) = list.spaces.first() {
                // Extract multikey from did:key:z6Mk...
                let multikey = first
                    .did
                    .strip_prefix("did:key:")
                    .unwrap_or(&first.did)
                    .to_string();
                navigate(&format!("/{}/", multikey), Default::default());
            }
        }
    });

    view! {
        <div class="space-redirect">
            <p>"Loading spaces..."</p>
        </div>
    }
}
