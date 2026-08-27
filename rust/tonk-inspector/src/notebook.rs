//! `<tonk-notebook>` — a prose document whose ```dialog fences are live
//! query cells.
//!
//! The element is a thin shell around `<tonk-prose>`: prose already mounts a
//! real `<tonk-code>` as a ProseMirror node view for every fenced code block
//! (`tonk-prose/src-js/editor/code-block.ts`), so the editor pairing needs no
//! construction here. What this adds is the *cell* half — for each fence whose
//! language is `dialog`, evaluate its body against the branch and render the
//! result directly beneath the editor.
//!
//! # Why a slot appended into the node view's DOM
//!
//! Each fence renders as `<div class="md-code-block">` wrapping the editor.
//! That div is a stable per-fence anchor: the node view's `update()` only
//! touches the editor's `language` attribute and `value`, never rebuilding
//! `this.dom`, and `ignoreMutation()` returns true — so a result node appended
//! there survives edits and ProseMirror will not fight it. Nothing about the
//! document model changes: the result is chrome around a fence, never content
//! inside it, so the markdown a notebook serializes to stays plain markdown
//! that renders anywhere.
//!
//! # This stage
//!
//! Query cells only (`plan/notebook.md`, build order step 1). A cell that
//! parses as a pure query auto-evaluates as a dry run and renders its matches,
//! exactly as the inspector's cells do. A cell carrying a mutation is
//! recognized and marked, but never run — the checkpoint machinery that gives
//! mutations somewhere to land is a later step, and running them against the
//! live branch in the meantime is precisely what the design rules out.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    CustomEvent, Element, Event, HtmlElement, MutationObserver, MutationObserverInit, window,
};

use crate::element::{evaluate, resolve_context};
use crate::render::render_result;

/// The fence language that marks a code block as a query cell. Other
/// languages stay ordinary code blocks: still editable, never evaluated.
const CELL_LANGUAGE: &str = "dialog";

/// Class of the wrapper the prose code-block node view builds per fence.
const FENCE_SELECTOR: &str = ".md-code-block";

/// Class of the result node this element appends into each fence wrapper.
const RESULT_CLASS: &str = "notebook-cell-result";

/// A bag of retained listener closures, dropped on disconnect.
type Closures = Rc<RefCell<Vec<Closure<dyn FnMut(Event)>>>>;

/// The retained MutationObserver callback — kept alive for the element's
/// lifetime so the observer's closure stays valid.
type MutationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// The retained MutationObserver itself.
type ObserverCell = Rc<RefCell<Option<MutationObserver>>>;

/// The custom element.
#[derive(Default)]
pub struct TonkNotebookElement {
    closures: Closures,
    observer: ObserverCell,
    mutation: MutationClosure,
}

impl CustomElement for TonkNotebookElement {
    fn shadow() -> bool {
        // Light DOM: the app stylesheet styles the prose document and the
        // result slots, the same way it styles the inspector's cells.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Some((repo, branch)) = resolve_context(this) else {
            this.set_inner_html(
                "<div class=\"tonk-notebook\">\
                   <section class=\"error\">no repository in context \
                   (nest under a with=&quot;branch@repo&quot; element)</section>\
                 </div>",
            );
            return;
        };

        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };

        // The document's initial markdown rides in as this element's own text,
        // the way `<tonk-prose>` itself takes content (newline-safe, unlike an
        // attribute). Take it before clearing so it can be handed straight on.
        let source = this.text_content().unwrap_or_default();
        this.set_inner_html("");
        this.set_class_name("tonk-notebook");

        // A diagnostics provider hosts the LSP client for every embedded
        // editor. The sealed guest has no app-wide one, so the notebook
        // supplies its own — the same reason the inspector does.
        let Some(provider) = document
            .create_element("tonk-diagnostics-provider")
            .ok()
            .and_then(|e| e.dyn_into::<HtmlElement>().ok())
        else {
            return;
        };

        let Some(prose) = document.create_element("tonk-prose").ok() else {
            return;
        };
        let _ = prose.set_attribute("placeholder", "Write, and add a ```dialog block…");
        prose.set_text_content(Some(&source));

        let _ = provider.append_child(&prose);
        let _ = this.append_child(&provider);

        let notebook = Rc::new(Notebook {
            prose,
            repo,
            branch,
            closures: self.closures.clone(),
            cells: RefCell::new(HashMap::new()),
        });

        // Fences appear asynchronously: the prose core is lazy-loaded, so the
        // node views (and their `<tonk-code>` elements) do not exist at
        // connect. Watch the subtree and bind whatever fences appear —
        // covering both the initial render and every fence added later by
        // typing. `ready` alone would miss the latter.
        notebook.observe(self.observer.clone(), self.mutation.clone());
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.observer.borrow_mut().take() {
            observer.disconnect();
        }
        self.mutation.borrow_mut().take();
        self.closures.borrow_mut().clear();
    }
}

/// Shared notebook state: the prose document, where to evaluate, and the
/// per-fence cells bound so far.
struct Notebook {
    prose: Element,
    repo: String,
    branch: String,
    closures: Closures,
    /// Fence wrappers already wired, keyed by the cell id stamped on them.
    /// Keeps a re-scan from binding the same fence twice.
    cells: RefCell<HashMap<String, Rc<Cell>>>,
}

