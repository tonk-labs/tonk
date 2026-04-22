//! New-repo overlay triggered by the `+` button in [`super::TonkToolbar`].
//!
//! Renders an `.auth .panel` overlay with a name input. On submit,
//! PUTs `/api/repository/{name}`, refreshes the shared repo list, and
//! navigates to `/space/{name}`. While the request is in flight the
//! panel transitions to its `.pending` loading state.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

use crate::api;
use crate::components::{NewRepoVisible, RepoListResource};

/// Overlay component. Always mounted; visibility gated by
/// [`NewRepoVisible`] context and the `.auth.pending` class.
#[component]
pub fn TonkNewRepo() -> impl IntoView {
    let visible = use_context::<NewRepoVisible>()
        .expect("NewRepoVisible provided by TonkLauncher")
        .0;
    let repos = use_context::<RepoListResource>()
        .expect("RepoListResource provided by TonkLauncher")
        .0;

    let name = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let cancel = move |_| {
        if pending.get_untracked() {
            return;
        }
        name.set(String::new());
        error.set(None);
        visible.set(false);
    };

    let submit = move |_| {
        if pending.get_untracked() {
            return;
        }
        let raw = name.get_untracked();
        let target = raw.trim().to_string();
        if target.is_empty() {
            error.set(Some("Name is required".into()));
            return;
        }

        pending.set(true);
        error.set(None);

        let navigate = use_navigate();
        spawn_local(async move {
            match api::create(&target).await {
                Ok(info) => {
                    repos.refetch();
                    visible.set(false);
                    name.set(String::new());
                    pending.set(false);
                    navigate(&format!("/space/{}", info.name), Default::default());
                }
                Err(e) => {
                    error.set(Some(format!("{e}")));
                    pending.set(false);
                }
            }
        });
    };

    view! {
        <section
            class="auth new-repo"
            class:pending=move || visible.get()
        >
            <form
                class="panel"
                class:pending=move || pending.get()
                on:submit=move |ev| {
                    ev.prevent_default();
                    submit(());
                }
            >
                <h2>"New repository"</h2>
                <input
                    type="text"
                    placeholder="Name"
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    autofocus
                />
                {move || error.get().map(|msg| view! {
                    <p class="error">{msg}</p>
                })}
                <div class="actions">
                    <button type="button" on:click=cancel>"Cancel"</button>
                    <button type="submit">"Create"</button>
                </div>
            </form>
        </section>
    }
}
