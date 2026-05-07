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
    components::{ActiveSubject, CreateSpaceOpen, HostId, InviteSpace, ProfileResource, Status},
    did,
};
use js_sys::encode_uri_component;

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

    // Per-branch transaction state. The buffer lives in the
    // editor's DOM property — we only mirror it on `change` so
    // we can submit it without reaching into the element on every
    // keystroke. `transact_state` drives the status surface below
    // the editor.
    //
    // `last_response` is sticky across submits — it only updates
    // on a successful `DoneEvaluate`. The result panel reads from
    // it and stays mounted with the *previous* response while a
    // new request is in flight, so the form doesn't shrink/grow
    // mid-request and the page doesn't reflow.
    let transact_buffer = RwSignal::new(String::new());
    let transact_state = RwSignal::new(TransactState::Idle);
    let last_response = RwSignal::new(None::<Box<EvaluateResponse>>);

    let on_transact_change = move |ev: leptos::ev::Event| {
        transact_buffer.set(read_tonk_code_value(&ev));
    };

    // Number of *error*-severity diagnostics the editor's
    // LSP/lint plugins currently show. The `<tonk-code>`
    // element fires a `diagnostics` event whenever the count
    // changes; we mirror it into a signal so the play button
    // can hide while errors are outstanding.
    let editor_error_count = RwSignal::new(0_u32);
    let on_diagnostics = move |ev: web_sys::CustomEvent| {
        let count =
            js_sys::Reflect::get(&ev.detail(), &wasm_bindgen::JsValue::from_str("errorCount"))
                .ok()
                .and_then(|v| v.as_f64())
                .map(|n| n as u32)
                .unwrap_or(0);
        editor_error_count.set(count);
    };

    // The play button is shown when:
    //  - the buffer is non-empty,
    //  - the parser accepts it, *and*
    //  - the LSP isn't showing any error-severity diagnostics.
    // Structural errors (`AssertionWithoutFields` etc.) come
    // back as LSP diagnostics; we use that count as the source
    // of truth for "this won't run."
    let is_runnable = Signal::derive(move || {
        let body = transact_buffer.get();
        if body.trim().is_empty() {
            return false;
        }
        if editor_error_count.get() > 0 {
            return false;
        }
        matches!(classify_for_dispatch(&body), DocDispatch::Submit)
    });

    // The actual evaluate call, fired from both the floating
    // play button and Shift+Enter on the editor. Defined as a
    // helper that takes the captured branch name and fires;
    // each adapter clones the name before delegating.
    let evaluate_now = {
        let branch_template = branch_name.clone();
        move |branch_name: String| {
            let body = transact_buffer.get_untracked();
            if body.trim().is_empty() {
                return;
            }
            let Some(repo) = space_name.get_untracked() else {
                transact_state.set(TransactState::Failed("no repository in scope".to_owned()));
                return;
            };
            if matches!(transact_state.get_untracked(), TransactState::Running) {
                return;
            }
            transact_state.set(TransactState::Running);
            let _ = &branch_template; // silence unused-capture warning
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
                    DocDispatch::Submit => {}
                }
                match api::evaluate(&repo, &branch_name, body, "application/yaml").await {
                    Ok(response) => {
                        last_response.set(Some(Box::new(response)));
                        transact_state.set(TransactState::Idle);
                    }
                    Err(err) => {
                        transact_state.set(TransactState::Failed(format!("{err}")));
                    }
                }
            });
        }
    };

    let on_play_click = {
        let branch_name = branch_name.clone();
        let evaluate_now = evaluate_now.clone();
        move |ev: leptos::ev::MouseEvent| {
            // Belt-and-braces: prevent any default action and
            // stop the click from bubbling out of the editor's
            // form. `<wa-button>` defaults to `type="button"`
            // (opposite of native), but we pin both sides so a
            // future change to the button's defaults can't
            // sneak a form submit back in (which scrolls the
            // page to the top via the browser's GET-current-url
            // default).
            ev.prevent_default();
            ev.stop_propagation();
            evaluate_now(branch_name.clone())
        }
    };
    // `<tonk-code>` fires a `run` CustomEvent on
    // Shift+Enter / Mod+Enter (consuming the key in the
    // editor's keymap so no line break is inserted). We
    // listen for it and forward to the same evaluate path as
    // the play button. The event name is `run` rather than
    // `submit` to avoid colliding with the form's native
    // `submit` event.
    let on_editor_run = {
        let branch_name = branch_name.clone();
        move |_ev: web_sys::CustomEvent| {
            evaluate_now(branch_name.clone());
        }
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
                // Asserted-notation editor. The submit handler
                // pre-classifies the document (query vs assertion)
                // and posts to `/query` or `/transact` accordingly.
                <form
                    class="branch-yaml-query wa-stack wa-gap-xs"
                    // Form submission on click anywhere in the
                    // form (e.g., the floating play button) was
                    // navigating to the page top because the
                    // browser default is GET on the current URL.
                    // Swallow it; the play button's own click
                    // handler runs the evaluator.
                    on:submit=|ev: leptos::ev::SubmitEvent| ev.prevent_default()
                >
                    <label class="hint">"Asserted-notation (query or transaction)"</label>
                    <div class="evaluate-editor">
                        <tonk-code
                            language="dialog-yaml"
                            language-server
                            active-line
                            placeholder="person ?alice:\n  name: \"Alice\"\n\n# or assert with `!`:\n# person!:\n#   name: \"Alice\""
                            on:change=on_transact_change
                            on:run=on_editor_run
                            on:diagnostics=on_diagnostics
                        ></tonk-code>
                        // Floating play button under the
                        // editor — Observable-style. Visible
                        // only when the buffer is runnable
                        // (parses cleanly + non-empty); click
                        // or Shift+Enter triggers evaluation.
                        // The `type="button"` prevents the
                        // browser from treating the click as a
                        // form-submit (which would scroll the
                        // page to the top).
                        // No `prop:disabled` here on purpose:
                        // disabling a *focused* button forces
                        // browsers to drop focus to `<body>`,
                        // which scrolls the page to the top.
                        // The `loading` state gives visual
                        // feedback while in flight, and
                        // `evaluate_now`'s own re-entry guard
                        // (the `Running` early return) prevents
                        // double-submit if the user clicks
                        // again.
                        <wa-button
                            class="evaluate-play"
                            class:is-visible=move || is_runnable.get()
                            type="button"
                            variant="neutral"
                            appearance="filled"
                            size="small"
                            pill
                            title="Run (Shift+Enter)"
                            prop:loading=move ||
                                matches!(transact_state.get(), TransactState::Running)
                            on:click=on_play_click
                        >
                            <wa-icon name="play" variant="solid"></wa-icon>
                        </wa-button>
                    </div>
                    { move || render_transact_state(transact_state.get(), last_response.get()) }
                </form>
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
    /// expression. The play button shows; on click, the worker
    /// gets the real say.
    Submit,
    /// Empty / whitespace-only document.
    Empty,
    /// Parser raised diagnostics.
    ParseError(String),
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
        DocDispatch::Empty
    } else {
        DocDispatch::Submit
    }
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
                    <iframe
                        class="space-viewer-frame"
                        sandbox="allow-scripts allow-same-origin"
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

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkPortalsParams {
    space: Option<String>,
}

