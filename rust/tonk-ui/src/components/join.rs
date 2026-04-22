//! `/join` route: claim an invite URL shared by another user.
//!
//! Reads the full current URL (including `#fragment`) and POSTs it to
//! `/api/claim`. Open invites carry the ephemeral seed in the URL
//! fragment; browsers never send fragments on normal fetches, so the
//! route component is responsible for forwarding the complete string.
//!
//! On success the user is navigated to `/space/{name}`, where the
//! space view picks up and the new row appears in the toolbar once
//! the shared repo-list resource refetches.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

use crate::api;
use crate::components::{RepoListResource, Status};

/// Status of the in-progress claim.
#[derive(Clone, Debug)]
enum ClaimStatus {
    /// Waiting for the claim request to complete.
    Pending,
    /// Claim succeeded; redirect is in flight.
    Ok,
    /// Claim failed with a human-readable message.
    Err(String),
}

fn current_url() -> Option<String> {
    window().location().href().ok()
}

/// `/join` route component. Runs the claim once init has completed;
/// on success, navigates to `/space/<local_name>`.
#[component]
pub fn TonkJoin() -> impl IntoView {
    let repos = use_context::<RepoListResource>().map(|ctx| ctx.0);
    let app_status =
        use_context::<Signal<Status, LocalStorage>>().expect("Status context required");

    let status = RwSignal::new(ClaimStatus::Pending);

    // Wait for TonkShell's init to finish before POSTing — the claim
    // handler writes into home, which doesn't exist until init has
    // PUT it. Tracks `app_status`: on each transition the Effect
    // re-runs, and we gate the claim behind `Ready` + a once-only
    // latch so subsequent status events don't retry the claim.
    Effect::new(move |kicked: Option<bool>| {
        if kicked.unwrap_or(false) {
            return true;
        }
        if !matches!(app_status.get(), Status::Ready) {
            return false;
        }
        let Some(url) = current_url() else {
            status.set(ClaimStatus::Err("could not read current URL".into()));
            return true;
        };

        let navigate = use_navigate();
        spawn_local(async move {
            match api::claim(&url).await {
                Ok(info) => {
                    status.set(ClaimStatus::Ok);
                    if let Some(r) = repos {
                        r.refetch();
                    }
                    navigate(&format!("/space/{}", info.name), Default::default());
                }
                Err(e) => status.set(ClaimStatus::Err(format!("{e}"))),
            }
        });

        true
    });

    view! {
        <section class="auth pending">
            <section
                class="panel"
                class:pending=move || matches!(status.get(), ClaimStatus::Pending)
            >
                <h2>"Joining a repository"</h2>
                {move || match status.get() {
                    ClaimStatus::Pending => view! {
                        <p class="status">"Claiming your invite…"</p>
                    }.into_any(),
                    ClaimStatus::Ok => view! {
                        <p class="status ok">"Invite claimed. Redirecting…"</p>
                    }.into_any(),
                    ClaimStatus::Err(msg) => view! {
                        <p class="status error">"Could not claim this invite:"</p>
                        <code class="error">{msg}</code>
                    }.into_any(),
                }}
            </section>
        </section>
    }
}
