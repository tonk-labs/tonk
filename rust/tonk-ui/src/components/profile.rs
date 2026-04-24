use leptos::prelude::*;

use crate::{
    api,
    components::{Status, space::repository_view},
};

/// Profile view, rendered at `/profile`.
///
/// Uses the same structured view as [`TonkSpace`] — the profile
/// is a repository in the schema sense (it has its own meta
/// branch, replica record, etc.), so rendering it with
/// [`repository_view`] keeps the two screens visually consistent.
///
/// The profile's `RepositoryInfo` that comes back from
/// `GET /api/profile` intentionally has empty `branch` and
/// `remote` maps — this route does not probe the profile repo
/// beyond the replica index that feeds the sidebar. The space
/// view handles empty maps by rendering "no branches recorded" /
/// "no remotes recorded" sections, which is the desired shape
/// here.
///
/// [`TonkSpace`]: super::TonkSpace
/// [`repository_view`]: super::space::repository_view
#[component]
pub fn TonkProfile() -> impl IntoView {
    // Same `Status::Ready` gate as `TonkSpace` — deep-linking to
    // `/profile` before the service worker has finished
    // installing would otherwise race the SW handover.
    let status = use_context::<Signal<Status, LocalStorage>>();
    let profile = LocalResource::new(move || {
        let ready = status.map(|s| s.get() == Status::Ready).unwrap_or(true);
        async move {
            if !ready {
                return Ok(None);
            }
            api::profile().await.map(Some)
        }
    });

    view! {
        <Suspense fallback=|| view! {
            <wa-spinner></wa-spinner>
        }>
            <ErrorBoundary fallback=|errors| view! {
                <wa-callout variant="danger">
                    <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                    { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                </wa-callout>
            }>
                { move || profile.get().map(|result| result.map(|info| {
                    info.map(|info| repository_view(info.profile))
                })) }
            </ErrorBoundary>
        </Suspense>
    }
}
