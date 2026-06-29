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
//! `<tonk-repository>` / `<tonk-branch>` ancestors (the route view provides them)
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
use web_sys::{CustomEvent, Element, Event, HtmlElement, Request, RequestInit, Response, window};

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
        let Some((repo, branch)) = resolve_context(this) else {
            this.set_inner_html(
                "<div class=\"tonk-inspector\">\
                   <section class=\"error\">no repository in context \
                   (nest under &lt;tonk-repository&gt;)</section>\
                 </div>",
            );
            return;
        };

        this.set_class_name("tonk-inspector branch-cells wa-stack wa-gap-s");
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

        let notebook = Rc::new(Notebook {
            host,
            repo,
            branch,
            next_id: Cell::new(0),
            closures: self.closures.clone(),
        });
        notebook.spawn_cell(true);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.closures.borrow_mut().clear();
    }
}

/// Shared notebook state: where to mount cells, how to scope their LSP URIs, and
/// the listener bag spawned cells register into.
struct Notebook {
    host: HtmlElement,
    repo: String,
    branch: String,
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
        // Re-announce the editor to the diagnostics provider after a frame. The
        // `<tonk-code>` and `<tonk-diagnostics-provider>` definitions arrive in
        // one async-imported bundle; if `tonk-code` upgrades first, the editor's
        // initial `tonk-code-connect` (from its connectedCallback) fires before
        // the provider installs its listener and is lost — no LSP, no diagnostics,
        // no auto-eval. Detaching + re-attaching the editor re-runs its
        // connectedCallback (an unconditional re-announce) once both have
        // upgraded. Harmless: the cell is freshly spawned with no user input yet.
        reannounce_editor(&cell.editor);
    }
}

/// Detach and re-attach `editor` on the next animation frame so its
/// `connectedCallback` re-runs and re-fires `tonk-code-connect` — see
/// [`Notebook::spawn_cell`]. (A `language` re-set is insufficient: the editor
/// only re-announces on a language change when it already has an LSP client.)
fn reannounce_editor(editor: &Element) {
    let Some(win) = window() else {
        return;
    };
    let editor = editor.clone();
    let cb = Closure::once_into_js(move || {
        if let Some(parent) = editor.parent_node() {
            let next = editor.next_sibling();
            let _ = parent.remove_child(&editor);
            let _ = parent.insert_before(&editor, next.as_ref());
        }
    });
    let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
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
        let source = format!(
            "tonk-buffer:///{}/{}/scratch-{id}",
            notebook.repo, notebook.branch
        );
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
            let notebook = notebook.clone();
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
                let notebook = notebook.clone();
                spawn_local(async move {
                    if let Ok(response) = evaluate(&notebook, &body, false).await {
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
            match evaluate(&notebook, &body, true).await {
                Ok(response) => {
                    self.render(Some(&response), None);
                    self.running.set(false);
                    self.sealed.set(true);
                    notify_committed();
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

/// Resolve `(repo, branch)` from the nearest `<tonk-repository>` /
/// `<tonk-branch>` ancestors. `None` until both are found.
fn resolve_context(el: &HtmlElement) -> Option<(String, String)> {
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

/// POST the document to the branch's evaluate endpoint. In the sealed guest the
/// portal's `window.fetch` proxy routes this over the bridge.
async fn evaluate(
    notebook: &Notebook,
    document: &str,
    transact: bool,
) -> Result<EvaluateResponse, String> {
    let url = format!(
        "/api/repository/{}/branch/{}/evaluate?transact={transact}",
        notebook.repo, notebook.branch
    );
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(document));
    let request = Request::new_with_str_and_init(&url, &init)
        .map_err(|_| "bad evaluate request".to_owned())?;
    request
        .headers()
        .set("content-type", "text/plain")
        .map_err(|_| "header set failed".to_owned())?;
    let win = window().ok_or_else(|| "no window".to_owned())?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
        .await
        .map_err(|e| {
            reflect_string(&e, "message").unwrap_or_else(|| "evaluate fetch failed".to_owned())
        })?;
    let response: Response = resp_value
        .dyn_into()
        .map_err(|_| "evaluate response not a Response".to_owned())?;
    let text_value = JsFuture::from(
        response
            .text()
            .map_err(|_| "evaluate response had no body".to_owned())?,
    )
    .await
    .map_err(|_| "evaluate body read failed".to_owned())?;
    let text = text_value.as_string().unwrap_or_default();
    if !response.ok() {
        return Err(error_message(&text).unwrap_or(text));
    }
    serde_json::from_str(&text).map_err(|e| format!("evaluate response decode: {e}"))
}

/// Pull a `{ error: { message } }` envelope's message out of an error body.
fn error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_owned)
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

fn reflect_string(value: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

fn reflect_f64(value: &JsValue, key: &str) -> Option<f64> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_f64())
}
