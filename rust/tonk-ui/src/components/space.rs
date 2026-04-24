use leptos::task::spawn_local;
use leptos::{either::Either, prelude::*};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};
use tonk_worker::{RemoteConfiguration, RepositoryInfo};

use crate::api;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkSpaceParams {
    space: Option<String>,
}

const DEFAULT_BRANCH: &str = "main";

/// Status of the invite-mint request.
#[derive(Clone, Debug)]
enum InviteState {
    /// Nothing in flight; panel shows only the "Create invite" button.
    Idle,
    /// POST in flight.
    Minting,
    /// Successful mint — display the URL for the user to copy.
    Ok(String),
    /// Mint failed.
    Failed(String),
}

/// Which sync operation is/was in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncOp {
    Pull,
    Push,
}

impl SyncOp {
    fn running(self) -> &'static str {
        match self {
            SyncOp::Pull => "Pulling",
            SyncOp::Push => "Pushing",
        }
    }

    fn past(self) -> &'static str {
        match self {
            SyncOp::Pull => "Pulled",
            SyncOp::Push => "Pushed",
        }
    }
}

/// Sync status for the current space. Tracks both pull and push so
/// whichever button was last clicked dictates what the status line
/// shows.
#[derive(Clone, Debug)]
enum SyncState {
    Idle,
    Running(SyncOp),
    Done(SyncOp),
    Failed { op: SyncOp, message: String },
}

