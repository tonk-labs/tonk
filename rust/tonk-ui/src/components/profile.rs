use leptos::{either::Either, prelude::*};

use crate::components::{
    ProfileResource,
    space::{BranchRow, RemoteCard},
};

/// Profile view, rendered at `/profile`.
///
/// The profile is a repository in the schema sense (it has its
/// own meta branch, replica record, etc.) so it renders with the
/// same `BranchRow` / `RemoteCard` components as a regular space.
/// What's special is the data source: instead of fetching the
/// repository directly we read it off the shared
/// [`ProfileResource`] (`info.profile`), which the shell already
/// keeps fresh on `/api/profile` broadcasts.
///
/// [`ProfileResource`]: super::ProfileResource
#[component]
pub fn TonkProfile() -> impl IntoView {
    let profile_resource =
        use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");

    view! {
        <Suspense fallback=|| view! { <wa-spinner></wa-spinner> }>
            <ErrorBoundary fallback=|errors| view! {
                <wa-callout variant="danger">
                    <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                    { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                </wa-callout>
            }>
                { move || profile_resource.get().map(|result| result.map(|info| match info {
                    Some(info) => Either::Left(render_profile(info.profile)),
                    None => Either::Right(view! {
                        <wa-callout variant="neutral">
                            <wa-icon slot="icon" name="circle-info"></wa-icon>
                            "Profile not yet loaded"
                        </wa-callout>
                    }),
                })) }
            </ErrorBoundary>
        </Suspense>
    }
}

/// Layout for the profile-as-repository: banner with the profile
/// name, branches list, remotes list. Mirrors the space banner
/// minus the share button (you don't share a profile the way you
/// share a space).
fn render_profile(info: tonk_worker::RepositoryInfo) -> impl IntoView {
    let local_subject = info.subject.to_string();
    let title_attr = local_subject.clone();
    let banner_title = info.name.clone();

    let mut branches: Vec<(String, tonk_worker::BranchConfiguration)> =
        info.branch.into_iter().collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut remotes: Vec<(String, tonk_worker::RemoteConfiguration)> =
        info.remote.into_iter().collect();
    remotes.sort_by(|(a, _), (b, _)| a.cmp(b));

    let force_open_solo = branches.len() == 1;
    let branch_rows = branches
        .into_iter()
        .map(|(name, config)| {
            view! {
                <BranchRow
                    name=name
                    config=config
                    force_open=force_open_solo
                />
            }
        })
        .collect::<Vec<_>>();

    let remote_cards = remotes
        .into_iter()
        .map(|(name, config)| {
            view! {
                <RemoteCard
                    name=name
                    config=config
                    local_subject=local_subject.clone()
                />
            }
        })
        .collect::<Vec<_>>();

    let branch_count = branch_rows.len();
    let remote_count = remote_cards.len();

    view! {
        <header slot="main-header" class="space-banner">
            <h1 class="space-banner-title" title=title_attr>
                { banner_title }
            </h1>
        </header>
        <main class="wa-stack space-view">
            <section class="wa-stack">
                <h2 class="space-section-title">{ format!("Branches ({branch_count})") }</h2>
                { if branch_rows.is_empty() {
                    Either::Left(view! {
                        <wa-callout variant="neutral">"no branches recorded"</wa-callout>
                    })
                } else {
                    Either::Right(view! { <div class="branch-list">{ branch_rows }</div> })
                } }
            </section>
            <section class="wa-stack">
                <h2 class="space-section-title">{ format!("Remotes ({remote_count})") }</h2>
                { if remote_cards.is_empty() {
                    Either::Left(view! {
                        <wa-callout variant="neutral">"no remotes recorded"</wa-callout>
                    })
                } else {
                    Either::Right(view! { <div class="remote-grid">{ remote_cards }</div> })
                } }
            </section>
        </main>
    }
}
