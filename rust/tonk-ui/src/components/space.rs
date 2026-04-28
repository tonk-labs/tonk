use dialog_repository::SiteAddress;
use leptos::{either::Either, prelude::*, task::spawn_local, web_sys};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};
use tonk_worker::{
    BranchConfiguration, ClaimResponse, RemoteConfiguration, RepositoryInfo, Revision,
};
use wasm_bindgen::JsCast;

use crate::{
    api,
    components::{ActiveSubject, InviteSpace, Status},
    did,
};

/// The currently-running (or last-completed) sync operation for a
/// single branch row. Each branch in the space view owns its own
/// `SyncState` so a Pull on `main` and a Push on `dev` can be in
/// flight independently and surface independent chips.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SyncState {
    /// No sync has run on this branch since the view mounted.
    Idle,
    /// A `pull` or `push` is in flight.
    Running(SyncOp),
    /// The most recent operation finished successfully — chip
    /// renders before/after revisions next to the op name.
    /// Revisions are boxed so the `Done` variant doesn't bloat
    /// the enum's stack size (each `Revision` is ~168 bytes).
    Done {
        op: SyncOp,
        before: Option<Box<Revision>>,
        after: Option<Box<Revision>>,
    },
    /// The most recent operation failed — chip surfaces the error.
    Failed { op: SyncOp, message: String },
}

/// Which sync operation a [`SyncState`] entry refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncOp {
    Pull,
    Push,
}

impl SyncOp {
    fn label(self) -> &'static str {
        match self {
            Self::Pull => "Pull",
            Self::Push => "Push",
        }
    }
}

/// Branch we expand by default. Other branches collapse to their
/// summary row; the user opens them on demand.
const DEFAULT_OPEN_BRANCH: &str = "main";

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
    // request. Reading the `Status` signal here makes the
    // resource re-fire the moment init flips to `Ready`.
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
                    Ok(None)
                }
                Some(name) => api::repository(&name).await,
            }
        }
    });

    // Publish the current space's subject DID so the sidebar
    // toolbar can render a matching sigil.
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

    // Open the shared invite dialog when the user clicks the
    // share affordance. Dialog itself drives the mint and renders
    // the URL.
    let invite_space = use_context::<InviteSpace>().expect("InviteSpace provided by TonkShell");
    let on_share = move |_| {
        if let Some(name) = space_name.get() {
            invite_space.set(Some(name));
        }
    };

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
                    Some(info) => Either::Left(render_space_view(
                        info,
                        space_name,
                        repository,
                        on_share,
                    )),
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

