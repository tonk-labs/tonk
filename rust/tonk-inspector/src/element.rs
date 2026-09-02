//! `<tonk-inspector>` — a notebook-style scratch editor, plain-DOM.
//!
//! A leptos-free port of the former `tonk-ui` Leptos component, built as a
//! `CustomElement` so it registers in the sealed guest. The notebook is a stack
//! of cells: the tail cell is an editable `<tonk-code>` editor; sealed cells
//! above are read-only history with their result still mounted. A clean
//! diagnostics frame auto-evaluates (dry-run preview); an explicit submit commits
//! (real `transact`), seals the cell, and spawns a fresh one.
//!
//! Data path: the inspector resolves `(repo, branch)` from its
//! its own `with="branch@repo"` (forwarded by the mounting `<tonk-display>`)
//! and POSTs to `/api/repository/{repo}/branch/{branch}/evaluate?transact=…`.
//! In the sealed guest `window.fetch` is the portal proxy, so the request rides
//! the bridge to the host's real origin transparently — no `<tonk-host>` consumer
//! event, no engine linkage.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{CustomEvent, Element, Event, HtmlElement, Response, window};

use crate::debug::{self, Probe};
use crate::render::render_result;
use crate::response::EvaluateResponse;

/// A bag of installed listener closures, kept alive until the element
/// disconnects. Shared by `Rc` so cells spawned later append to the same bag.
type Closures = Rc<RefCell<Vec<Closure<dyn FnMut(Event)>>>>;

/// The custom element.
#[derive(Default)]
pub struct TonkInspectorElement {
    closures: Closures,
}

impl CustomElement for TonkInspectorElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Some(context) = resolve_context(this) else {
            this.set_inner_html(
                "<div class=\"tonk-inspector\">\
                   <section class=\"error\">no repository in context \
                   (nest under a with=&quot;branch@repo&quot; element)</section>\
                 </div>",
            );
            return;
        };

        this.set_class_name("tonk-inspector wa-stack wa-gap-m");
        this.set_inner_html("");

        // Mount a `<tonk-diagnostics-provider>` as the cell host. Each cell's
        // `<tonk-code>` dispatches a `tonk-code-connect` event on connect that
        // must bubble to an ancestor provider; the provider owns the LSP client
        // (`/api/language-server`, routed over the guest fetch bridge) and feeds
        // diagnostics back. The host page mounts one app-wide, but the sealed
        // guest has none — so the inspector provides its own.
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let host: HtmlElement = match document
            .create_element("tonk-diagnostics-provider")
            .ok()
            .and_then(|e| e.dyn_into::<HtmlElement>().ok())
        {
            Some(provider) => provider,
            None => return,
        };
        host.set_class_name("branch-cells wa-stack wa-gap-s");
        let _ = this.append_child(&host);

        let debug_panel = if context.profile {
            None
        } else {
            DebugPanel::mount(
                context.repo.clone(),
                context.branch.clone(),
                self.closures.clone(),
            )
        };

        // Diagnostics belong before the scratch notebook in reading order.
        if let Some(panel) = debug_panel.as_ref() {
            let _ = this.insert_before(&panel.host, Some(host.as_ref()));
        }

        let notebook = Rc::new(Notebook {
            host,
            repo: context.repo,
            branch: context.branch,
            debug_panel,
            next_id: Cell::new(0),
            closures: self.closures.clone(),
        });

        // Spawn the first cell only AFTER `<tonk-code>` is defined. Its bundle is
        // imported asynchronously by the guest (after the inspector registers), so
        // mounting a cell now would append an un-upgraded `<tonk-code>` and, worse,
        // race the `<tonk-diagnostics-provider>` (same bundle): the editor's
        // initial `tonk-code-connect` could fire before the provider's listener
        // exists and be lost — no LSP, no diagnostics, no auto-eval. Waiting until
        // the element is defined guarantees the provider (appended above, upgraded
        // by the same `define` pass) is listening when the editor connects.
        let registry = window().map(|w| w.custom_elements());
        match registry.and_then(|r| r.when_defined("tonk-code").ok()) {
            Some(defined) => {
                spawn_local(async move {
                    let _ = JsFuture::from(defined).await;
                    notebook.spawn_cell(true);
                });
            }
            // No registry / bad name (shouldn't happen in a browser) — mount now.
            None => notebook.spawn_cell(true),
        }
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.closures.borrow_mut().clear();
    }
}

#[derive(Default)]
enum ProbeState {
    #[default]
    Idle,
    Loading,
    Response(String),
    Failure(String),
}

