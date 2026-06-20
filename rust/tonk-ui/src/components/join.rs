//! `/join` route — redeem an invite, declaratively.
//!
//! The join is no longer orchestrated in Leptos. This component just
//! mounts the standard library's `tonk:join/status` view on the profile's
//! meta branch — the same routing-context pattern as the Hub. Inside that
//! view, a `<tonk-page onmount=tonk/join>` reads the page location and
//! fires the `tonk:join` command; the worker provider parses + claims the
//! invite and drives the overlay-only `tonk:join/status` (pending →
//! failed, or retract + durable replica on success). The view renders the
//! status and links into the joined space.
//!
//! Why the page-side `<tonk-page>` is still needed: the service worker
//! can't see the URL `#fragment` (browsers strip it), and an audience-open
//! invite carries the ephemeral seed there. `<tonk-page>` reads it
//! client-side and delivers it to the command.

use leptos::prelude::*;

use crate::api;

/// The `/join` view. Pure routing context over the profile's meta branch:
/// a no-entity `<tonk-display model="tonk:join/status">` resolves the
/// `tonk:join/status` *directory view* in the standard library, whose
/// chrome holds the `<tonk-page onmount=tonk/join>` trigger (always mounts
/// → fires the command on load) and the pending/failed status rendering.
/// Directory chrome renders even with zero instances, so the trigger
/// fires before any status exists — and it's what creates the status.
#[component]
pub fn TonkJoin() -> impl IntoView {
    view! {
        <main class="join-route">
            <tonk-repository class="display-route" name=api::DEFAULT_REPO profile>
                <tonk-branch name="meta">
                    <div class="display-view-slot">
                        <tonk-display model="tonk:join/status"></tonk-display>
                    </div>
                </tonk-branch>
            </tonk-repository>
        </main>
    }
}
