use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};
use tonk_worker::{BranchConfiguration, RemoteConfiguration, RepositoryInfo};

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
                        Some(info) => Either::Left(repository_view(info)),
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

/// Render a [`RepositoryInfo`] as a structured detail view.
///
/// Layout: a header with the repo name and DID, an identity
/// block showing profile/operator, and two card grids for
/// branches and remotes. A `None` upstream or empty map is
/// rendered as an "empty" placeholder so the reader doesn't
/// have to guess whether the absence is data or a UI bug.
fn repository_view(info: RepositoryInfo) -> impl IntoView {
    // Sort branches / remotes by name so the view is stable
    // across renders — `HashMap` iteration order is otherwise
    // nondeterministic.
    let mut branches: Vec<(String, BranchConfiguration)> = info.branch.into_iter().collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut remotes: Vec<(String, RemoteConfiguration)> = info.remote.into_iter().collect();
    remotes.sort_by(|(a, _), (b, _)| a.cmp(b));

    let subject = info.subject.to_string();
    let operator = info.operator.to_string();
    let profile = info.profile.to_string();

    let branch_cards = branches
        .into_iter()
        .map(|(name, config)| {
            let upstream = config.upstream.map(|up| format!("{}/{}", up.remote, up.branch));
            let revision = config.revision.map(|rev| {
                // Split the revision into two human-readable
                // parts: a compact version string and the tree
                // hash. `TreeReference`'s `Display` renders as
                // `#<base58>` — short and already identifies the
                // prolly-tree root without needing to see its
                // debug form.
                let version = format!("{}.{}", rev.period, rev.moment);
                let tree = rev.tree.to_string();
                (version, tree)
            });
            view! {
                <article class="card">
                    <div class="card-header">
                        <span class="card-name">{ name.clone() }</span>
                        <span class="card-kind">"branch"</span>
                    </div>
                    <dl class="fields">
                        <dt>"upstream"</dt>
                        <dd>{
                            match upstream {
                                Some(u) => Either::Left(view! { <code>{ u }</code> }),
                                None => Either::Right(view! { <span class="empty">"none"</span> }),
                            }
                        }</dd>
                        <dt>"version"</dt>
                        <dd>{
                            match revision.as_ref().map(|(v, _)| v.clone()) {
                                Some(v) => Either::Left(view! { <code>{ v }</code> }),
                                None => Either::Right(view! { <span class="empty">"no commits"</span> }),
                            }
                        }</dd>
                        <dt>"tree"</dt>
                        <dd>{
                            match revision.as_ref().map(|(_, t)| t.clone()) {
                                Some(t) => Either::Left(view! { <code>{ t }</code> }),
                                None => Either::Right(view! { <span class="empty">"—"</span> }),
                            }
                        }</dd>
                    </dl>
                </article>
            }
        })
        .collect::<Vec<_>>();

    // Local repository subject, used to decide whether a
    // remote's subject DID matches our own. Always shown on
    // every remote card — `None` on the wire means "same as
    // local," but the UI still displays the concrete DID plus
    // a badge that indicates whether it matches.
    let local_subject = info.subject.to_string();

    let remote_cards = remotes
        .into_iter()
        .map(|(name, config)| {
            let address = serde_json::to_string_pretty(&config.address).unwrap_or_default();
            let remote_subject = match &config.subject {
                Some(did) => did.to_string(),
                None => local_subject.clone(),
            };
            let is_same = config.subject.is_none();
            let badge_class = if is_same { "badge badge-same" } else { "badge badge-other" };
            let badge_text = if is_same { "same" } else { "other" };
            view! {
                <article class="card">
                    <div class="card-header">
                        <span class="card-name">{ name.clone() }</span>
                        <span class="card-kind">"remote"</span>
                    </div>
                    <dl class="fields">
                        <dt>"subject"</dt>
                        <dd class="subject-row">
                            <code>{ remote_subject }</code>
                            <span class=badge_class>{ badge_text }</span>
                        </dd>
                        <dt>"address"</dt>
                        <dd><code>{ address }</code></dd>
                    </dl>
                </article>
            }
        })
        .collect::<Vec<_>>();

    view! {
        <article class="repository">
            <header>
                <span class="eyebrow">"repository"</span>
                <h1>{ info.name.clone() }</h1>
                <dl class="fields">
                    <dt>"subject"</dt>
                    <dd><code>{ subject }</code></dd>
                </dl>
            </header>

            <section>
                <h2>"Identity"</h2>
                <dl class="fields">
                    <dt>"profile"</dt>
                    <dd><code>{ profile }</code></dd>
                    <dt>"operator"</dt>
                    <dd><code>{ operator }</code></dd>
                </dl>
            </section>

            <section>
                <h2>{ format!("Branches ({})", branch_cards.len()) }</h2>
                {
                    if branch_cards.is_empty() {
                        Either::Left(view! { <div class="empty">"no branches recorded"</div> })
                    } else {
                        Either::Right(view! { <div class="cards">{ branch_cards }</div> })
                    }
                }
            </section>

            <section>
                <h2>{ format!("Remotes ({})", remote_cards.len()) }</h2>
                {
                    if remote_cards.is_empty() {
                        Either::Left(view! { <div class="empty">"no remotes recorded"</div> })
                    } else {
                        Either::Right(view! { <div class="cards">{ remote_cards }</div> })
                    }
                }
            </section>
        </article>
    }
}
