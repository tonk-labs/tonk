//! `<tonk-inspector>` — a notebook-style scratch editor.
//!
//! Observable-style cell list: the active cell at the bottom is an
//! editable `<tonk-code>` editor; everything above it is sealed,
//! read-only history with the result its submit produced still
//! mounted below. A clean buffer auto-evaluates (dry-run) so a
//! pure query previews its matches as you type; an explicit submit
//! commits, seals the cell, and spawns a fresh one.
//!
//! The inspector is a plain consumer: it owns no branch, no remote,
//! no sync, and never fetches. It dispatches `tonk-evaluate` on its
//! editor element and the surrounding `<tonk-host>` / routing
//! context (the `<tonk-repository>` / `<tonk-branch>` ancestors a
//! display route mounts) annotates space + branch and performs the
//! IO — exactly as `<tonk-display>` consumes the host. Dropped into
//! a board tile via `<tonk-display concept=inspector>`, it evaluates
//! against whatever space the route is showing.

// The inspector is a DOM custom element that consumes the host via
// `tonk_host::consumer` — both wasm-only. The whole module is gated
// to wasm32 (mirroring `tonk-display`'s element module); native
// builds (test/SSR) get the no-op `register` stub at the bottom so
// the library still compiles off-target.
#[cfg(target_arch = "wasm32")]
use std::any::Any;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
use custom_elements::CustomElement;
#[cfg(target_arch = "wasm32")]
use leptos::{prelude::*, task::spawn_local, web_sys};
#[cfg(target_arch = "wasm32")]
use tonk_host::consumer as host_consumer;
#[cfg(target_arch = "wasm32")]
use tonk_worker::EvaluateResponse;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{Element, HtmlElement, window};

#[cfg(target_arch = "wasm32")]
use super::space::{
    DocDispatch, TransactState, classify_for_dispatch, clear_pushed_diagnostics,
    read_tonk_code_value, render_transact_state,
};

/// Per-instance state for one `<tonk-inspector>` element.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct TonkInspectorElement {
    /// Boxed [`leptos::mount::UnmountHandle`]. Type-erased because
    /// the concrete `N::State` is the giant view's mountable type
    /// and we don't want to name it. Dropping the box unmounts
    /// the sub-tree.
    mount: RefCell<Option<Box<dyn Any>>>,
}

#[cfg(target_arch = "wasm32")]
impl CustomElement for TonkInspectorElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let handle = leptos::mount::mount_to(this.clone(), move || {
            view! { <TonkInspector /> }
        });
        *self.mount.borrow_mut() = Some(Box::new(handle));
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop the mount handle so Leptos can tear down the
        // sub-tree's reactive graph cleanly.
        self.mount.borrow_mut().take();
    }
}

/// The notebook. A growing list of cells; the tail is the active
/// editor, every earlier entry is sealed history. Cell IDs are
/// monotonic so each cell's LSP `source` URI stays unique and
/// stable for its lifetime.
#[cfg(target_arch = "wasm32")]
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkInspector() -> impl IntoView {
    let cells: RwSignal<Vec<u32>> = RwSignal::new(vec![0]);
    let next_cell_id = RwSignal::new(1_u32);
    let on_cell_sealed = move || {
        let id = next_cell_id.get_untracked();
        next_cell_id.set(id + 1);
        cells.update(|list| list.push(id));
    };

    // The `(repo, branch)` the cells' LSP buffer URIs are built from.
    // We read it from the same `<tonk-repository>` / `<tonk-branch>`
    // routing ancestors the host annotator reads for `tonk-evaluate`,
    // so the language server can open the branch and source
    // branch-aware completions / hovers. The inspector holds no branch
    // state of its own — this is purely the ambient context, resolved
    // once the element is in the DOM. Cells render only after it
    // resolves so each `<tonk-code>` opens its LSP document under the
    // right `tonk-buffer:///<repo>/<branch>/...` URI the first time.
    let context = RwSignal::<Option<(String, String)>, LocalStorage>::new_local(None);
    let root: NodeRef<leptos::html::Div> = NodeRef::new();
    Effect::new(move |_| {
        let Some(el) = root.get() else {
            return;
        };
        context.set(resolve_context(&el));
    });

    view! {
        <div class="tonk-inspector branch-cells wa-stack wa-gap-s" node_ref=root>
            { move || context.get().map(|(repo, branch)| view! {
                <For
                    each=move || cells.get()
                    key=|id| *id
                    children={
                        let repo = repo.clone();
                        let branch = branch.clone();
                        move |id| {
                            // The newest cell — the tail of the list — is
                            // the active editor. Everything above it is
                            // sealed.
                            let is_active = Signal::derive(move || {
                                cells.with(|list| list.last().copied() == Some(id))
                            });
                            // Focus on mount: freshly spawned cells (the
                            // user just submitted) and the very first cell,
                            // so the inspector lands ready to type.
                            let auto_focus = true;
                            view! {
                                <InspectorCell
                                    id=id
                                    repo=repo.clone()
                                    branch=branch.clone()
                                    is_active=is_active
                                    auto_focus=auto_focus
                                    on_sealed=on_cell_sealed
                                />
                            }
                        }
                    }
                />
            }) }
        </div>
    }
}