impl Notebook {
    /// Watch the prose subtree and bind fences as they appear.
    fn observe(self: &Rc<Self>, slot: ObserverCell, retained: MutationClosure) {
        // Bind whatever is already present (the observer only reports future
        // mutations, and a fast prose core may have rendered already).
        self.bind_fences();

        let notebook = self.clone();
        let callback = Closure::wrap(Box::new(move || {
            notebook.bind_fences();
        }) as Box<dyn FnMut()>);

        let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
            return;
        };
        let init = MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        let _ = observer.observe_with_options(&self.prose, &init);

        *retained.borrow_mut() = Some(callback);
        *slot.borrow_mut() = Some(observer);
    }

    /// Find every `dialog` fence in the document and bind the ones not yet
    /// bound. Idempotent — the stamped id is what makes a re-scan cheap.
    fn bind_fences(self: &Rc<Self>) {
        let Ok(wrappers) = self.prose.query_selector_all(FENCE_SELECTOR) else {
            return;
        };
        for index in 0..wrappers.length() {
            let Some(wrapper) = wrappers
                .item(index)
                .and_then(|n| n.dyn_into::<HtmlElement>().ok())
            else {
                continue;
            };
            // Only `dialog` fences are cells. The editor carries the language
            // the node view read off the fence info string.
            let Some(editor) = wrapper.query_selector("tonk-code").ok().flatten() else {
                continue;
            };
            if editor.get_attribute("language").as_deref() != Some(CELL_LANGUAGE) {
                continue;
            }
            let id = match wrapper.dataset().get("notebookCell") {
                Some(id) => id,
                None => {
                    // Index-derived, stable for as long as the fence keeps its
                    // position. Only used to key the LSP buffer and the cell
                    // map; nothing persists it, so a shift on insert is
                    // harmless here (see `plan/notebook.md`, open question 4).
                    let id = index.to_string();
                    let _ = wrapper.dataset().set("notebookCell", &id);
                    id
                }
            };
            if self.cells.borrow().contains_key(&id) {
                continue;
            }
            let cell = Rc::new(Cell::bind(self, &wrapper, &editor, &id));
            self.cells.borrow_mut().insert(id, cell);
        }
    }
}

/// One bound fence: its editor, the result node beneath it, and the state the
/// auto-evaluate needs.
struct Cell {
    editor: Element,
    result: Element,
}

impl Cell {
    /// Wire one fence: stamp the editor's LSP buffer URI, append a result
    /// node, and evaluate whenever the editor reports a clean frame.
    fn bind(notebook: &Rc<Notebook>, wrapper: &HtmlElement, editor: &Element, id: &str) -> Cell {
        // The LSP buffer URI scopes completion and diagnostics to this branch,
        // the same shape the inspector's cells use. The provider keys its
        // client by this string, so it must be unique per editor.
        let _ = editor.set_attribute(
            "source",
            &format!(
                "tonk-buffer:///{}/{}/notebook-{id}",
                notebook.repo, notebook.branch
            ),
        );

        let result = window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("div").ok())
            .expect("document creates an element");
        result.set_class_name(RESULT_CLASS);
        let _ = wrapper.append_child(&result);

        let cell = Cell {
            editor: editor.clone(),
            result,
        };

        cell.install(notebook);
        cell
    }

    /// Listen for the editor's `diagnostics` frame and evaluate on a clean
    /// one. Mirrors the inspector's auto-evaluate: the LSP has just validated
    /// the buffer, so this is the moment the document is worth running.
    fn install(&self, notebook: &Rc<Notebook>) {
        let editor = self.editor.clone();
        // In-flight guard: a diagnostics burst must not stack evaluates, and
        // a late reply from a superseded run must not overwrite a newer one.
        let running = Rc::new(std::cell::Cell::new(false));

        let cell_result = self.result.clone();
        let cell_editor = editor.clone();
        let closure = Closure::wrap(Box::new(move |event: Event| {
            let detail = event
                .dyn_ref::<CustomEvent>()
                .map(|c| c.detail())
                .unwrap_or(JsValue::NULL);
            let error_count = js_sys::Reflect::get(&detail, &"errorCount".into())
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as u32;
            if error_count > 0 {
                return;
            }
            let body = js_sys::Reflect::get(&detail, &"value".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            if body.trim().is_empty() {
                cell_result.set_inner_html("");
                return;
            }
            // A mutation cell is recognized but never auto-run: there is
            // nowhere for its writes to land yet (checkpoints are a later
            // step), and running it against the live branch is exactly what
            // the design forbids.
            if has_mutation(&body) {
                cell_result.set_inner_html(
                    "<div class=\"notebook-cell-held\">\
                       This cell mutates. Mutation cells do not run automatically.\
                     </div>",
                );
                return;
            }
            if running.get() {
                return;
            }
            running.set(true);
            let slot = cell_result.clone();
            let consumer = cell_editor.clone();
            let in_flight = running.clone();
            spawn_local(async move {
                match evaluate(&consumer, &body, false).await {
                    Ok(response) => slot.set_inner_html(&render_result(None, Some(&response))),
                    Err(message) => slot.set_inner_html(&render_result(Some(&message), None)),
                }
                in_flight.set(false);
            });
        }) as Box<dyn FnMut(Event)>);

        let _ = editor
            .add_event_listener_with_callback("diagnostics", closure.as_ref().unchecked_ref());
        notebook.closures.borrow_mut().push(closure);
    }
}

/// Whether a buffer parses and contains at least one assertion (a mutation).
///
/// Re-exported logic from the inspector: a cell that only queries is safe to
/// run on every clean frame, while one that asserts is not.
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