/// Portals view at `/portals/{space}`.
///
/// Portals are a UI *layer* over a repo, not a piece of data
/// inside one — that's why the URL doesn't share the
/// `/space/{name}/branch/...` namespace, which is reserved for
/// addressing data within a repo. Individual artifact tiles
/// rendered inside the portal *do* link back to data URLs of
/// that shape (`/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`),
/// so each tile picks its own branch as needed.
///
/// Reuses [`TonkSpace`]'s shell — banner with the share button —
/// and replaces `<main>` with the `<tonk-portals>` web component.
/// The custom element is implemented in TS/React (see
/// `rust/tonk-portals/src-js/`), which lets us host a grid of
/// iframe panels without rewriting the React `grid-select`
/// prototype in Leptos.
///
/// `repo` and `host` are passed in as attributes; the React app
/// reads them via `attributeChangedCallback` so a host id
/// arriving late (after the `PUT /api/repository/...` round-trip
/// resolves) flows in without a remount.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkPortals() -> impl IntoView {
    let params = use_params::<TonkPortalsParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
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
                    Some(info) => Either::Left(render_portals_view(
                        info,
                        space_name,
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

/// Renders the portals view as a single full-bleed `<main>` with
/// two floating "notch" controls — Figma-style — instead of a
/// horizontal banner. The grid claims every vertical pixel; the
/// chrome (space switcher, profile, share) lives on top of it
/// in the corners.
///
/// Top-left notch: a `<details>` dropdown showing the current
/// space name. Click to reveal a list of spaces from the shared
/// [`ProfileResource`]; each entry is a link to
/// `/portals/<name>`.
///
/// Top-right notch: profile sigil (linking to `/profile`) and the
/// share button. Both float over the grid surface.
///
/// The `<tonk-portals>` element is gated on a non-empty host id —
/// the React app would render a "waiting…" stub otherwise, but
/// gating in Leptos avoids registering an empty guest binding in
/// the service worker.
fn render_portals_view<F>(
    info: RepositoryInfo,
    space_name: Signal<Option<String>, LocalStorage>,
    host_id: Option<Signal<Option<HostId>, LocalStorage>>,
    on_share: F,
) -> impl IntoView
where
    F: Fn(leptos::ev::MouseEvent) + 'static + Clone,
{
    let space_title = info.name.clone();

    let repo_attr = Signal::derive_local(move || space_name.get().unwrap_or_default());
    let host_attr = Signal::derive_local(move || {
        host_id
            .and_then(|s| s.get())
            .map(|h| h.0)
            .unwrap_or_default()
    });
    let ready = Signal::derive_local(move || {
        !repo_attr.get().is_empty() && !host_attr.get().is_empty()
    });

    // Profile data drives both the space-switcher dropdown
    // (top-left notch) and the profile sigil (top-right notch).
    // Provided by `TonkShell`; refetched on `/api/profile`
    // broadcasts so a newly-created space appears without a
    // reload.
    let profile_resource =
        use_context::<ProfileResource>().expect("ProfileResource provided by TonkShell");
    let create_space_open =
        use_context::<CreateSpaceOpen>().expect("CreateSpaceOpen provided by TonkShell");
    let open_create_space = move |_| create_space_open.set(true);

    let profile_sigil = Signal::derive_local(move || {
        let info = profile_resource.get().and_then(|r| r.ok()).flatten()?;
        did::did_key_prefix(info.profile.subject.as_ref()).map(|bytes| {
            let n = u32::from_be_bytes(bytes);
            format!("0x{n:08x}")
        })
    });

    // Sorted list of (name, did:key:...) pairs for the
    // dropdown. Same shape and sort order as the sidebar
    // toolbar so the two stay in sync. Entries become
    // `<a href="/portals/<encoded-name>">` links.
    let space_entries = Signal::derive_local(move || {
        let info = profile_resource.get().and_then(|r| r.ok()).flatten()?;
        let mut spaces: Vec<(String, String)> = info
            .space
            .into_iter()
            .map(|(name, did)| (name, did.to_string()))
            .collect();
        spaces.sort_by(|a, b| a.0.cmp(&b.0));
        Some(spaces)
    });

    let space_title_for_compare = space_title.clone();

    view! {
        <main class="space-portals">
            // Top-left drawer: a 40px chevron button parked in the
            // rail's corner. Click toggles a panel containing the
            // profile link and the space switcher list — both nav
            // items the previous design split between two notches.
            // The chevron sits inside a `__handle-inner` span so the
            // native `<summary>` marker can be hidden cleanly:
            // browsers render the disclosure triangle as a flex
            // child of the summary unless the summary is `display:
            // block`, so we put the flex centering on the inner span.
            <details class="portals-drawer">
                <summary class="portals-drawer__handle" aria-label="Open menu">
                    <span class="portals-drawer__handle-inner">
                        <wa-icon
                            class="portals-drawer__caret"
                            name="chevron-right"
                        ></wa-icon>
                    </span>
                </summary>
                <div class="portals-drawer__panel">
                    <a
                        class="portals-drawer__profile"
                        href="/profile"
                        aria-label="Profile"
                    >
                        <tonk-sigil
                            class="portals-drawer__profile-sigil"
                            value=move || profile_sigil.get()
                        ></tonk-sigil>
                        <span class="portals-drawer__profile-label">"Profile"</span>
                    </a>
                    <div class="portals-drawer__divider" role="separator"></div>
                    <div class="portals-drawer__spaces">
                        { move || space_entries.get().map(|spaces| {
                            spaces
                                .into_iter()
                                .map(|(name, did)| {
                                    let encoded = encode_uri_component(&name)
                                        .as_string()
                                        .unwrap_or_else(|| name.clone());
                                    let href = format!("/portals/{encoded}");
                                    let is_current = name == space_title_for_compare;
                                    let n = u32::from_be_bytes(
                                        did::did_key_prefix(&did).unwrap_or_default(),
                                    );
                                    let sigil = format!("0x{n:08x}");
                                    let label_aria = format!("Open {name}");
                                    let item_class = if is_current {
                                        "portals-switcher__item portals-switcher__item--current"
                                    } else {
                                        "portals-switcher__item"
                                    };
                                    view! {
                                        <a
                                            class=item_class
                                            href=href
                                            aria-label=label_aria
                                        >
                                            <tonk-sigil
                                                class="portals-switcher__sigil"
                                                value=sigil
                                            ></tonk-sigil>
                                            <span class="portals-switcher__item-name">{ name }</span>
                                        </a>
                                    }
                                })
                                .collect_view()
                        }) }
                        <button
                            type="button"
                            class="portals-switcher__item portals-switcher__item--create"
                            on:click=open_create_space
                            aria-label="Create new space"
                        >
                            <span class="portals-switcher__plus" aria-hidden="true">"+"</span>
                            <span class="portals-switcher__item-name">"New space"</span>
                        </button>
                    </div>
                </div>
            </details>

            // Top-right corner: bare share icon button, also sized
            // to fit inside the 40px rail strip. Plain `<button>`
            // (not wa-button) so it has no chrome by default —
            // matches the drawer chevron's "icon only" feel.
            <button
                type="button"
                class="portals-corner-share"
                on:click=on_share
                aria-label="Invite someone to this space"
            >
                <wa-icon name="share-nodes" variant="solid"></wa-icon>
            </button>

            { move || if ready.get() {
                Either::Left(view! {
                    <tonk-portals
                        repo=move || repo_attr.get()
                        host=move || host_attr.get()
                    ></tonk-portals>
                })
            } else {
                Either::Right(view! {
                    <wa-spinner></wa-spinner>
                })
            } }
        </main>
    }
}
