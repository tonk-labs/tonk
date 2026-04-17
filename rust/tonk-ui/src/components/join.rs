//! `/join` route: browser-side invite claim flow.
//!
//! Reads the full current URL (including `#fragment`) and POSTs it to the
//! service worker's `/api/invite/claim` endpoint, which does the actual
//! parse/redelegate/persist work via `tonk-invite`. This component is a
//! thin status surface; the claim logic lives in the service worker.

use crate::api;
use leptos::{logging::log, prelude::*};

/// Status of the in-progress claim.
#[derive(Clone, Debug)]
enum ClaimStatus {
    /// Waiting for the service worker and the claim request.
    Pending,
    /// Claim succeeded; holds the subject (repo DID).
    Ok { subject: String },
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

/// `/join` route component. Kicks off the claim flow on mount and renders
/// pending / ok / error status to the user.
#[component]
pub fn TonkJoin() -> impl IntoView {
    let claim = LocalResource::new(|| async {
        let url = current_url().ok_or_else(|| {
            crate::error::TonkUiError::ApiError("could not read current URL".into())
        })?;
        log!("Join flow: claiming URL");
        api::claim_invite(&url).await
    });

    let status = Signal::derive_local(move || match claim.get() {
        None => ClaimStatus::Pending,
        Some(Err(e)) => ClaimStatus::Err(format!("{e}")),
        Some(Ok(resp)) if resp.success => ClaimStatus::Ok {
            subject: resp
                .subject
                .unwrap_or_else(|| "(unknown subject)".to_string()),
        },
        Some(Ok(resp)) => ClaimStatus::Err(resp.error.unwrap_or_else(|| "claim failed".into())),
    });

    view! {
        <section class="join">
            <h1>"Join a space"</h1>
            {move || match status.get() {
                ClaimStatus::Pending => view! {
                    <p class="status">"Claiming your invite in this browser…"</p>
                }.into_any(),
                ClaimStatus::Ok { subject } => view! {
                    <p class="status ok">"Invite claimed."</p>
                    <code class="did">{subject}</code>
                }.into_any(),
                ClaimStatus::Err(msg) => view! {
                    <p class="status error">"Could not claim this invite:"</p>
                    <code class="error">{msg}</code>
                }.into_any(),
            }}
        </section>
    }
}
