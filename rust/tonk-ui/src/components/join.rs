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

/// The `/join` view. A thin shim that mounts the `<tonk-join>` custom
/// element (see [`crate::components::route_views`]); the element carries the
/// routing-context markup and the `tonk:join/status` directory view holds
/// the `<tonk-page onmount=tonk:join>` trigger that fires the join command.
/// This shim exists only while the router is still Leptos.
#[component]
pub fn TonkJoin() -> impl IntoView {
    view! { <tonk-join></tonk-join> }
}