/// Top-level layout for a loaded [`RepositoryInfo`]: banner with
/// share button, branches section (each as a `<wa-details>`
/// rendered by [`BranchRow`]), remotes section (compact
/// [`RemoteCard`] tiles).
fn render_space_view<F>(
    info: RepositoryInfo,
    space_name: Signal<Option<String>, LocalStorage>,
    repository: LocalResource<Result<Option<RepositoryInfo>, crate::error::TonkUiError>>,
    on_share: F,
) -> impl IntoView
where
    F: Fn(leptos::ev::MouseEvent) + 'static + Clone,
{
    let local_subject = info.subject.to_string();
    let space_title = info.name.clone();

    // Sort branches/remotes for stable rendering — `HashMap`
    // iteration order is otherwise nondeterministic.
    let mut branches: Vec<(String, BranchConfiguration)> = info.branch.into_iter().collect();
    branches.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut remotes: Vec<(String, RemoteConfiguration)> = info.remote.into_iter().collect();
    remotes.sort_by(|(a, _), (b, _)| a.cmp(b));

    // A solo branch should expand regardless of its name —
    // there's nothing else for the eye to land on. With multiple
    // branches we open `main` by default and leave the rest
    // collapsed. `BranchRow`'s own `name == DEFAULT_OPEN_BRANCH`
    // check still fires; this prop forces it open when that
    // wouldn't otherwise match.
    let force_open_solo = branches.len() == 1;
    let branch_rows = branches
        .into_iter()
        .map(|(name, config)| {
            view! {
                <BranchRow
                    name=name
                    config=config
                    space_name=space_name
                    repository=repository
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
    let title_attr = local_subject.clone();

    view! {
        <header slot="main-header" class="space-banner">
            <h1 class="space-banner-title" title=title_attr>
                { space_title }
            </h1>
            <wa-button
                variant="neutral"
                appearance="accent"
                size="small"
                on:click=on_share
            >
                <wa-icon
                    name="share-nodes"
                    variant="solid"
                    label="Invite someone to this space"
                ></wa-icon>
            </wa-button>
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

/// One branch's `<wa-details>` row.
///
/// Summary line: branch icon, name, version tag, push/pull
/// icon-buttons (only when the branch has an upstream).
///
/// Body: tree hash, upstream details, claim-query form scoped to
/// this branch.
///
/// Per-branch local state — each row owns its own `SyncState`
/// and claim-query signals so concurrent operations on different
/// branches stay isolated.
#[component]
pub(super) fn BranchRow(
    name: String,
    config: BranchConfiguration,
    space_name: Signal<Option<String>, LocalStorage>,
    repository: LocalResource<Result<Option<RepositoryInfo>, crate::error::TonkUiError>>,
    /// Caller may force the row to render expanded — used by the
    /// space view when there's only one branch, so a solo `meta`
    /// (or any other lone branch) still pops open by default.
    /// Combined with the row's own `name == DEFAULT_OPEN_BRANCH`
    /// rule via `||`, so either trigger opens the row.
    #[prop(optional)]
    force_open: bool,
) -> impl IntoView {
    let upstream = config
        .upstream
        .as_ref()
        .map(|up| (up.remote.clone(), up.branch.clone()));
    let upstream_label = upstream
        .as_ref()
        .map(|(remote, branch)| format!("{remote}/{branch}"));
    let tree_pair = config.revision.as_ref().map(|rev| {
        let full = rev.tree.to_string();
        (full.clone(), abbreviate_tree(&full))
    });

    let has_upstream = upstream.is_some();
    let is_default = force_open || name == DEFAULT_OPEN_BRANCH;
    let branch_name = name.clone();

    let sync_state = RwSignal::new(SyncState::Idle);

    let trigger_sync = {
        let branch_name = branch_name.clone();
        move |op: SyncOp| {
            let Some(repo) = space_name.get() else {
                return;
            };
            if matches!(sync_state.get_untracked(), SyncState::Running(_)) {
                return;
            }
            sync_state.set(SyncState::Running(op));
            let branch_for_call = branch_name.clone();
            spawn_local(async move {
                let result = match op {
                    SyncOp::Pull => api::pull(&repo, &branch_for_call).await,
                    SyncOp::Push => api::push(&repo, &branch_for_call).await,
                };
                match result {
                    Ok(response) if response.success => {
                        sync_state.set(SyncState::Done {
                            op,
                            before: response.before.map(Box::new),
                            after: response.after.map(Box::new),
                        });
                        repository.refetch();
                    }
                    Ok(response) => {
                        sync_state.set(SyncState::Failed {
                            op,
                            message: response
                                .error
                                .unwrap_or_else(|| format!("{} failed", op.label())),
                        });
                    }
                    Err(err) => {
                        sync_state.set(SyncState::Failed {
                            op,
                            message: format!("{err}"),
                        });
                    }
                }
            });
        }
    };
    let on_pull = {
        let trigger = trigger_sync.clone();
        move |ev: leptos::ev::MouseEvent| {
            // Don't toggle the parent `<wa-details>` when clicking
            // the icon-button inside the summary slot.
            ev.stop_propagation();
            trigger(SyncOp::Pull);
        }
    };
    let on_push = {
        let trigger = trigger_sync.clone();
        move |ev: leptos::ev::MouseEvent| {
            ev.stop_propagation();
            trigger(SyncOp::Push);
        }
    };

    // Per-branch claim-query state. Two inputs (`the` =
    // attribute, `of` = entity); a `LocalResource` fires when
    // either is non-empty after a submit.
    let the_input = RwSignal::new(String::new());
    let of_input = RwSignal::new(String::new());
    let submitted = RwSignal::new(None::<(Option<String>, Option<String>)>);

    let claims = {
        let branch_name = branch_name.clone();
        LocalResource::new(move || {
            let repo = space_name.get();
            let query = submitted.get();
            let branch = branch_name.clone();
            async move {
                match (repo, query) {
                    (Some(repo), Some((the, of))) => {
                        api::select_claims(&repo, &branch, the.as_deref(), of.as_deref())
                            .await
                            .map(|r| Some(r.claims))
                            .map_err(|e| format!("{e}"))
                    }
                    _ => Ok(None),
                }
            }
        })
    };

    let submit_query = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
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

    let on_the_input = move |ev: leptos::ev::Event| {
        the_input.set(read_wa_input_value(&ev));
    };
    let on_of_input = move |ev: leptos::ev::Event| {
        of_input.set(read_wa_input_value(&ev));
    };

    view! {
        <wa-details class="branch-row" prop:open=is_default>
            <div slot="summary" class="branch-summary">
                // Branch label + revision render as a pair of
                // adjacent `<wa-badge>`s — the WA primitive for
                // "draw attention / display status," which is
                // exactly what these boxes are. Non-interactive
                // by definition (no false click affordance) and
                // sized consistently to their content.
                <span class="branch-summary-pair">
                    <wa-badge variant="neutral" appearance="accent">
                        <wa-icon
                            name="code-branch"
                            variant="solid"
                            slot="start"
                        ></wa-icon>
                        { name }
                    </wa-badge>
                    { match tree_pair.clone() {
                        Some((full, short)) => Either::Left(view! {
                            <wa-badge
                                variant="neutral"
                                appearance="filled"
                                title=full
                            >
                                <wa-icon name="code-commit" slot="start"></wa-icon>
                                { short }
                            </wa-badge>
                        }),
                        None => Either::Right(view! {
                            <wa-badge
                                variant="neutral"
                                appearance="filled"
                            >
                                "no commits"
                            </wa-badge>
                        }),
                    } }
                </span>
                // Sync buttons render unconditionally so the row
                // shape stays uniform across branches. With an
                // upstream they're full accent buttons; without
                // one they downgrade to plain (no chrome) so
                // they read as muted ghosts rather than heavy
                // disabled accent boxes. Plain inline pair
                // instead of `<wa-button-group>` because that
                // component stretches its children to a uniform
                // width (good for text-labeled buttons, wrong
                // for icon-only square sync controls).
                <wa-button
                    variant="neutral"
                    appearance=if has_upstream { "accent" } else { "plain" }
                    size="small"
                    prop:disabled=move ||
                        !has_upstream || matches!(sync_state.get(), SyncState::Running(_))
                    prop:loading=move || matches!(sync_state.get(), SyncState::Running(SyncOp::Pull))
                    on:click=on_pull
                >
                    <wa-icon
                        name="arrow-up-from-bracket"
                        variant="solid"
                        label="Pull"
                        style="transform: rotate(180deg);"
                    ></wa-icon>
                </wa-button>
                <wa-button
                    variant="neutral"
                    appearance=if has_upstream { "accent" } else { "plain" }
                    size="small"
                    prop:disabled=move ||
                        !has_upstream || matches!(sync_state.get(), SyncState::Running(_))
                    prop:loading=move || matches!(sync_state.get(), SyncState::Running(SyncOp::Push))
                    on:click=on_push
                >
                    <wa-icon name="arrow-up-from-bracket" variant="solid" label="Push"></wa-icon>
                </wa-button>
                { move || sync_chip(sync_state.get()) }
            </div>
            <div class="branch-body wa-stack wa-gap-s">
                <dl class="branch-detail">
                    <dt>"upstream"</dt>
                    <dd>{ match upstream_label.clone() {
                        Some(u) => Either::Left(view! { <wa-tag variant="neutral" appearance="filled-outlined">{ u }</wa-tag> }),
                        None => Either::Right(view! { <span>"none"</span> }),
                    } }</dd>
                </dl>
                // YAML query editor — first cut. Lives alongside
                // the existing two-input form rather than replacing
                // it; the form still drives the live query while
                // this surface is the iteration target for a
                // GraphiQL-style editor + result pane.
                <div class="branch-yaml-query wa-stack wa-gap-xs">
                    <label class="hint">"Query (dialog-yaml, draft)"</label>
                    <tonk-code
                        language="dialog-yaml"
                        language-server
                        active-line
                        placeholder="entity:\n  context:\n    field: value"
                    ></tonk-code>
                </div>
                <form class="branch-claims" on:submit=submit_query>
                    <div class="wa-grid wa-gap-s">
                        <wa-input
                            name="claim-the"
                            label="Attribute"
                            placeholder="namespace/name"
                            autocomplete="off"
                            prop:value=move || the_input.get()
                            on:input=on_the_input
                        ></wa-input>
                        <wa-input
                            name="claim-of"
                            label="Entity"
                            placeholder="did:key:… or test:id"
                            autocomplete="off"
                            prop:value=move || of_input.get()
                            on:input=on_of_input
                        ></wa-input>
                    </div>
                    <wa-button
                        type="submit"
                        size="small"
                        variant="neutral"
                        appearance="filled"
                    >"Query"</wa-button>
                </form>
                { move || render_claims(claims.get()) }
            </div>
        </wa-details>
    }
}

/// Compact remote tile: sigil + name. Click affordance is
/// reserved for a future inspect/edit view; for now the card
/// has no interaction so we don't promise something we don't
/// deliver yet.
///
/// `local_subject` is the host repo's DID; remotes whose
/// `subject` is `None` on the wire default to the local one
/// (existing convention from `RemoteConfiguration`).
#[component]
pub(super) fn RemoteCard(
    name: String,
    config: RemoteConfiguration,
    local_subject: String,
) -> impl IntoView {
    let remote_subject = match &config.subject {
        Some(did) => did.to_string(),
        None => local_subject,
    };
    let sigil_value = did_sigil_value(&remote_subject);
    let title_attr = remote_subject;

    view! {
        <wa-card class="remote-tile" appearance="outlined" orientation="horizontal" title=title_attr>
            <tonk-sigil slot="media" value=sigil_value></tonk-sigil>
            <strong class="remote-tile-name">{ name }</strong>
        </wa-card>
    }
}

/// Pull a current value out of a `wa-input` event. The custom
/// element re-fires the platform `input` event whose `target.value`
/// carries the live text, but `target` is typed as
/// [`web_sys::EventTarget`] so we have to walk up to
/// [`web_sys::HtmlElement`] and read the `value` property
/// reflectively. Mirrors the pattern in [`super::create_space`].
fn read_wa_input_value(event: &leptos::ev::Event) -> String {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
        .and_then(|el| {
            js_sys::Reflect::get(&el, &wasm_bindgen::JsValue::from_str("value"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_default()
}

/// Render the claim-query result for a branch's expanded body.
///
/// Pre-submit / unset states render nothing — the form itself
/// is the affordance, no need to nag the user. Post-submit
/// branches render an error callout, an empty-state hint, or
/// the actual list.
fn render_claims(state: Option<Result<Option<Vec<ClaimResponse>>, String>>) -> impl IntoView {
    use leptos::either::EitherOf4;
    match state {
        None | Some(Ok(None)) => EitherOf4::A(()),
        Some(Err(e)) => EitherOf4::B(view! {
            <wa-callout variant="danger">
                <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                { e }
            </wa-callout>
        }),
        Some(Ok(Some(list))) if list.is_empty() => {
            EitherOf4::C(view! { <p class="hint">"No claims matched."</p> })
        }
        Some(Ok(Some(list))) => EitherOf4::D(view! {
            <ul class="claims-list">
                { list.into_iter().map(|c| {
                    let value = serde_json::to_string(&c.is).unwrap_or_default();
                    view! {
                        <li>
                            <code class="the">{ c.the }</code>
                            " of "
                            <code class="of">{ c.of }</code>
                            " is "
                            <code class="is">{ value }</code>
                        </li>
                    }
                }).collect_view() }
            </ul>
        }),
    }
}

/// Render a `<wa-tag>` chip describing the current `SyncState`.
/// Returns nothing for [`SyncState::Idle`] so the summary line
/// collapses to just the name + version + buttons before any
/// sync has run.
fn sync_chip(state: SyncState) -> impl IntoView {
    match state {
        SyncState::Idle => Either::Left(()),
        SyncState::Running(op) => Either::Right(Either::Left(view! {
            <wa-tag variant="neutral" appearance="filled-outlined">
                { format!("{}…", op.label()) }
            </wa-tag>
        })),
        SyncState::Done { op, before, after } => {
            let summary = describe_sync(before.as_deref(), after.as_deref());
            Either::Right(Either::Right(Either::Left(view! {
                <wa-tag variant="success" appearance="filled-outlined">
                    { format!("{}: {summary}", op.label()) }
                </wa-tag>
            })))
        }
        SyncState::Failed { op, message } => {
            let title = message.clone();
            Either::Right(Either::Right(Either::Right(view! {
                <wa-tag variant="danger" appearance="filled-outlined" title=title>
                    { format!("{}: failed", op.label()) }
                </wa-tag>
            })))
        }
    }
}

/// Compact `before → after` summary for a finished sync. Falls
/// back to a single revision label when the operation didn't
/// change the local branch (before == after) or when one side is
/// missing.
fn describe_sync(before: Option<&Revision>, after: Option<&Revision>) -> String {
    fn label(rev: &Revision) -> String {
        format!("{}.{}", rev.period, rev.moment)
    }
    match (before, after) {
        (Some(b), Some(a)) if b.period == a.period && b.moment == a.moment => {
            format!("at {}", label(a))
        }
        (Some(b), Some(a)) => format!("{} → {}", label(b), label(a)),
        (None, Some(a)) => format!("init → {}", label(a)),
        (Some(b), None) => format!("{} → ?", label(b)),
        (None, None) => "no commits".to_string(),
    }
}

/// Github-style short form of a tree reference. `TreeReference`'s
/// `Display` produces `#<base58>`; this drops the `#` marker and
/// truncates the base58 body to 8 chars. Callers should expose
/// the full value via a `title` attribute for hover disclosure.
fn abbreviate_tree(tree: &str) -> String {
    const SHORT_LEN: usize = 8;
    let body = tree.strip_prefix('#').unwrap_or(tree);
    body.chars().take(SHORT_LEN).collect()
}

/// Sigil hex string for a DID, suitable for `<tonk-sigil value=...>`.
/// Same helper used by `super::join` so a space's sigil is
/// consistent across the join page and the space view.
fn did_sigil_value(did: &str) -> Option<String> {
    did::did_key_prefix(did).map(|bytes| {
        let n = u32::from_be_bytes(bytes);
        format!("0x{n:08x}")
    })
}

/// Address-summary helper kept around in case we surface it on
/// a future remote-detail panel. The compact tile renders just
/// sigil + name today, but the full address (and an editable
/// form) will live behind a click — same shape Jack hinted at
/// with the original card layout.
#[allow(dead_code)]
struct RemoteAddressSummary {
    url: String,
    details: Option<String>,
}

#[allow(dead_code)]
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
