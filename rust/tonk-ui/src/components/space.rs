use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};

use crate::{api, components::Status};

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkSpaceParams {
    space: Option<String>,
}

/// Main workspace area for displaying a repository.
///
/// Fetches the repository record at `/api/repository/{space}`. The
/// component uses [`Suspense`] to render a fallback while the
/// request is in flight, and [`ErrorBoundary`] to render a fallback
/// for genuine failures (network errors, 5xx). A 404 is *not* an
/// error — it's surfaced as `Ok(None)` from the API so it can flow
/// through the normal value path and render a dedicated view.
///
/// If the `:space` segment is missing, redirects to
/// `/space/{DEFAULT_REPO}`.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpace() -> impl IntoView {
    let params = use_params::<TonkSpaceParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
    });

    // Wait for the shell to finish init (service worker active
    // AND default repository PUT completed) before firing any
    // request. Deep-link loads like `/space/home` mount this
    // component on the very first render, when the service
    // worker is still installing — firing now would race the
    // SW handover, which the browser handles by cancelling the
    // in-flight fetch ("Fetch failed loading"). Reading the
    // `Status` signal here makes the resource re-fire the
    // moment init flips to `Ready`.
    let status = use_context::<Signal<Status, LocalStorage>>();
    let repository = LocalResource::new(move || {
        let name = space_name.get();
        let ready = status.map(|s| s.get() == Status::Ready).unwrap_or(true);
        async move {
            if !ready {
                return Ok(None);
            }
            match name {
                None => {
                    BrowserUrl::redirect(&format!("/space/{}", api::DEFAULT_REPO));
                    // Never-shown placeholder — the redirect tears down
                    // this component before the value is rendered.
                    Ok(None)
                }
                Some(name) => api::repository(&name).await,
            }
        }
    });

    view! {
        <section class="space">
            <Suspense fallback=|| view! { <span class="loading">"Loading…"</span> }>
                <ErrorBoundary fallback=|errors| view! {
                    <section class="error">
                        { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                    </section>
                }>
                    { move || repository.get().map(|result| result.map(|repo| match repo {
                        Some(status) => Either::Left(view! {
                            <pre class="repository">
                                { serde_json::to_string_pretty(&status).unwrap_or_default() }
                            </pre>
                        }),
                        None => Either::Right(view! {
                            <section class="not-found">
                                { move || format!(
                                    "Repository '{}' not found",
                                    space_name.get().unwrap_or_default(),
                                ) }
                            </section>
                        }),
                    })) }
                </ErrorBoundary>
            </Suspense>
        </section>
    }
}