/// Read-only repository metadata mounted above a named-space notebook.
struct DebugPanel {
    host: HtmlElement,
    repo: String,
    branch: String,
    repository: RefCell<Option<String>>,
    probe: RefCell<ProbeState>,
    refreshing: Cell<bool>,
    probing: Cell<bool>,
}

impl DebugPanel {
    fn mount(repo: String, branch: String, closures: Closures) -> Option<Rc<Self>> {
        let document = window()?.document()?;
        let host = document
            .create_element("section")
            .ok()?
            .dyn_into::<HtmlElement>()
            .ok()?;
        host.set_class_name("inspector-debug");
        host.set_attribute("aria-label", "branch diagnostics").ok();
        host.set_inner_html(&debug::render_loading(&repo, &branch));

        let panel = Rc::new(Self {
            host,
            repo,
            branch,
            repository: RefCell::new(None),
            probe: RefCell::new(ProbeState::Idle),
            refreshing: Cell::new(false),
            probing: Cell::new(false),
        });

        add(panel.host.as_ref(), "click", &closures, {
            let panel = panel.clone();
            move |event| panel.clone().handle_click(event)
        });

        // The panel is detached until the caller inserts it before the
        // notebook, but its fetch may start now: the portal's window.fetch
        // relay does not depend on DOM event bubbling.
        panel.clone().refresh_local();
        Some(panel)
    }

    fn handle_click(self: Rc<Self>, event: Event) {
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        let Some(button) = target.closest("button").ok().flatten() else {
            return;
        };
        if !self.host.contains(Some(button.as_ref())) {
            return;
        }

        match button.get_attribute("data-debug-action").as_deref() {
            Some("refresh") => self.refresh_local(),
            Some("probe") => self.probe_remote(),
            _ => {
                if let Some(value) = button.get_attribute("data-copy-value") {
                    self.copy(button, value);
                }
            }
        }
    }

    fn refresh_local(self: Rc<Self>) {
        if self.refreshing.get() || self.probing.get() {
            return;
        }
        self.refreshing.set(true);
        self.sync_busy_controls();
        let panel = self.clone();
        let path = format!("/api/repository/{}", self.repo);
        spawn_local(async move {
            match fetch_text(&path).await {
                Ok(body) => {
                    panel.repository.replace(Some(body));
                    panel.probe.replace(ProbeState::Idle);
                    panel.paint();
                }
                Err(error) if panel.repository.borrow().is_some() => {
                    panel.feedback(&format!("refresh failed: {error}"), true);
                }
                Err(error) => {
                    panel.replace_html(&debug::render_failure(&panel.repo, &panel.branch, &error))
                }
            }
            panel.refreshing.set(false);
            panel.sync_busy_controls();
        });
    }

    fn probe_remote(self: Rc<Self>) {
        if self.probing.get() || self.refreshing.get() || self.repository.borrow().is_none() {
            return;
        }
        self.probing.set(true);
        self.probe.replace(ProbeState::Loading);
        self.paint();
        let panel = self.clone();
        let path = format!(
            "/api/repository/{}/branch/{}/sync/status",
            self.repo, self.branch
        );
        spawn_local(async move {
            match fetch_text(&path).await {
                Ok(body) => panel.probe.replace(ProbeState::Response(body)),
                Err(error) => panel.probe.replace(ProbeState::Failure(error)),
            };
            panel.probing.set(false);
            panel.paint();
        });
    }

    fn paint(&self) {
        let Some(repository) = self.repository.borrow().clone() else {
            return;
        };
        let probe = self.probe.borrow();
        let probe = match &*probe {
            ProbeState::Idle => Probe::Idle,
            ProbeState::Loading => Probe::Loading,
            ProbeState::Response(body) => Probe::Response(body),
            ProbeState::Failure(error) => Probe::Failure(error),
        };
        match debug::render_repository(&self.repo, &self.branch, &repository, probe) {
            Ok(html) => self.replace_html(&html),
            Err(error) => {
                self.replace_html(&debug::render_failure(&self.repo, &self.branch, &error))
            }
        }
        self.sync_busy_controls();
    }

    fn replace_html(&self, html: &str) {
        let expanded = self
            .host
            .query_selector(".inspector-debug__disclosure")
            .ok()
            .flatten()
            .is_some_and(|details| details.has_attribute("open"));
        self.host.set_inner_html(html);
        if expanded {
            if let Some(details) = self
                .host
                .query_selector(".inspector-debug__disclosure")
                .ok()
                .flatten()
            {
                details.set_attribute("open", "").ok();
            }
        }
    }

