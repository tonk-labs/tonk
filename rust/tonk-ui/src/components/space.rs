use dialog_repository::SiteAddress;
use leptos::{either::Either, prelude::*, task::spawn_local, web_sys};
use leptos_router::{
    hooks::use_params,
    location::{BrowserUrl, LocationProvider},
    params::Params,
};
use tonk_worker::{
    BranchConfiguration, EvaluateResponse, RemoteConfiguration, RepositoryInfo, Revision,
};
use wasm_bindgen::JsCast;

use crate::{
    api,
    components::{ActiveSubject, HostId, InviteSpace, Status},
    did,
    error::TonkUiError,
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
                    owner=BranchOwner::Repository(space_name)
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
/// Which side of the routing namespace a [`BranchRow`] belongs
/// to. Drives whether API calls go through the named-repo
/// routes (`/api/repository/{repo}/...`) or the
/// profile-as-repository routes (`/api/profile/...`).
#[derive(Clone, Debug)]
pub(super) enum BranchOwner {
    /// A user-created space, identified by its name. Subscribes
    /// to `space_name.get()` so the row reacts to URL changes.
    Repository(Signal<Option<String>, LocalStorage>),
    /// The profile-as-repository — singleton, no name needed.
    Profile,
}

impl BranchOwner {
    /// Editor `source` URI for this owner's branch. Includes the
    /// owner's identity so the LSP server (when it becomes
    /// branch-aware) can route correctly.
    fn editor_source(&self, branch: &str) -> String {
        match self {
            Self::Repository(name) => format!(
                "tonk-buffer:///{}/{}/scratch",
                name.get_untracked().unwrap_or_default(),
                branch,
            ),
            Self::Profile => format!("tonk-buffer:///<profile>/{branch}/scratch"),
        }
    }
}

#[component]
pub(super) fn BranchRow(
    name: String,
    config: BranchConfiguration,
    /// Identifies which routing namespace this branch belongs
    /// to — see [`BranchOwner`].
    owner: BranchOwner,
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
        let owner = owner.clone();
        move |op: SyncOp| {
            // Sync routes are repository-scoped today — profile
            // branches have no upstream, so the affordance never
            // surfaces for them. Treat this as a no-op rather
            // than wiring profile-side sync routes that don't
            // exist yet.
            let BranchOwner::Repository(space_name) = &owner else {
                return;
            };
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

    // Notebook-style cell list. Each cell is its own
    // <tonk-code> editor + result panel; the active cell is
    // editable, sealed cells are read-only history. When the
    // active cell submits successfully it transitions to sealed
    // and a fresh active cell appends below — Observable-style.
    //
    // Cell IDs are monotonic so the source URIs we hand the LSP
    // are unique and stable for the cell's lifetime, even after
    // reorderings or deletions (neither of which we do today
    // but the IDs survive them for free).
    let cells: RwSignal<Vec<u32>> = RwSignal::new(vec![0]);
    let next_cell_id = RwSignal::new(1_u32);
    let on_cell_sealed = move || {
        let id = next_cell_id.get_untracked();
        next_cell_id.set(id + 1);
        cells.update(|list| list.push(id));
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
                // Notebook of cells. The active cell is the
                // editable one at the bottom; everything above
                // it is sealed history (read-only editor + the
                // result panel its submit produced).
                <div class="branch-cells wa-stack wa-gap-s">
                    <For
                        each=move || cells.get()
                        key=|id| *id
                        children={
                            let owner = owner.clone();
                            let branch_name = branch_name.clone();
                            move |id| {
                                let owner = owner.clone();
                                let branch_name = branch_name.clone();
                                // The newest cell — the one at
                                // the tail of the list — is the
                                // active editor. Everything above
                                // it is sealed.
                                let is_active = Signal::derive(move || {
                                    cells.with(|list| list.last().copied() == Some(id))
                                });
                                // Take focus on mount when:
                                //  - this is a freshly spawned
                                //    cell (id > 0 means the user
                                //    just submitted, focus the
                                //    new editable cell), or
                                //  - this is the very first cell
                                //    (id == 0) and we're inside
                                //    the row that opens by
                                //    default — so on initial
                                //    page load only one row
                                //    grabs focus.
                                let auto_focus = id > 0 || is_default;
                                view! {
                                    <BranchCell
                                        id=id
                                        owner=owner
                                        branch_name=branch_name
                                        is_active=is_active
                                        auto_focus=auto_focus
                                        on_sealed=on_cell_sealed
                                    />
                                }
                            }
                        }
                    />
                </div>
            </div>
        </wa-details>
    }
}

/// One cell in a branch's notebook. Owns its own editor +
/// transaction state. The active cell (newest in the list) is
/// editable; sealed cells are locked read-only history with
/// the result panel their submit produced still mounted below.
///
/// On a successful explicit submit, the cell calls
/// `on_sealed()` so the parent row appends a fresh active cell
/// below — Observable-style.
#[component]
fn BranchCell<F>(
    id: u32,
    owner: BranchOwner,
    branch_name: String,
    is_active: Signal<bool>,
    /// When true, the cell focuses its editor on mount. Used
    /// for the very first cell in the default branch (so the
    /// page lands ready to type) and for cells freshly spawned
    /// after a submit.
    auto_focus: bool,
    on_sealed: F,
) -> impl IntoView
where
    F: Fn() + Clone + 'static,
{
    // Buffer lives in the editor's DOM property — we mirror it
    // on `change` so we can submit it without reaching into the
    // element on every keystroke.
    let transact_buffer = RwSignal::new(String::new());
    let transact_state = RwSignal::new(TransactState::Idle);
    let last_response = RwSignal::new(None::<Box<EvaluateResponse>>);

    // LSP document URI for this specific cell. Including the
    // numeric cell ID keeps every cell distinct on the LSP
    // server even when they all share the same (repo, branch).
    let editor_source = format!("{}-{}", owner.editor_source(&branch_name), id);

    let on_transact_change = {
        let editor_source = editor_source.clone();
        move |ev: leptos::ev::Event| {
            transact_buffer.set(read_tonk_code_value(&ev));
            // Pushed diagnostics — the analyze-error squiggle a
            // failed eval emits — are stale by construction the
            // moment the user touches the buffer they were
            // emitted for. Clear them on every edit so the
            // cell's `errorCount` doesn't carry the previous
            // eval's verdict into the next auto-eval. (LSP-side
            // diagnostics get replaced by the next server frame,
            // which fires the `diagnostics` event.)
            clear_pushed_diagnostics(&editor_source);
        }
    };

    let editor_error_count = RwSignal::new(0_u32);

    // Submit is allowed when:
    //  - the cell is still active (sealed cells are read-only),
    //  - the buffer is non-empty,
    //  - the parser accepts it,
    //  - the LSP isn't showing any error-severity diagnostics, *and*
    //  - the document has at least one assertion (`head!:`).
    // Pure-query documents auto-evaluate on every fresh
    // diagnostics frame, so the play affordance only needs to
    // surface when there's actually something to commit.
    let is_runnable = Signal::derive(move || {
        if !is_active.get() {
            return false;
        }
        let body = transact_buffer.get();
        if body.trim().is_empty() {
            return false;
        }
        if editor_error_count.get() > 0 {
            return false;
        }
        matches!(
            classify_for_dispatch(&body),
            DocDispatch::Submit { has_mutation: true }
        )
    });

    let evaluate_now = {
        let editor_source_for_eval = editor_source.clone();
        let owner = owner.clone();
        let branch_name = branch_name.clone();
        let on_sealed = on_sealed.clone();
        move || {
            // Sealed cells can't submit — even if the keymap
            // somehow fires (shouldn't, since the editor is
            // readonly).
            if !is_active.get_untracked() {
                return;
            }
            let body = transact_buffer.get_untracked();
            if body.trim().is_empty() {
                return;
            }
            let target = match resolve_evaluate_target(&owner, &branch_name) {
                Some(t) => t,
                None => {
                    transact_state.set(TransactState::Failed("no repository in scope".to_owned()));
                    return;
                }
            };
            if matches!(transact_state.get_untracked(), TransactState::Running) {
                return;
            }
            transact_state.set(TransactState::Running);
            let editor_source = editor_source_for_eval.clone();
            let on_sealed = on_sealed.clone();
            spawn_local(async move {
                match classify_for_dispatch(&body) {
                    DocDispatch::ParseError(messages) => {
                        transact_state.set(TransactState::Failed(messages));
                        return;
                    }
                    DocDispatch::Empty => {
                        transact_state.set(TransactState::Idle);
                        return;
                    }
                    DocDispatch::Submit { .. } => {}
                }
                // Explicit submit (play button / Shift+Enter) is
                // a real commit — the user asked for it.
                match target.evaluate(body, "application/yaml", true).await {
                    Ok(response) => {
                        clear_pushed_diagnostics(&editor_source);
                        last_response.set(Some(Box::new(response)));
                        transact_state.set(TransactState::Idle);
                        // Seal this cell and spawn a fresh one
                        // below — the user is moving on.
                        on_sealed();
                    }
                    Err(err) => {
                        if matches!(&err, TonkUiError::Analyze { .. }) {
                            push_analyzer_diagnostic(&editor_source, &err);
                            transact_state.set(TransactState::Idle);
                        } else {
                            transact_state.set(TransactState::Failed(format!("{err}")));
                        }
                    }
                }
            });
        }
    };

    let on_play_click = {
        let evaluate_now = evaluate_now.clone();
        move |ev: leptos::ev::MouseEvent| {
            ev.prevent_default();
            ev.stop_propagation();
            evaluate_now();
        }
    };
    let on_editor_run = {
        let evaluate_now = evaluate_now.clone();
        move |_ev: web_sys::CustomEvent| {
            evaluate_now();
        }
    };

    // Diagnostics arriving from the server are our cue to
    // auto-evaluate (transact=false) — the LSP told us the
    // buffer is clean, so the eval will succeed and surface
    // the would-be result. The handler also keeps
    // `editor_error_count` in sync for the submit button's
    // disabled state. Sealed cells skip the eval — their
    // last_response is frozen — but still update the signal.
    let on_editor_diagnostics = {
        let editor_source_for_eval = editor_source.clone();
        let owner = owner.clone();
        let branch_name = branch_name.clone();
        move |ev: web_sys::CustomEvent| {
            let detail = ev.detail();
            let error_count =
                js_sys::Reflect::get(&detail, &wasm_bindgen::JsValue::from_str("errorCount"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .map(|n| n as u32)
                    .unwrap_or(0);
            editor_error_count.set(error_count);

            if !is_active.get_untracked() {
                return;
            }
            if error_count > 0 {
                return;
            }
            let body = js_sys::Reflect::get(&detail, &wasm_bindgen::JsValue::from_str("value"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            if body.trim().is_empty() {
                return;
            }
            let target = match resolve_evaluate_target(&owner, &branch_name) {
                Some(t) => t,
                None => return,
            };
            if matches!(transact_state.get_untracked(), TransactState::Running) {
                return;
            }
            let editor_source = editor_source_for_eval.clone();
            spawn_local(async move {
                match target.evaluate(body, "application/yaml", false).await {
                    Ok(response) => {
                        clear_pushed_diagnostics(&editor_source);
                        last_response.set(Some(Box::new(response)));
                    }
                    Err(err) => {
                        if matches!(&err, TonkUiError::Analyze { .. }) {
                            push_analyzer_diagnostic(&editor_source, &err);
                        }
                    }
                }
            });
        }
    };

    // First cell on page load shows the placeholder hint;
    // spawned cells (after a submit) start blank — by then the
    // user knows what they're doing and the prompt would just
    // be visual noise.
    let placeholder = if id == 0 {
        "person:\n  this: ?alice\n  name: \"Alice\"\n\n# or assert with `!`:\n# person!: &alice\n#   name: \"Alice\""
    } else {
        ""
    };

    view! {
        <form
            class="branch-yaml-query wa-stack wa-gap-xs"
            class:cell-sealed=move || !is_active.get()
            on:submit=|ev: leptos::ev::SubmitEvent| ev.prevent_default()
        >
            <div class="evaluate-editor">
                <tonk-code
                    language="dialog-yaml"
                    source=editor_source.clone()
                    active-line
                    placeholder=placeholder
                    auto-focus=auto_focus.then_some("")
                    readonly=move || (!is_active.get()).then_some("")
                    on:change=on_transact_change
                    on:run=on_editor_run
                    on:diagnostics=on_editor_diagnostics
                ></tonk-code>
                <wa-button
                    class="evaluate-play"
                    class:is-visible=move || is_runnable.get()
                    type="button"
                    variant="neutral"
                    appearance="filled"
                    size="small"
                    pill
                    title="Submit transaction (Shift+Enter)"
                    prop:loading=move ||
                        matches!(transact_state.get(), TransactState::Running)
                    on:click=on_play_click
                >
                    <wa-icon name="bolt" variant="solid"></wa-icon>
                </wa-button>
            </div>
            { move || render_transact_state(transact_state.get(), last_response.get()) }
        </form>
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

/// State machine for the per-branch transaction editor.
///
/// Mirrors the shape of [`SyncState`]: idle until the user
/// submits, `Running` while the request is in flight, then
/// either a `Done…` variant (the worker accepted the document
/// and either returned matches or committed) or `Failed` (parse
/// error, network error, non-200 from the worker — surfaced as
/// the message verbatim).
///
/// State of the editor's submit cycle. The worker's `/evaluate`
/// route handles any mix of queries and mutations — the editor
/// no longer has to pre-classify the document.
///
/// Successful results don't live here; they live in
/// `last_response` and stay across re-runs so the result panel
/// keeps rendering during a new in-flight request. This state
/// only carries the *transient* lifecycle (idle / running /
/// failed).
#[derive(Clone, Debug, PartialEq)]
enum TransactState {
    Idle,
    Running,
    Failed(String),
}

/// Pre-flight check: parser-only. Catches malformed buffers
/// before they hit the worker. Structural errors the parser is
/// permissive about (e.g. `AssertionWithoutFields`) come back
/// as LSP diagnostics in the editor — the LSP runs the
/// analyzer with a no-op resolver on every change and surfaces
/// the errors as squigglies. The worker's analyzer is the final
/// authority for "this can run" so we don't duplicate that work
/// here.
enum DocDispatch {
    /// Parser accepted the buffer and there's at least one
    /// expression. `has_mutation` is true if any expression is
    /// an assertion (`head!:`) — the play button only surfaces
    /// when there's something to commit; pure-query documents
    /// auto-evaluate on every fresh diagnostics frame and don't
    /// need the affordance.
    Submit { has_mutation: bool },
    /// Empty / whitespace-only document.
    Empty,
    /// Parser raised diagnostics.
    ParseError(String),
}

/// Resolved target for an evaluate call — captures the branch's
/// owner identity at the moment a submit / auto-evaluate fires
/// so the spawned task can dispatch to the right route without
/// re-reading reactive signals from inside `spawn_local`.
enum EvaluateTarget {
    /// User-created repository — `/api/repository/{repo}/...`.
    Repository {
        /// Repository name (cloned from the parent space's signal).
        repo: String,
        /// Branch name within the repository.
        branch: String,
    },
    /// Profile-as-repository — `/api/profile/...`.
    Profile {
        /// Branch name within the profile-as-repository.
        branch: String,
    },
}

impl EvaluateTarget {
    /// POST the body to the appropriate evaluate route. Same
    /// `transact` flag as [`api::evaluate`].
    async fn evaluate(
        self,
        body: String,
        content_type: &str,
        transact: bool,
    ) -> Result<EvaluateResponse, TonkUiError> {
        match self {
            Self::Repository { repo, branch } => {
                api::evaluate(&repo, &branch, body, content_type, transact).await
            }
            Self::Profile { branch } => {
                api::evaluate_profile(&branch, body, content_type, transact).await
            }
        }
    }
}

/// Resolve a [`BranchOwner`] + branch name into an
/// [`EvaluateTarget`]. Returns `None` only when the owner is a
/// repository whose name signal is empty (`space_name` not yet
/// resolved); the profile owner always resolves.
fn resolve_evaluate_target(owner: &BranchOwner, branch: &str) -> Option<EvaluateTarget> {
    match owner {
        BranchOwner::Repository(name) => Some(EvaluateTarget::Repository {
            repo: name.get_untracked()?,
            branch: branch.to_owned(),
        }),
        BranchOwner::Profile => Some(EvaluateTarget::Profile {
            branch: branch.to_owned(),
        }),
    }
}

fn classify_for_dispatch(body: &str) -> DocDispatch {
    let parsed = tonk_notation::parse(body);
    if !parsed.diagnostics.is_empty() {
        let messages = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return DocDispatch::ParseError(messages);
    }
    let Some(syntax) = parsed.syntax else {
        return DocDispatch::Empty;
    };
    if syntax.expressions.is_empty() {
        return DocDispatch::Empty;
    }
    let has_mutation = syntax
        .expressions
        .iter()
        .any(|e| matches!(e, tonk_notation::Expression::Assertion(_)));
    DocDispatch::Submit { has_mutation }
}

/// Read the `value` property off a `<tonk-code>` element from one
/// of its `change` events. The element exposes the buffer through
/// the standard custom-element property (see
/// `rust/tonk-code/src-js/index.ts:794`); we walk through the
/// event target and pull it reflectively, mirroring
/// [`read_wa_input_value`] but for the editor's contract.
fn read_tonk_code_value(event: &leptos::ev::Event) -> String {
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

/// Push an analyzer diagnostic onto the editor whose `source`
/// matches `source` by dispatching a `tonk-push-diagnostics`
/// CustomEvent on the nearest `<tonk-diagnostics-provider>` in
/// the document. The provider routes the diagnostic into that
/// editor's lint state.
///
/// `error` must be a `TonkUiError::Analyze`; other variants are
/// silently dropped (they don't carry the structure needed to
/// produce a positional diagnostic).
fn push_analyzer_diagnostic(source: &str, error: &TonkUiError) {
    let TonkUiError::Analyze {
        code,
        message,
        range,
    } = error
    else {
        return;
    };
    let Some(range) = range else { return };
    let diagnostics = js_sys::Array::new();
    let entry = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &entry,
        &wasm_bindgen::JsValue::from_str("range"),
        &lsp_range_to_js(*range),
    );
    // LSP severity 1 = Error.
    let _ = js_sys::Reflect::set(
        &entry,
        &wasm_bindgen::JsValue::from_str("severity"),
        &wasm_bindgen::JsValue::from_f64(1.0),
    );
    let _ = js_sys::Reflect::set(
        &entry,
        &wasm_bindgen::JsValue::from_str("code"),
        &wasm_bindgen::JsValue::from_str(code),
    );
    let _ = js_sys::Reflect::set(
        &entry,
        &wasm_bindgen::JsValue::from_str("message"),
        &wasm_bindgen::JsValue::from_str(message),
    );
    diagnostics.push(&entry);
    dispatch_push_diagnostics(source, &diagnostics);
}

/// Clear externally-pushed diagnostics on the editor whose
/// `source` matches by dispatching an empty
/// `tonk-push-diagnostics` event. Called after a successful
/// re-submit so a stale squiggle from a previous failure doesn't
/// linger.
fn clear_pushed_diagnostics(source: &str) {
    dispatch_push_diagnostics(source, &js_sys::Array::new());
}

/// Dispatch a `tonk-push-diagnostics` CustomEvent on the
/// `<tonk-diagnostics-provider>` for `source`. The event detail
/// is `{ source, diagnostics }`. Provider routes by `source` so
/// multiple editors under one provider don't collide.
fn dispatch_push_diagnostics(source: &str, diagnostics: &js_sys::Array) {
    let document = match window().document() {
        Some(d) => d,
        None => return,
    };
    let provider = match document
        .query_selector("tonk-diagnostics-provider")
        .ok()
        .flatten()
    {
        Some(el) => el,
        None => return,
    };
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &detail,
        &wasm_bindgen::JsValue::from_str("source"),
        &wasm_bindgen::JsValue::from_str(source),
    );
    let _ = js_sys::Reflect::set(
        &detail,
        &wasm_bindgen::JsValue::from_str("diagnostics"),
        diagnostics,
    );
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    let event = match web_sys::CustomEvent::new_with_event_init_dict("tonk-push-diagnostics", &init)
    {
        Ok(e) => e,
        Err(_) => return,
    };
    let _ = provider.dispatch_event(&event);
}

fn lsp_range_to_js(range: lsp_types::Range) -> wasm_bindgen::JsValue {
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &obj,
        &wasm_bindgen::JsValue::from_str("start"),
        &lsp_position_to_js(range.start),
    );
    let _ = js_sys::Reflect::set(
        &obj,
        &wasm_bindgen::JsValue::from_str("end"),
        &lsp_position_to_js(range.end),
    );
    obj.into()
}

fn lsp_position_to_js(position: lsp_types::Position) -> wasm_bindgen::JsValue {
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &obj,
        &wasm_bindgen::JsValue::from_str("line"),
        &wasm_bindgen::JsValue::from_f64(position.line as f64),
    );
    let _ = js_sys::Reflect::set(
        &obj,
        &wasm_bindgen::JsValue::from_str("character"),
        &wasm_bindgen::JsValue::from_f64(position.character as f64),
    );
    obj.into()
}

/// Render the status surface below the editor.
///
/// `Idle` and `Running` render nothing (the button itself shows
/// the loading spinner via `prop:loading`). The `Done…` variants
/// show kind-specific success callouts; `Failed` shows the
/// worker's error text in a danger callout.
/// Render the area below the editor: the failure callout (when
/// the most recent submit errored) plus the result panel from
/// the most recent successful response. Both regions are always
/// mounted; their inner content swaps. Keeping the wrapper divs
/// in the tree across the Idle → Running → Done cycle prevents
/// the form from shrinking and re-expanding mid-request, which
/// otherwise reads as a "flash" as the page reflows.
fn render_transact_state(
    state: TransactState,
    response: Option<Box<EvaluateResponse>>,
) -> impl IntoView {
    let failure = match state {
        TransactState::Failed(message) => Either::Left(view! {
            <wa-callout variant="danger">
                <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                { message }
            </wa-callout>
        }),
        TransactState::Idle | TransactState::Running => Either::Right(()),
    };
    let result = match response {
        Some(response) => {
            let response = *response;
            Either::Left(render_evaluate_matches(
                response.matches_before,
                response.matches_after,
                response.revision_before,
                response.revision_after,
            ))
        }
        None => Either::Right(()),
    };
    view! {
        <div class="evaluate-result">
            <div class="evaluate-failure">{ failure }</div>
            <div class="evaluate-content">{ result }</div>
        </div>
    }
}

/// Render the evaluate response's match blocks.
///
/// When the commit changed the result set, render a
/// `<wa-comparison>` slider with the pre-commit state on the
/// left (dimmed) and the post-commit state on the right, with
/// each side's branch revision badged in its header. Otherwise
/// just render the blocks once with the after-revision badge.
fn render_evaluate_matches(
    before: Vec<tonk_worker::QueryMatchBlock>,
    after: Vec<tonk_worker::QueryMatchBlock>,
    revision_before: Option<Revision>,
    revision_after: Option<Revision>,
) -> impl IntoView {
    use leptos::either::EitherOf3;
    if after.is_empty() && before.is_empty() {
        return EitherOf3::A(view! {
            <div class="evaluate-revision">{ revision_badge(revision_after.or(revision_before)) }</div>
        });
    }
    if before == after {
        let badge = revision_badge(revision_after.or(revision_before.clone()));
        return EitherOf3::B(view! {
            <div class="evaluate-results wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ badge }</div>
                { render_match_block_list(after) }
            </div>
        });
    }
    EitherOf3::C(view! {
        <wa-comparison position="50" class="evaluate-comparison">
            <div slot="before" class="evaluate-side evaluate-side-before wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ revision_badge(revision_before) }</div>
                { render_match_block_list(before) }
            </div>
            <div slot="after" class="evaluate-side evaluate-side-after wa-stack wa-gap-2xs">
                <div class="evaluate-revision">{ revision_badge(revision_after) }</div>
                { render_match_block_list(after) }
            </div>
        </wa-comparison>
    })
}

/// Render one stack of `<ul>`s for a list of match blocks.
/// Shared between the comparison-slider arms and the no-change
/// fallback.
fn render_match_block_list(blocks: Vec<tonk_worker::QueryMatchBlock>) -> impl IntoView {
    view! {
        <ul class="query-results">
            { blocks.into_iter().map(|block| view! {
                <li>
                    <code class="head-label">{ block.label }</code>
                    <ul class="query-fields">
                        { block.results.into_iter().map(|result| view! {
                            <li>
                                <code class="entity">{ result.this }</code>
                                <ul>
                                    { result.fields.into_iter().map(|(name, value)| view! {
                                        <li>
                                            <code class="field-name">{ name }</code>
                                            ": "
                                            <code class="field-value">{
                                                serde_json::to_string(&value)
                                                    .unwrap_or_else(|_| "<?>".to_string())
                                            }</code>
                                        </li>
                                    }).collect_view() }
                                </ul>
                            </li>
                        }).collect_view() }
                    </ul>
                </li>
            }).collect_view() }
        </ul>
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

/// Render a single revision as the same `<wa-badge>` shape used
/// in the branch-row header (truncated tree hash with the full
/// hash exposed via `title`). `None` produces a "no commits"
/// fallback identical to the branch row's empty state.
fn revision_badge(revision: Option<Revision>) -> impl IntoView {
    match revision {
        Some(rev) => {
            let full = rev.tree.to_string();
            let short = abbreviate_tree(&full);
            Either::Left(view! {
                <wa-badge variant="neutral" appearance="filled" title=full>
                    <wa-icon name="code-commit" slot="start"></wa-icon>
                    { short }
                </wa-badge>
            })
        }
        None => Either::Right(view! {
            <wa-badge variant="neutral" appearance="filled">
                "no commits"
            </wa-badge>
        }),
    }
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

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkSpaceViewerParams {
    space: Option<String>,
    branch: Option<String>,
    entity: Option<String>,
}

/// Entity-rendering view at
/// `/space/{space}/branch/{branch}/view/{entity}`.
///
/// Shares the shell layout with [`TonkSpace`] (banner + share
/// button) but replaces the branches/remotes sections in `main`
/// with a sandboxed guest iframe. The iframe's `src` is the
/// host/guest bridge URL — initial navigation registers the
/// iframe's client id against `{repo, branch}` and serves the
/// entity's HTML body via the same path that backs
/// `GET /api/repository/{repo}/branch/{branch}/content/{entity}`
/// with `Accept: text/html`.
///
/// The iframe is gated on the [`HostId`] context: until the
/// shell's `PUT /api/repository/...` round-trip completes we
/// don't have a hosting Client ID, so the URL would be
/// incomplete. Rendering is held back to a spinner state until
/// then.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkSpaceViewer() -> impl IntoView {
    let params = use_params::<TonkSpaceViewerParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
    });
    let branch_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.branch)
            .filter(|s| !s.is_empty())
    });
    let entity_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.entity)
            .filter(|s| !s.is_empty())
    });

    let status = use_context::<Signal<Status, LocalStorage>>();
    let host_id = use_context::<Signal<Option<HostId>, LocalStorage>>();

    let repository = LocalResource::new(move || {
        let name = space_name.get();
        let ready = status.map(|s| s.get() == Status::Ready).unwrap_or(true);
        async move {
            if !ready {
                return Ok(None);
            }
            match name {
                None => Ok(None),
                Some(name) => api::repository(&name).await,
            }
        }
    });

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
                    Some(info) => Either::Left(render_viewer_view(
                        info,
                        space_name,
                        branch_name,
                        entity_name,
                        host_id,
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

/// Renders the viewer's banner header and an iframe-only
/// `<main>`. Mirrors [`render_space_view`]'s banner so the
/// viewer reads as the same page chrome — just with the
/// branches/remotes sections replaced by the entity render.
fn render_viewer_view<F>(
    info: RepositoryInfo,
    space_name: Signal<Option<String>, LocalStorage>,
    branch_name: Signal<Option<String>, LocalStorage>,
    entity_name: Signal<Option<String>, LocalStorage>,
    host_id: Option<Signal<Option<HostId>, LocalStorage>>,
    on_share: F,
) -> impl IntoView
where
    F: Fn(leptos::ev::MouseEvent) + 'static + Clone,
{
    let local_subject = info.subject.to_string();
    let space_title = info.name.clone();
    let title_attr = local_subject.clone();

    // Iframe `src` is `Some(...)` once we know all four pieces:
    // the hosting document's Client ID (from [`HostId`]) plus
    // the route's space, branch, and entity segments. Until
    // then the iframe is a placeholder — same loading affordance
    // [`TonkSpace`] uses while waiting on the shell.
    let iframe_src = Signal::derive_local(move || {
        let host = host_id.and_then(|s| s.get()).map(|h| h.0)?;
        let space = space_name.get()?;
        let branch = branch_name.get()?;
        let entity = entity_name.get()?;
        Some(format!(
            "/api/repository/{}/branch/{}/host/{}/{}",
            space, branch, host, entity,
        ))
    });

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
        <main class="wa-stack space-view space-viewer">
            { move || match iframe_src.get() {
                Some(src) => Either::Left(view! {
                    // `allow-forms` is granted because `allow-scripts`
                    // already lets in-iframe JS POST anywhere same-origin
                    // via `fetch()`; forms are a strict subset of that
                    // capability. Top navigation, popups, modals, and
                    // downloads stay blocked — those gate behavior JS
                    // alone can't reach.
                    <iframe
                        class="space-viewer-frame"
                        sandbox="allow-scripts allow-same-origin allow-forms"
                        src=src
                    />
                }),
                None => Either::Right(view! {
                    <wa-spinner></wa-spinner>
                }),
            } }
        </main>
    }
}