/// Walk up from `el` to the nearest `<tonk-repository>` and
/// `<tonk-branch>` ancestors and read their `name` attributes — the
/// same routing context the host's annotators stamp onto
/// `tonk-evaluate`. Returns `None` until both are found (the routing
/// chain a display route mounts always provides both); the cells hold
/// off rendering until then so they never open an LSP document under a
/// branchless URI the server can't resolve.
#[cfg(target_arch = "wasm32")]
fn resolve_context(el: &Element) -> Option<(String, String)> {
    let mut repo: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut node = el.parent_element();
    while let Some(current) = node {
        let tag = current.tag_name().to_ascii_lowercase();
        if branch.is_none() && tag == "tonk-branch" {
            branch = current.get_attribute("name").filter(|s| !s.is_empty());
        } else if repo.is_none() && tag == "tonk-repository" {
            repo = current.get_attribute("name").filter(|s| !s.is_empty());
        }
        if repo.is_some() && branch.is_some() {
            break;
        }
        node = current.parent_element();
    }
    Some((repo?, branch?))
}

/// One notebook cell. Owns its editor + transaction state. The
/// active cell (newest in the list) is editable; sealed cells are
/// locked read-only history with the result panel their submit
/// produced still mounted below. On a successful explicit submit
/// the cell calls `on_sealed()` so the notebook appends a fresh
/// active cell below.
#[cfg(target_arch = "wasm32")]
#[component]
fn InspectorCell<F>(
    id: u32,
    /// Repository the cell's LSP buffer URI is scoped to — the
    /// nearest `<tonk-repository>` the notebook resolved.
    repo: String,
    /// Branch the cell's LSP buffer URI is scoped to — the nearest
    /// `<tonk-branch>` the notebook resolved.
    branch: String,
    is_active: Signal<bool>,
    auto_focus: bool,
    on_sealed: F,
) -> impl IntoView
where
    F: Fn() + Clone + 'static,
{
    // Buffer mirrors the editor's DOM `value` on `change` so we can
    // submit it without reaching into the element on every keystroke.
    let transact_buffer = RwSignal::new(String::new());
    let transact_state = RwSignal::new(TransactState::Idle);
    let last_response = RwSignal::new(None::<Box<EvaluateResponse>>);
    let editor_error_count = RwSignal::new(0_u32);

    // The editor element, captured from the events it fires so
    // evaluate dispatches bubble up through the routing context that
    // annotates space + branch. Populated by the editor's first
    // `change` / `diagnostics` frame, both of which precede any state
    // in which the play button or an auto-eval could fire.
    let editor_el = RwSignal::<Option<Element>, LocalStorage>::new_local(None);
    let capture_editor = move |target: Option<web_sys::EventTarget>| {
        if editor_el.with_untracked(Option::is_some) {
            return;
        }
        if let Some(el) = target.and_then(|t| t.dyn_into::<Element>().ok()) {
            editor_el.set(Some(el));
        }
    };

    // LSP document URI for this cell. The numeric id keeps every
    // cell distinct on the LSP server even though they share a host.
    // The `tonk-buffer:///<repo>/<branch>/...` shape is what the
    // language server parses back into `(repo, branch)` to open the
    // live branch (see `tonk_language_server::server::parse_repo_branch`),
    // so completions and hovers resolve against branch-published
    // concepts — not just the cell's own declarations.
    let editor_source = format!("tonk-buffer:///{repo}/{branch}/scratch-{id}");

    let on_transact_change = {
        let editor_source = editor_source.clone();
        move |ev: leptos::ev::Event| {
            capture_editor(ev.target());
            transact_buffer.set(read_tonk_code_value(&ev));
            // A failed eval's analyzer squiggle is stale the moment
            // the buffer it was emitted for changes — clear it so the
            // cell's `errorCount` doesn't carry the old verdict into
            // the next auto-eval.
            clear_pushed_diagnostics(&editor_source);
        }
    };

    // Submit is allowed when the cell is active, the buffer is
    // non-empty, the LSP shows no error-severity diagnostics, and the
    // document has at least one assertion to commit. Pure-query
    // documents auto-evaluate on every clean diagnostics frame, so
    // the play affordance only surfaces when there's a mutation.
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

    // Dispatch a `tonk-evaluate` on the editor element. `transact`
    // distinguishes a committing submit from a dry-run preview. The
    // host annotates context and performs the IO; we deserialize its
    // parsed-JSON response back into the typed shape.
    let evaluate = move |body: String, transact: bool| async move {
        let Some(consumer) = editor_el.get_untracked() else {
            return Err("inspector editor not mounted".to_owned());
        };
        match host_consumer::evaluate(&consumer, &body, transact).await {
            Ok(value) => parse_evaluate_response(&value),
            Err(detail) => Err(detail.message),
        }
    };

    let evaluate_now = {
        let on_sealed = on_sealed.clone();
        move || {
            // Sealed cells can't submit.
            if !is_active.get_untracked() {
                return;
            }
            let body = transact_buffer.get_untracked();
            if body.trim().is_empty() {
                return;
            }
            if matches!(transact_state.get_untracked(), TransactState::Running) {
                return;
            }
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
            transact_state.set(TransactState::Running);
            let on_sealed = on_sealed.clone();
            spawn_local(async move {
                // Explicit submit is a real commit — the user asked.
                match evaluate(body, true).await {
                    Ok(response) => {
                        last_response.set(Some(Box::new(response)));
                        transact_state.set(TransactState::Idle);
                        // Nudge whatever sync controller is mounted (the
                        // display route runs one) so this commit reaches
                        // the remote without a manual push. The inspector
                        // owns no sync of its own — this is a decoupled
                        // window-event signal, debounced controller-side.
                        crate::sync_controller::notify_committed();
                        // Seal this cell and spawn a fresh one below —
                        // the user is moving on.
                        on_sealed();
                    }
                    Err(message) => {
                        transact_state.set(TransactState::Failed(message));
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
        move |ev: web_sys::CustomEvent| {
            capture_editor(ev.target());
            evaluate_now();
        }
    };

    // A clean diagnostics frame from the editor is our cue to
    // auto-evaluate as a dry run — the LSP says the buffer parses, so
    // the eval will surface the would-be result without committing.
    // The handler also keeps `editor_error_count` live for the submit
    // button's disabled state. Sealed cells freeze their result but
    // still track the count.
    let on_editor_diagnostics = move |ev: web_sys::CustomEvent| {
        capture_editor(ev.target());
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
        if matches!(transact_state.get_untracked(), TransactState::Running) {
            return;
        }
        spawn_local(async move {
            if let Ok(response) = evaluate(body, false).await {
                last_response.set(Some(Box::new(response)));
            }
        });
    };

    // The first cell shows a placeholder hint; spawned cells start
    // blank — by then the user knows what they're doing.
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
            { move || render_transact_state(
                transact_state.get(),
                last_response.get(),
            ) }
        </form>
    }
}

/// Deserialize the host's parsed-JSON evaluate result (a `JsValue`
/// from `JSON.parse`) back into the typed [`EvaluateResponse`].
///
/// We round-trip through `JSON.stringify` + `serde_json` rather
/// than `serde_wasm_bindgen` so the `serde_json::Value` field maps
/// and `Revision` decode with the exact same semantics as the
/// worker's own `Deserialize`.
#[cfg(target_arch = "wasm32")]
fn parse_evaluate_response(value: &wasm_bindgen::JsValue) -> Result<EvaluateResponse, String> {
    let text = js_sys::JSON::stringify(value)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| "evaluate response was not serializable JSON".to_owned())?;
    serde_json::from_str(&text).map_err(|e| format!("evaluate response decode: {e}"))
}

/// Register `<tonk-inspector>`. Idempotent — calling more than
/// once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    if already_registered() {
        return;
    }
    TonkInspectorElement::define("tonk-inspector");
}

#[cfg(target_arch = "wasm32")]
fn already_registered() -> bool {
    let Some(win) = window() else { return false };
    !win.custom_elements().get("tonk-inspector").is_undefined()
}

/// Native (non-wasm) builds have no DOM to register against. The
/// element only exists in the browser; off-target the library still
/// needs a `register` symbol for the components module to re-export.
#[cfg(not(target_arch = "wasm32"))]
pub fn register() {}
