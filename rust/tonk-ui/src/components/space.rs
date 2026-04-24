use dialog_repository::SiteAddress;
use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};
use tonk_worker::{BranchConfiguration, RemoteConfiguration, RepositoryInfo};

use crate::{
    api,
    components::{ActiveSubject, Status},
    did,
};

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

    // Publish the current space's subject DID so the sidebar
    // toolbar can render a matching sigil. Cleared back to `None`
    // on load failure / 404, so the sidebar falls back to its
    // empty state.
    let active_subject =
        use_context::<ActiveSubject>().expect("ActiveSubject context provided by TonkShell");
    Effect::new(move |_| {
        let subject = repository
            .get()
            .and_then(|result| result.ok())
            .flatten()
            .map(|info| info.subject.to_string());
        active_subject.set(subject);
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
                { move || repository.get().map(|result| result.map(|repo| match repo {
                    Some(info) => Either::Left(repository_view(info)),
                    None => Either::Right(view! {
                        <wa-callout variant="neutral">
                            <wa-icon slot="icon" name="circle-info"></wa-icon>
                            { move || format!(
                                "Repository '{}' not found",
                                space_name.get().unwrap_or_default(),
                            ) }
                        </wa-callout>
                    }),
                })) }
            </ErrorBoundary>
        </Suspense>
    }
}

/// Render a [`RepositoryInfo`] as a structured detail view.
fn did_sigil_value(did: &str) -> Option<String> {
    did::did_key_prefix(did).map(|bytes| {
        let n = u32::from_be_bytes(bytes);
        format!("0x{n:08x}")
    })
}

fn repository_view(info: RepositoryInfo) -> impl IntoView {
    // Sort branches / remotes by name so the view is stable
    // across renders — `HashMap` iteration order is otherwise
    // nondeterministic.
    let mut branches: Vec<(String, BranchConfiguration)> = info.branch.into_iter().collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut remotes: Vec<(String, RemoteConfiguration)> = info.remote.into_iter().collect();
    remotes.sort_by(|(a, _), (b, _)| a.cmp(b));

    let branch_cards = branches
        .into_iter()
        .map(|(name, config)| {
            let upstream = config.upstream.map(|up| format!("{}/{}", up.remote, up.branch));
            let revision = config.revision.map(|rev| {
                let version = format!("{}.{}", rev.period, rev.moment);
                let tree_full = rev.tree.to_string();
                let tree_short = abbreviate_tree(&tree_full);
                (version, tree_full, tree_short)
            });
            view! {
                <wa-card class="branch-card">
                    <div class="wa-stack wa-gap-2xs">
                        <strong class="branch-card-name">{ name.clone() }</strong>
                        <div>
                            "upstream: "
                            { match upstream {
                                Some(u) => Either::Left(view! { <wa-tag variant="neutral">{ u }</wa-tag> }),
                                None => Either::Right(view! { <span>"none"</span> }),
                            } }
                        </div>
                        <div>
                            "version: "
                            { match revision.as_ref().map(|(v, _, _)| v.clone()) {
                                Some(v) => Either::Left(view! { <wa-tag variant="neutral">{ v }</wa-tag> }),
                                None => Either::Right(view! { <span>"no commits"</span> }),
                            } }
                        </div>
                        <div>
                            "tree: "
                            { match revision.as_ref().map(|(_, full, short)| (full.clone(), short.clone())) {
                                Some((full, short)) => Either::Left(view! {
                                    <wa-tag variant="neutral" title=full>{ short }</wa-tag>
                                }),
                                None => Either::Right(view! { <span>"—"</span> }),
                            } }
                        </div>
                    </div>
                </wa-card>
            }
        })
        .collect::<Vec<_>>();

    // Remote's subject defaults to the local repository's subject
    // when omitted on the wire. The rendered sigil is keyed on
    // whichever it ends up being, so same-peer remotes visually
    // match the sidebar.
    let local_subject = info.subject.to_string();

    let remote_tiles = remotes
        .into_iter()
        .map(|(name, config)| {
            let summary = summarize_address(&config.address);
            let remote_subject = match &config.subject {
                Some(did) => did.to_string(),
                None => local_subject.clone(),
            };
            let sigil_value = did_sigil_value(&remote_subject);
            let subject_title = remote_subject.clone();
            view! {
                <wa-card appearance="plain" orientation="horizontal" class="remote-card">
                    <tonk-sigil slot="media" value=sigil_value></tonk-sigil>
                    <div class="wa-stack wa-gap-2xs">
                        <strong>{ name.clone() }</strong>
                        <code class="remote-card-subject" title=subject_title>
                            { remote_subject }
                        </code>
                        <code>{ summary.url }</code>
                        { summary.details.map(|detail| view! { <small>{ detail }</small> }) }
                    </div>
                </wa-card>
            }
        })
        .collect::<Vec<_>>();

    let branch_count = branch_cards.len();
    let remote_count = remote_tiles.len();

    view! {
        <header slot="main-header" class="space-banner">
            <h1>{ info.name.clone() }</h1>
        </header>
        <main class="wa-stack space-view">
            <section class="wa-stack">
                <h2 class="space-section-title">{ format!("Branches ({branch_count})") }</h2>
                { if branch_cards.is_empty() {
                    Either::Left(view! {
                        <wa-callout variant="neutral">"no branches recorded"</wa-callout>
                    })
                } else {
                    Either::Right(view! { <div class="wa-grid">{ branch_cards }</div> })
                } }
            </section>
            <section class="wa-stack">
                <h2 class="space-section-title">{ format!("Remotes ({remote_count})") }</h2>
                { if remote_tiles.is_empty() {
                    Either::Left(view! {
                        <wa-callout variant="neutral">"no remotes recorded"</wa-callout>
                    })
                } else {
                    Either::Right(view! { <div class="wa-stack">{ remote_tiles }</div> })
                } }
            </section>
        </main>
    }
}

/// Github-style short form of a tree reference. `TreeReference`'s
/// `Display` produces `#<base58>`; this drops the `#` marker and
/// truncates the base58 body to 8 chars. Callers should expose the
/// full value via a `title` attribute for hover disclosure.
fn abbreviate_tree(tree: &str) -> String {
    const SHORT_LEN: usize = 8;
    let body = tree.strip_prefix('#').unwrap_or(tree);
    body.chars().take(SHORT_LEN).collect()
}

/// A remote's address distilled to the minimum a human needs at a glance.
/// UCAN: just the service endpoint URL. S3: the endpoint plus a
/// secondary line with bucket and region.
struct RemoteAddressSummary {
    url: String,
    details: Option<String>,
}

fn summarize_address(address: &SiteAddress) -> RemoteAddressSummary {
    match address {
        SiteAddress::Ucan(addr) => RemoteAddressSummary {
            url: addr.endpoint().to_string(),
            details: None,
        },
        SiteAddress::S3(addr) => RemoteAddressSummary {
            url: addr.endpoint().to_string(),
            details: Some(format!("{} · {}", addr.bucket(), addr.region())),
        },
    }
}
