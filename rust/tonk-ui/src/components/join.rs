//! `/join` route: browser-side invite claim flow.
//!
//! Reads the full current URL (including `#fragment`) and POSTs it to the
//! service worker's `/api/invite/claim` endpoint, which does the actual
//! parse/redelegate/persist work via `tonk-invite`. On success the user
//! is navigated to `/repo/<local_repo>` where the sidebar and repo view
//! take over.

use super::service_worker_activates;
use crate::api;
use crate::components::RepoListResource;
use leptos::{logging::log, prelude::*};
use leptos_router::hooks::use_navigate;

/// Status of the in-progress claim.
#[derive(Clone, Debug)]
enum ClaimStatus {
    /// Waiting for the service worker and the claim request.
    Pending,
    /// Claim succeeded; redirect is in flight.
    Ok,
    /// Claim failed with a human-readable message.
    Err(String),
}

/// Reads the full current URL including the fragment.
///
/// The fragment is required for open invites (it carries the ephemeral
/// seed) and is kept browser-side by navigation, so the only way to
/// recover it is via `window.location.href`.
fn current_url() -> Option<String> {
    window().location().href().ok()
}

/// `/join` route component. Kicks off the claim flow on mount; on
/// success, navigates to `/repo/<local_repo>`.
#[component]
pub fn TonkJoin() -> impl IntoView {
    let claim = LocalResource::new(|| async {
        let url = current_url().ok_or_else(|| {
            crate::error::TonkUiError::ApiError("could not read current URL".into())
        })?;
        // Wait for the service worker to take control before POSTing the
        // claim — without this, the request can race past the SW and hit
        // the SPA `index.html` fallback, producing an HTML response that
        // fails to decode as JSON.
        service_worker_activates().await;
        log!("Join flow: claiming URL");
        api::claim_invite(&url).await
    });

    let status = Signal::derive_local(move || match claim.get() {
        None => ClaimStatus::Pending,
        Some(Err(e)) => ClaimStatus::Err(format!("{e}")),
        Some(Ok(resp)) if resp.success => ClaimStatus::Ok,
        Some(Ok(resp)) => ClaimStatus::Err(resp.error.unwrap_or_else(|| "claim failed".into())),
    });

    let navigate = use_navigate();
    let repos = use_context::<RepoListResource>().map(|ctx| ctx.0);
    Effect::new(move |_| {
        if let Some(Ok(resp)) = claim.get()
            && resp.success
            && let Some(repo) = resp.repo
        {
            log!("Claim succeeded; navigating to /repo/{}", repo.local_repo);
            if let Some(r) = repos {
                r.refetch();
            }
            navigate(&format!("/repo/{}", repo.local_repo), Default::default());
        }
    });

    view! {
        <section class="join">
            <h1>"Join a repo"</h1>
            {move || match status.get() {
                ClaimStatus::Pending => view! {
                    <p class="status">"Claiming your invite in this browser…"</p>
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
    }
}