    fn sync_busy_controls(&self) {
        let busy = self.refreshing.get() || self.probing.get();
        self.host
            .set_attribute("aria-busy", if busy { "true" } else { "false" })
            .ok();
        for selector in ["[data-debug-action=refresh]", "[data-debug-action=probe]"] {
            if let Some(button) = self.host.query_selector(selector).ok().flatten() {
                if busy {
                    button.set_attribute("disabled", "").ok();
                } else {
                    button.remove_attribute("disabled").ok();
                }
            }
        }
    }

    fn copy(self: Rc<Self>, button: Element, value: String) {
        let promise = window().map(|window| window.navigator().clipboard().write_text(&value));
        let Some(promise) = promise else {
            copy_failed(&button, "clipboard unavailable");
            return;
        };
        spawn_local(async move {
            match JsFuture::from(promise).await {
                Ok(_) => {
                    button.set_text_content(Some("copied"));
                    button.set_attribute("aria-label", "copied").ok();
                    button.class_list().add_1("is-copied").ok();
                }
                Err(error) => copy_failed(&button, &format!("{error:?}")),
            }
        });
    }

    fn feedback(&self, message: &str, error: bool) {
        let feedback = self
            .host
            .query_selector(".inspector-debug__transient")
            .ok()
            .flatten()
            .or_else(|| {
                let document = window()?.document()?;
                let feedback = document.create_element("div").ok()?;
                feedback.set_class_name("inspector-debug__transient");
                feedback.set_attribute("role", "status").ok();
                feedback.set_attribute("aria-live", "polite").ok();
                self.host
                    .query_selector(".inspector-debug__body")
                    .ok()
                    .flatten()?
                    .append_child(&feedback)
                    .ok()?;
                Some(feedback)
            });
        if let Some(feedback) = feedback {
            feedback.set_text_content(Some(message));
            let _ = feedback.class_list().toggle_with_force("is-error", error);
        }
    }
}

fn copy_failed(button: &Element, message: &str) {
    button.set_text_content(Some("retry"));
    button
        .set_attribute("aria-label", "copy failed; retry")
        .ok();
    button.set_attribute("title", message).ok();
    button.class_list().add_1("is-error").ok();
}

async fn fetch_text(path: &str) -> Result<String, String> {
    let window = window().ok_or_else(|| "no window available".to_owned())?;
    let value = JsFuture::from(window.fetch_with_str(path))
        .await
        .map_err(|error| format!("GET {path} failed: {error:?}"))?;
    let response: Response = value
        .dyn_into()
        .map_err(|_| format!("GET {path} did not return a response"))?;
    let status = response.status();
    let text = JsFuture::from(
        response
            .text()
            .map_err(|error| format!("GET {path} body: {error:?}"))?,
    )
    .await
    .map_err(|error| format!("GET {path} body: {error:?}"))?
    .as_string()
    .unwrap_or_default();
    if response.ok() {
        Ok(text)
    } else if text.is_empty() {
        Err(format!("GET {path} returned HTTP {status}"))
    } else {
        Err(format!("GET {path} returned HTTP {status}: {text}"))
    }
}

/// Shared notebook state: where to mount cells, how to scope their LSP URIs, and
/// the listener bag spawned cells register into.
struct Notebook {
    host: HtmlElement,
    repo: String,
    branch: String,
    debug_panel: Option<Rc<DebugPanel>>,
    next_id: Cell<u32>,
    closures: Closures,
}

impl Notebook {
    /// Append a fresh active cell, sealing whatever was active before.
    fn spawn_cell(self: &Rc<Self>, auto_focus: bool) {
        // Seal the previous tail: mark its form read-only via the `cell-sealed`
        // class and lock its editor.
        if let Some(prev) = self.host.last_element_child() {
            prev.class_list().add_1("cell-sealed").ok();
            if let Some(editor) = prev.query_selector("tonk-code").ok().flatten() {
                editor.set_attribute("readonly", "").ok();
            }
        }

        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let cell = Rc::new(NotebookCell::build(self, id, auto_focus));
        let _ = self.host.append_child(&cell.form);
        cell.install(self);
    }
}