/// Main workspace area for displaying a repository.
///
/// Fetches the repository record at `/api/repository/{space}` and
/// renders a structured summary (identifiers, branches, remotes).
/// Pull and claim query are explicit user actions; nothing syncs
/// automatically on mount.
///
/// If `:space` is missing, redirects to `/space/{DEFAULT_REPO}`.
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

    let repository = LocalResource::new(move || {
        let name = space_name.get();
        async move {
            match name {
                None => {
                    BrowserUrl::redirect(&format!("/space/{}", api::DEFAULT_REPO));
                    Ok(None)
                }
                Some(name) => api::repository(&name).await,
            }
        }
    });

    // Sync is strictly user-triggered — no auto-sync on mount. After
    // a successful op we refetch `repository` so the rendered branch
    // revision tracks the new state.
    let sync_state = RwSignal::new(SyncState::Idle);
    let trigger_sync = move |op: SyncOp| {
        let Some(name) = space_name.get() else {
            return;
        };
        if matches!(sync_state.get_untracked(), SyncState::Running(_)) {
            return;
        }
        sync_state.set(SyncState::Running(op));
        spawn_local(async move {
            let result = match op {
                SyncOp::Pull => api::pull(&name, DEFAULT_BRANCH).await,
                SyncOp::Push => api::push(&name, DEFAULT_BRANCH).await,
            };
            match result {
                Ok(resp) if resp.success => {
                    sync_state.set(SyncState::Done(op));
                    repository.refetch();
                }
                Ok(resp) => {
                    sync_state.set(SyncState::Failed {
                        op,
                        message: resp
                            .error
                            .unwrap_or_else(|| format!("{} failed", op.past().to_lowercase())),
                    });
                }
                Err(e) => sync_state.set(SyncState::Failed {
                    op,
                    message: format!("{e}"),
                }),
            }
        });
    };
    let pull_click = move |_| trigger_sync(SyncOp::Pull);
    let push_click = move |_| trigger_sync(SyncOp::Push);

    // Invite minting: a single user-triggered action per click. Open
    // invites (no audience) are the default — the ephemeral seed is
    // embedded in the URL fragment so anyone with the link can claim.
    let invite_state = RwSignal::new(InviteState::Idle);
    let mint_invite = move |_| {
        let Some(name) = space_name.get() else {
            return;
        };
        if matches!(invite_state.get_untracked(), InviteState::Minting) {
            return;
        }
        invite_state.set(InviteState::Minting);
        spawn_local(async move {
            match api::create_invite(&name, None).await {
                Ok(resp) => invite_state.set(InviteState::Ok(resp.url)),
                Err(e) => invite_state.set(InviteState::Failed(format!("{e}"))),
            }
        });
    };

    // Query form: two inputs + a submit signal. The claims resource
    // fires only when the submitted query is non-empty.
    let the_input = RwSignal::new(String::new());
    let of_input = RwSignal::new(String::new());
    let submitted = RwSignal::new(None::<(Option<String>, Option<String>)>);

    let claims = LocalResource::new(move || {
        let name = space_name.get();
        let query = submitted.get();
        async move {
            match (name, query) {
                (Some(name), Some((the, of))) => {
                    api::select_claims(&name, DEFAULT_BRANCH, the.as_deref(), of.as_deref())
                        .await
                        .map(|r| Some(r.claims))
                        .map_err(|e| format!("{e}"))
                }
                _ => Ok(None),
            }
        }
    });

    let submit_query = move |_| {
        let the = the_input.get_untracked().trim().to_string();
        let of = of_input.get_untracked().trim().to_string();
        if the.is_empty() && of.is_empty() {
            return;
        }
        submitted.set(Some((
            (!the.is_empty()).then_some(the),
            (!of.is_empty()).then_some(of),
        )));
    };

    // Reset per-space state when navigating to a different space.
    Effect::new(move |prev: Option<Option<String>>| {
        let current = space_name.get();
        if let Some(p) = prev
            && p != current
        {
            the_input.set(String::new());
            of_input.set(String::new());
            submitted.set(None);
            sync_state.set(SyncState::Idle);
            invite_state.set(InviteState::Idle);
        }
        current
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
                        Some(info) => Either::Left(render_repository(info)),
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

            <section class="sync">
                <h2>"Upstream"</h2>
                {move || render_sync_state(sync_state.get())}
                <div class="actions">
                    <button
                        on:click=pull_click
                        prop:disabled=move || matches!(sync_state.get(), SyncState::Running(_))
                    >
                        "Pull main"
                    </button>
                    <button
                        on:click=push_click
                        prop:disabled=move || matches!(sync_state.get(), SyncState::Running(_))
                    >
                        "Push main"
                    </button>
                </div>
            </section>

            <section class="invite">
                <h2>"Invite"</h2>
                {move || render_invite_state(invite_state.get())}
                <div class="actions">
                    <button
                        on:click=mint_invite
                        prop:disabled=move || matches!(invite_state.get(), InviteState::Minting)
                    >
                        "Create invite"
                    </button>
                </div>
            </section>

            <section class="claims">
                <h2>"Claims"</h2>
                <form on:submit=move |ev| { ev.prevent_default(); submit_query(()); }>
                    <input
                        type="text"
                        placeholder="attribute (e.g. user/name)"
                        prop:value=move || the_input.get()
                        on:input=move |ev| the_input.set(event_target_value(&ev))
                    />
                    <input
                        type="text"
                        placeholder="entity (e.g. did:key:…)"
                        prop:value=move || of_input.get()
                        on:input=move |ev| of_input.set(event_target_value(&ev))
                    />
                    <button type="submit">"Query"</button>
                </form>
                {move || match claims.get() {
                    None => view! { <p class="hint">"Submit to query."</p> }.into_any(),
                    Some(Ok(None)) => view! { <p class="hint">"Submit to query."</p> }.into_any(),
                    Some(Err(e)) => view! { <p class="error">{e}</p> }.into_any(),
                    Some(Ok(Some(list))) if list.is_empty() => view! {
                        <p class="hint">"No claims matched."</p>
                    }.into_any(),
                    Some(Ok(Some(list))) => view! {
                        <ul class="claims-list">
                            {list.into_iter().map(|c| view! {
                                <li>
                                    <code class="the">{c.the}</code>
                                    " of "
                                    <code class="of">{c.of}</code>
                                    " is "
                                    <code class="is">
                                        { serde_json::to_string(&c.is).unwrap_or_default() }
                                    </code>
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any(),
                }}
            </section>
        </section>
    }
}

/// Render a loaded [`RepositoryInfo`] as a structured summary.
fn render_repository(info: RepositoryInfo) -> impl IntoView {
    // Deterministic display order for hash-map-backed fields.
    let mut branches: Vec<_> = info.branch.into_iter().collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut remotes: Vec<_> = info.remote.into_iter().collect();
    remotes.sort_by(|(a, _), (b, _)| a.cmp(b));

    view! {
        <section class="repository">
            <h1 class="name">{info.name}</h1>

            <dl class="dids">
                <dt>"Subject"</dt>
                <dd><code>{info.subject.to_string()}</code></dd>
                <dt>"Operator"</dt>
                <dd><code>{info.operator.to_string()}</code></dd>
                <dt>"Profile"</dt>
                <dd><code>{info.profile.to_string()}</code></dd>
            </dl>

            <h2>"Branches"</h2>
            {if branches.is_empty() {
                view! { <p class="hint">"No branches yet."</p> }.into_any()
            } else {
                view! {
                    <ul class="branches">
                        {branches.into_iter().map(|(name, cfg)| {
                            let upstream = cfg
                                .upstream
                                .map(|u| format!("{}/{}", u.remote, u.branch))
                                .unwrap_or_else(|| "(none)".into());
                            let has_revision = cfg.revision.is_some();
                            view! {
                                <li>
                                    <span class="branch-name">{name}</span>
                                    <span class="branch-upstream">
                                        "upstream "
                                        <code>{upstream}</code>
                                    </span>
                                    <span class="branch-revision">
                                        {if has_revision { "has commits" } else { "empty" }}
                                    </span>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}

            <h2>"Remotes"</h2>
            {if remotes.is_empty() {
                view! { <p class="hint">"No remotes configured."</p> }.into_any()
            } else {
                view! {
                    <ul class="remotes">
                        {remotes.into_iter().map(|(name, cfg)| {
                            view! {
                                <li>
                                    <span class="remote-name">{name}</span>
                                    <span class="remote-endpoint">
                                        <code>{render_address(&cfg)}</code>
                                    </span>
                                    {cfg.subject.map(|s| view! {
                                        <span class="remote-subject">
                                            "subject "
                                            <code>{s.to_string()}</code>
                                        </span>
                                    })}
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </section>
    }
}

/// Render a remote address as a short human-readable string.
/// `SiteAddress` has many variants; serializing to JSON is the
/// simplest way to show whatever fields it carries without
/// pulling in the full dialog-repository surface here.
fn render_address(cfg: &RemoteConfiguration) -> String {
    serde_json::to_string(&cfg.address).unwrap_or_else(|_| "(unrenderable)".into())
}

/// Render the invite panel body based on the current mint state.
///
/// `InviteState::Ok` shows the URL in a read-only text input so the
/// user can select + copy it without a separate clipboard API call,
/// which would need extra browser permission handling.
fn render_invite_state(state: InviteState) -> impl IntoView {
    match state {
        InviteState::Idle => view! {
            <p class="status">"No invite yet. Click to mint one."</p>
        }
        .into_any(),
        InviteState::Minting => view! {
            <p class="status">"Minting…"</p>
        }
        .into_any(),
        InviteState::Ok(url) => view! {
            <p class="status ok">"Invite ready. Share this link:"</p>
            <input class="invite-url" type="text" prop:value=url readonly />
        }
        .into_any(),
        InviteState::Failed(message) => view! {
            <p class="status error">{format!("Mint failed: {message}")}</p>
        }
        .into_any(),
    }
}

/// Render the upstream sync status line.
fn render_sync_state(state: SyncState) -> impl IntoView {
    match state {
        SyncState::Idle => view! {
            <p class="status">"Not synced this session."</p>
        }
        .into_any(),
        SyncState::Running(op) => view! {
            <p class="status">{format!("{}…", op.running())}</p>
        }
        .into_any(),
        SyncState::Done(op) => view! {
            <p class="status ok">{format!("{}.", op.past())}</p>
        }
        .into_any(),
        SyncState::Failed { op, message } => view! {
            <p class="status error">{format!("{} failed: {message}", op.past().to_lowercase())}</p>
        }
        .into_any(),
    }
}
