use leptos::prelude::*;

use crate::components::{ProfileResource, space::repository_view};

/// Profile view, rendered at `/profile`.
///
/// Uses the same structured view as [`TonkSpace`] — the profile
/// is a repository in the schema sense (it has its own meta
/// branch, replica record, etc.), so rendering it with
/// [`repository_view`] keeps the two screens visually consistent.
///
/// Reads the shell's shared [`ProfileResource`] so creating a new
/// space (which refetches that resource) automatically refreshes
/// this screen too.
///
/// [`TonkSpace`]: super::TonkSpace
/// [`repository_view`]: super::space::repository_view
/// [`ProfileResource`]: super::ProfileResource
#[component]
pub fn TonkProfile() -> impl IntoView {
    let profile = use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");

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