/// One notebook cell's DOM + state.
struct NotebookCell {
    form: Element,
    editor: Element,
    result_slot: Element,
    source: String,
    /// Latest editor buffer (mirrored on `change`).
    buffer: RefCell<String>,
    /// LSP error-severity count from the latest `diagnostics` frame.
    error_count: Cell<u32>,
    /// True while an evaluate request is in flight.
    running: Cell<bool>,
    /// True once sealed (no more submits).
    sealed: Cell<bool>,
}

impl NotebookCell {
    /// Build the cell's DOM (editor + result slot), detached from the document.
    fn build(notebook: &Notebook, id: u32, auto_focus: bool) -> NotebookCell {
        let document = window().unwrap().document().unwrap();
        let source = lsp_buffer_uri(&notebook.repo, &notebook.branch, id);
        let placeholder = if id == 0 {
            "person:\n  this: ?alice\n  name: \"Alice\"\n\n# or assert with `!`:\n# person!: &alice\n#   name: \"Alice\""
        } else {
            ""
        };

        let form = document.create_element("form").unwrap();
        form.set_class_name("branch-yaml-query wa-stack wa-gap-xs");

        let editor_wrap = document.create_element("div").unwrap();
        editor_wrap.set_class_name("evaluate-editor");

        let editor = document.create_element("tonk-code").unwrap();
        editor.set_attribute("language", "dialog-yaml").ok();
        editor.set_attribute("source", &source).ok();
        editor.set_attribute("active-line", "").ok();
        editor.set_attribute("placeholder", placeholder).ok();
        if auto_focus {
            editor.set_attribute("auto-focus", "").ok();
        }

        let play = document.create_element("wa-button").unwrap();
        play.set_class_name("evaluate-play");
        play.set_attribute("type", "button").ok();
        play.set_attribute("variant", "neutral").ok();
        play.set_attribute("appearance", "filled").ok();
        play.set_attribute("size", "small").ok();
        play.set_attribute("pill", "").ok();
        play.set_attribute("title", "Submit transaction (Shift+Enter)")
            .ok();
        play.set_inner_html("<wa-icon name=\"bolt\" variant=\"solid\"></wa-icon>");

        let _ = editor_wrap.append_child(&editor);
        let _ = editor_wrap.append_child(&play);
        let _ = form.append_child(&editor_wrap);

        let result_slot = document.create_element("div").unwrap();
        let _ = form.append_child(&result_slot);

        NotebookCell {
            form,
            editor,
            result_slot,
            source,
            buffer: RefCell::new(String::new()),
            error_count: Cell::new(0),
            running: Cell::new(false),
            sealed: Cell::new(false),
        }
    }

    /// Wire the cell's event listeners. Closures are kept alive in the notebook's
    /// shared bag (dropped on element disconnect).
    fn install(self: &Rc<Self>, notebook: &Rc<Notebook>) {
        let store = &notebook.closures;

        // Prevent native form submit.
        add(&self.form, "submit", store, |ev: Event| {
            ev.prevent_default()
        });

        // editor `change` → mirror buffer, clear stale pushed diagnostics.
        add(&self.editor, "change", store, {
            let cell = self.clone();
            move |ev: Event| {
                cell.buffer.replace(read_value(&ev));
                clear_pushed_diagnostics(&cell.source);
                cell.refresh_play();
            }
        });

        // editor `run` (Shift+Enter) → explicit submit.
        add(&self.editor, "run", store, {
            let cell = self.clone();
            let notebook = notebook.clone();
            move |_ev: Event| cell.clone().submit(&notebook)
        });

        // play button click → explicit submit.
        if let Some(play) = self.form.query_selector(".evaluate-play").ok().flatten() {
            add(&play, "click", store, {
                let cell = self.clone();
                let notebook = notebook.clone();
                move |ev: Event| {
                    ev.prevent_default();
                    ev.stop_propagation();
                    cell.clone().submit(&notebook);
                }
            });
        }

        // editor `diagnostics` → track error count + auto-eval dry-run on clean.
        add(&self.editor, "diagnostics", store, {
            let cell = self.clone();
            move |ev: Event| {
                let detail = ev
                    .dyn_ref::<CustomEvent>()
                    .map(|c| c.detail())
                    .unwrap_or(JsValue::NULL);
                let error_count = reflect_f64(&detail, "errorCount").unwrap_or(0.0) as u32;
                cell.error_count.set(error_count);
                cell.refresh_play();
                if cell.sealed.get() || error_count > 0 || cell.running.get() {
                    return;
                }
                let body = reflect_string(&detail, "value").unwrap_or_default();
                if body.trim().is_empty() {
                    return;
                }
                let cell2 = cell.clone();
                spawn_local(async move {
                    if let Ok(response) = evaluate(&cell2.editor, &body, false).await {
                        cell2.render(Some(&response), None);
                    }
                });
            }
        });
    }

