//! First-run modal offering the two entry points into a fresh profile:
//! create a new repo, or redeem an invite.
//!
//! The "create" CTA POSTs to `/api/repository/create`; on success the
//! user is navigated to `/repo/<local_repo>`. The "redeem" CTA navigates
//! to the existing [`crate::components::TonkJoin`] route.

use leptos::{logging::log, prelude::*, task::spawn_local};
use leptos_router::{components::A, hooks::use_navigate};
use tonk_worker::{CreateRepositoryRequest, RemoteConfig};

use crate::api;
use crate::components::RepoListResource;

/// Which remote option the user picked in the create-repo form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteChoice {
    Default,
    Url,
    None,
}

/// First-run modal. Rendered by the index route when the profile has no
/// repos yet; dismissed implicitly once the user picks an entry point
/// and the create/claim flow redirects away.
#[component]
pub fn TonkFirstRunModal() -> impl IntoView {
    let name = RwSignal::new(String::new());
    let choice = RwSignal::new(RemoteChoice::Default);
    let url = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let submit = move |_| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);

        let remote = match choice.get_untracked() {
            RemoteChoice::Default => RemoteConfig::Default,
            RemoteChoice::Url => RemoteConfig::Url {
                url: url.get_untracked(),
            },
            RemoteChoice::None => RemoteConfig::None,
        };
        let raw_name = name.get_untracked();
        let name_opt = (!raw_name.trim().is_empty()).then(|| raw_name.trim().to_string());
        let req = CreateRepositoryRequest {
            name: name_opt,
            remote,
        };

        let navigate = use_navigate();
        let repos = use_context::<RepoListResource>().map(|ctx| ctx.0);
        spawn_local(async move {
            match api::create_repository(&req).await {
                Ok(resp) if resp.success => match resp.repo {
                    Some(repo) => {
                        log!("Created repo {}; navigating", repo.local_repo);
                        if let Some(r) = repos {
                            r.refetch();
                        }
                        navigate(&format!("/repo/{}", repo.local_repo), Default::default());
                    }
                    None => {
                        error.set(Some("create succeeded but response had no repo".into()));
                    }
                },
                Ok(resp) => {
                    error.set(Some(resp.error.unwrap_or_else(|| "create failed".into())));
                }
                Err(e) => error.set(Some(format!("{e}"))),
            }
            pending.set(false);
        });
    };

    view! {
        <dialog class="first-run" open=true>
            <h2>"Get started"</h2>
            <p>"Create a new repo, or redeem an invite someone shared with you."</p>

            <form class="create-repo" on:submit=move |ev| {
                ev.prevent_default();
                submit(());
            }>
                <label>
                    "Name (optional)"
                    <input type="text"
                        placeholder="my-journal"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>

                <fieldset class="remote">
                    <legend>"Sync remote"</legend>

                    <label>
                        <input type="radio" name="remote"
                            prop:checked=move || choice.get() == RemoteChoice::Default
                            on:change=move |_| choice.set(RemoteChoice::Default)
                        />
                        "Default"
                    </label>

                    <label>
                        <input type="radio" name="remote"
                            prop:checked=move || choice.get() == RemoteChoice::Url
                            on:change=move |_| choice.set(RemoteChoice::Url)
                        />
                        "Custom URL"
                    </label>
                    <input type="text"
                        placeholder="https://access.example.com/ucan/"
                        prop:value=move || url.get()
                        prop:disabled=move || choice.get() != RemoteChoice::Url
                        on:input=move |ev| url.set(event_target_value(&ev))
                    />

                    <label>
                        <input type="radio" name="remote"
                            prop:checked=move || choice.get() == RemoteChoice::None
                            on:change=move |_| choice.set(RemoteChoice::None)
                        />
                        "Local only"
                    </label>
                </fieldset>

                <div class="ctas">
                    <button type="submit" class="create" prop:disabled=move || pending.get()>
                        {move || if pending.get() { "Creating…" } else { "Create repo" }}
                    </button>
                    <A href="/join" attr:class="redeem">"Redeem invite"</A>
                </div>

                {move || error.get().map(|msg| view! {
                    <p class="error">{ msg }</p>
                })}
            </form>
        </dialog>
    }
}
