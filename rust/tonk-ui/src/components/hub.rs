//! `/` route — the Tonk Hub.
//!
//! A repository picker: the set of spaces this profile can open,
//! rendered as cards that link to `/space/{name}`. The list lives on
//! the profile repository's meta branch (one `space`/replica record per
//! repository), so the Hub is a `<tonk-display model=space>` directory
//! mounted under a `<tonk-repository profile>` — the `profile` flag
//! routes its queries to the profile-as-repository endpoint
//! (`/api/profile/branch/meta/query`) rather than a named repo.
//!
//! Like the bare display route, the Hub renders without shell chrome:
//! it is the page. The repository/branch elements are pure routing
//! context (transparent in layout); the cards and their styling come
//! from the `space` directory view in the standard library.

use leptos::prelude::*;

use crate::api;

/// The Tonk Hub view at `/`. Mounts the `space` directory over the
/// profile's meta branch.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkHub() -> impl IntoView {
    view! {
        <tonk-repository class="display-route hub-route" name=api::DEFAULT_REPO profile>
            <tonk-branch name="meta">
                <div class="display-view-slot">
                    <tonk-display model="space"></tonk-display>
                </div>
            </tonk-branch>
        </tonk-repository>
    }
}