    /// Show/hide the play affordance: visible only when active, error-free, and
    /// the document has a mutation to commit.
    fn refresh_play(&self) {
        let runnable = !self.sealed.get()
            && self.error_count.get() == 0
            && has_mutation(&self.buffer.borrow());
        if let Some(play) = self.form.query_selector(".evaluate-play").ok().flatten() {
            let _ = play.class_list().toggle_with_force("is-visible", runnable);
        }
    }

    /// Explicit submit — a committing evaluate. On success, render the result,
    /// seal this cell, and spawn a fresh one.
    fn submit(self: Rc<Self>, notebook: &Rc<Notebook>) {
        if self.sealed.get() || self.running.get() {
            return;
        }
        let body = self.buffer.borrow().clone();
        if body.trim().is_empty() {
            return;
        }
        self.running.set(true);
        let notebook = notebook.clone();
        spawn_local(async move {
            match evaluate(&self.editor, &body, true).await {
                Ok(response) => {
                    self.render(Some(&response), None);
                    self.running.set(false);
                    self.sealed.set(true);
                    notify_committed();
                    if let Some(panel) = notebook.debug_panel.as_ref() {
                        panel.clone().refresh_local();
                    }
                    notebook.spawn_cell(true);
                }
                Err(message) => {
                    self.render(None, Some(&message));
                    self.running.set(false);
                }
            }
        });
    }

    /// Render the result slot from a response (or a failure message).
    fn render(&self, response: Option<&EvaluateResponse>, failure: Option<&str>) {
        self.result_slot
            .set_inner_html(&render_result(failure, response));
    }
}

/// Canonical document URI consumed by the scoped language-server route.
///
/// `profile:` is a URI namespace marker, not part of the profile identity. The
/// identity after it and every named repository/branch use the shared segment
/// codec, so reserved bytes cannot change URI structure.
fn lsp_buffer_uri(repo: &str, branch: &str, id: u32) -> String {
    let repo = match repo.strip_prefix("profile:") {
        Some(profile) => format!(
            "profile:{}",
            tonk_worker_api::encode_lsp_scope_segment(profile)
        ),
        None => tonk_worker_api::encode_lsp_scope_segment(repo),
    };
    let branch = tonk_worker_api::encode_lsp_scope_segment(branch);
    format!("tonk-buffer:///{repo}/{branch}/scratch-{id}")
}

/// Resolve `(repo, branch)` from this element's OWN `with="branch@repo"`
/// attribute (forwarded onto it by the mounting `<tonk-display>`; routing
/// is never inferred from DOM ancestors). `None` when absent or unstamped.
///
/// The pair scopes the cells' LSP buffer URIs and labels the header — the
/// actual evaluate routes through the host consumer, which re-resolves the
/// route (space/branch/profile) from this element's `with`.
///
/// A profile context keeps its `profile:<name>` repo token rather than
/// flattening to a bare `profile`: the language server parses this segment
/// back out of the buffer URI to decide which namespace to open, and the
/// profile is not reachable as a repository *named* `profile`. Flattening
/// it sent the LSP looking for a named repo that does not exist, so it
/// opened no branch and completion fell back to built-ins only.
pub(crate) struct InspectorContext {
    pub(crate) repo: String,
    pub(crate) branch: String,
    pub(crate) profile: bool,
}

pub(crate) fn resolve_context(el: &HtmlElement) -> Option<InspectorContext> {
    let location: tonk_host::location::Location = el
        .get_attribute("with")
        .filter(|v| !v.is_empty() && !v.contains('{'))
        .and_then(|v| v.parse().ok())?;
    let repo = match location.space() {
        Some(name) => name.to_owned(),
        // `Repo::Profile`'s `Display` is exactly the `profile:<name>`
        // token; a location with no branch renders bare, so build the
        // segment from the repo half alone.
        None => location.repo.to_string(),
    };
    Some(InspectorContext {
        repo,
        branch: location.effective_branch().to_owned(),
        profile: location.profile(),
    })
}

