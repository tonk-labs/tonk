use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::{NewRepoVisible, RepoListResource, Status};

/// Toolbar: vertical column with app controls, the `+` new-repo
/// button, the list of registered repos, and the user menu.
#[component]
pub fn TonkToolbar() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");
    let repos = use_context::<RepoListResource>()
        .expect("RepoListResource provided by TonkLauncher")
        .0;
    let new_repo_visible = use_context::<NewRepoVisible>()
        .expect("NewRepoVisible provided by TonkLauncher")
        .0;

    let open_new_repo = move |_| {
        new_repo_visible.set(true);
    };

    view! {
        <section
            class="toolbar"
            class:visible=move || status.get() == Status::Ready
        >
            <img src="/images/tonk-logo.svg" />
            <button class="plus" on:click=open_new_repo aria-label="New repository">
                <img src="/images/circle-plus.svg" width="36"/>
            </button>
            <nav class="repos">
                {move || match repos.get() {
                    None => view! { <span class="loading">"…"</span> }.into_any(),
                    Some(Err(msg)) => view! { <span class="error">{msg}</span> }.into_any(),
                    Some(Ok(names)) => names.into_iter().map(|name| {
                        let initial = name
                            .chars()
                            .next()
                            .map(|c| c.to_ascii_uppercase().to_string())
                            .unwrap_or_default();
                        let href = format!("/space/{}", name);
                        view! {
                            <A href=href attr:class="repo-row" attr:title=name.clone()>
                                <span class="mark">{initial}</span>
                            </A>
                        }
                    }).collect_view().into_any(),
                }}
            </nav>
            <div class="spacer"></div>
            <img src="/images/question-mark-circle.svg" width="30"/>
            <img src="/images/moon.svg" width="30"/>
            <img src="/images/dummy-avatar.png" width="40"/>
        </section>
    }
}
