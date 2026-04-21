//! Repo list sidebar.
//!
//! Consumes the [`RepoListResource`] context provided by
//! [`crate::components::TonkLauncher`] and renders one row per
//! [`RepoEntry`]. Rows are `<A>` links that navigate to the repo view.

use leptos::prelude::*;
use leptos_router::components::A;
use tonk_worker::RepoEntry;

/// Context-carried handle for the shared repo-list resource. Owned by
/// [`crate::components::TonkLauncher`]; consumers include the sidebar
/// (renders rows), the empty-state gate (shows first-run modal when the
/// list is empty), and the create/claim flows (call `.refetch()` on
/// success).
#[derive(Clone, Copy)]
pub struct RepoListResource(pub LocalResource<Result<Vec<RepoEntry>, String>>);

/// Left-hand repo list.
#[component]
pub fn TonkSidebar() -> impl IntoView {
    let repos = use_context::<RepoListResource>()
        .expect("RepoListResource provided by TonkLauncher")
        .0;

    view! {
        <aside class="sidebar">
            <div class="brand">
                <img src="/images/mark-white.svg" alt="" />
                <span>"Tonk"</span>
            </div>
            <ul class="repos">
                {move || match repos.get() {
                    None => view! { <li class="loading">"Loading…"</li> }.into_any(),
                    Some(Err(msg)) => view! { <li class="error">{msg}</li> }.into_any(),
                    Some(Ok(entries)) if entries.is_empty() => view! {
                        <li class="empty">"No repos yet"</li>
                    }.into_any(),
                    Some(Ok(entries)) => entries.into_iter().map(|entry| {
                        let href = format!("/repo/{}", entry.local_repo);
                        view! {
                            <li class="repo-row">
                                <A href=href>
                                    <span class="local-name">{entry.local_repo}</span>
                                    <code class="subject">{entry.subject}</code>
                                </A>
                            </li>
                        }
                    }).collect_view().into_any(),
                }}
            </ul>
        </aside>
    }
}