/// Evaluate `document` against the branch via the `<tonk-host>` consumer.
///
/// Dispatches a `tonk-evaluate` event on the editor element (`consumer`); it
/// bubbles to the `<tonk-host>` ancestor — in the sealed guest, the proxy host,
/// which relays it over the bridge to the host's real `<tonk-host>` consumer. So
/// the inspector uses the SAME host IO path as the in-page editor (the host owns
/// the request, annotates space/branch, and returns the parsed response) rather
/// than issuing its own HTTP. The result is the host's parsed-JSON
/// `EvaluateResponse`; round-trip it through `serde_json` into the local mirror.
pub(crate) async fn evaluate(
    consumer: &Element,
    document: &str,
    transact: bool,
) -> Result<EvaluateResponse, String> {
    let value = tonk_host::consumer::evaluate(consumer, document, transact)
        .await
        .map_err(|detail| detail.message)?;
    // `value` is a `JsValue` from `JSON.parse`; stringify + serde so the field
    // maps decode with the same semantics as the worker's own `Deserialize`.
    let text = js_sys::JSON::stringify(&value)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| "evaluate response was not serializable JSON".to_owned())?;
    serde_json::from_str(&text).map_err(|e| format!("evaluate response decode: {e}"))
}

/// Whether a buffer parses and contains at least one assertion (a mutation).
fn has_mutation(body: &str) -> bool {
    if body.trim().is_empty() {
        return false;
    }
    let parsed = tonk_notation::parse(body);
    if !parsed.diagnostics.is_empty() {
        return false;
    }
    parsed
        .syntax
        .map(|s| {
            s.expressions
                .iter()
                .any(|e| matches!(e, tonk_notation::Expression::Claim(_)))
        })
        .unwrap_or(false)
}

/// Read the `value` property off a `<tonk-code>` element via one of its events.
fn read_value(event: &Event) -> String {
    event
        .target()
        .and_then(|t| t.dyn_into::<HtmlElement>().ok())
        .and_then(|el| reflect_string(&el, "value"))
        .unwrap_or_default()
}

/// Clear externally-pushed diagnostics for `source` (empty
/// `tonk-push-diagnostics` on the provider).
fn clear_pushed_diagnostics(source: &str) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(provider) = document
        .query_selector("tonk-diagnostics-provider")
        .ok()
        .flatten()
    else {
        return;
    };
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &"source".into(), &JsValue::from_str(source));
    let _ = js_sys::Reflect::set(&detail, &"diagnostics".into(), &js_sys::Array::new());
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    if let Ok(event) =
        web_sys::CustomEvent::new_with_event_init_dict("tonk-push-diagnostics", &init)
    {
        let _ = provider.dispatch_event(&event);
    }
}

/// Nudge a mounted sync controller that a commit landed (decoupled window event).
fn notify_committed() {
    if let Some(win) = window() {
        let init = web_sys::CustomEventInit::new();
        init.set_bubbles(false);
        if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("tonk:committed", &init) {
            let _ = win.dispatch_event(&event);
        }
    }
}

/// Add a listener to `target`, keeping the closure alive in `store`.
fn add(target: &Element, event: &str, store: &Closures, handler: impl FnMut(Event) + 'static) {
    let closure = Closure::<dyn FnMut(Event)>::new(handler);
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    store.borrow_mut().push(closure);
}

pub(crate) fn reflect_string(value: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn it_builds_canonical_lsp_uris_for_slash_branches() {
        assert_eq!(
            lsp_buffer_uri("did:key:zSpace", "feat/artifact", 7),
            "tonk-buffer:///did%3Akey%3AzSpace/feat%2Fartifact/scratch-7",
        );
        assert_eq!(
            lsp_buffer_uri("profile:tonk", "feat/artifact", 8),
            "tonk-buffer:///profile:tonk/feat%2Fartifact/scratch-8",
        );
    }

    #[dialog_common::test]
    async fn it_mounts_branch_diagnostics_for_spaces_but_not_profiles() {
        crate::register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let space = document.create_element("tonk-inspector").unwrap();
        space.set_attribute("with", "main@did:key:zSpace").unwrap();
        body.append_child(&space).unwrap();
        assert!(
            space.query_selector(".inspector-debug").unwrap().is_some(),
            "a named-space inspector should mount branch diagnostics"
        );

        let profile = document.create_element("tonk-inspector").unwrap();
        profile.set_attribute("with", "main@profile:tonk").unwrap();
        body.append_child(&profile).unwrap();
        assert!(
            profile
                .query_selector(".inspector-debug")
                .unwrap()
                .is_none(),
            "the profile inspector should keep its notebook-only surface"
        );

        space.remove();
        profile.remove();
    }
}

fn reflect_f64(value: &JsValue, key: &str) -> Option<f64> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_f64())
}
