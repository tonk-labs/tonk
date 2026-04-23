use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};

use crate::{api, components::HostId};

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
/// Once the repository record is fetched, the space is presented
/// inside a sandboxed guest iframe pointed at
/// `/api/host/{host_id}/guest/{space}/`. The SW serves a
/// pretty-printed JSON view of the repository as the iframe's
/// content; the guest iframe reaches back into the SW to pull
/// that info itself.
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

    let host_id = use_context::<Signal<Option<HostId>, LocalStorage>>();

    let repository = LocalResource::new(move || {
        let name = space_name.get();
        let ready = host_id.and_then(|s| s.get()).is_some();
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
                        Some(_info) => {
                            let space = space_name.get().unwrap_or_default();
                            let host = host_id.and_then(|s| s.get()).map(|h| h.0);
                            Either::Left(match host {
                                Some(host) => Either::Left(view! {
                                    <iframe
                                        class="guest"
                                        sandbox="allow-scripts allow-same-origin"
                                        src=format!(
                                            "/api/host/{}/guest/{}/",
                                            host, space,
                                        )
                                    />
                                }),
                                None => Either::Right(view! {
                                    <span class="loading">
                                        "Waiting for service worker…"
                                    </span>
                                }),
                            })
                        }
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
