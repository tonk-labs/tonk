//! First-run modal offering the two entry points into a fresh profile:
//! create a new repo, or redeem an invite.
//!
//! The "create" CTA is wired up once the `/api/repository/create` endpoint
//! lands; for now it is a no-op placeholder. The "redeem" CTA navigates to
//! the existing [`crate::components::TonkJoin`] route.

use leptos::prelude::*;
use leptos_router::components::A;

/// First-run modal. Rendered by the index route when the profile has no
/// repos yet; dismissed implicitly once the user picks an entry point.
#[component]
pub fn TonkFirstRunModal() -> impl IntoView {
    view! {
        <dialog class="first-run" open=true>
            <h2>"Get started"</h2>
            <p>"Create a new repo, or redeem an invite someone shared with you."</p>
            <div class="ctas">
                <button class="create" disabled=true>"Create repo"</button>
                <A href="/join" attr:class="redeem">"Redeem invite"</A>
            </div>
        </dialog>
    }
}
