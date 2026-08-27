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
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    CustomEvent, Element, Event, HtmlElement, MutationObserver, MutationObserverInit, window,
};

use crate::blocks::{Block, project, reconcile, split};
use crate::element::{evaluate, reflect_string, resolve_context};
use crate::render::render_result;

/// The language pack a cell's editor uses — the id `<tonk-code>` resolves a
/// grammar by (`tonk-code/assets/tonk-code-lang-dialog-yaml.js`).
const CELL_LANGUAGE: &str = "dialog-yaml";

/// Fence info words that mark a code block as a query cell. `dialog` is the
/// spelling an author reaches for; `dialog-yaml` is what the language pack is
/// actually called, and both must work — a fence tagged `dialog` that
/// silently stayed inert would be a trap.
///
/// An UNTAGGED fence (bare ```) is a cell too: in a notebook the common case
/// is a query, so typing three backticks should give you one without having
/// to remember the tag.
const CELL_LANGUAGES: [&str; 2] = ["dialog", "dialog-yaml"];

/// Class of the wrapper the prose code-block node view builds per fence.
const FENCE_SELECTOR: &str = ".md-code-block";

/// Gap between projection retries, and how many. The observer is the real
/// mechanism; these are the safety net for a pane that is replaced wholesale
/// (which detaches the watched node) rather than filled in place. Measured
/// against a cold load, where the nested displays settle around a second in.
const RETRY_MS: i32 = 120;
const RETRIES: u32 = 25;

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
        // `<tonk-display>` forwards its routing context by stamping `with`
        // AFTER mounting the view, so the first `connectedCallback` often has
        // no context to resolve. Observe it, and mount when it lands.
        &["with"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // DEFERRED one microtask, and guarded on `is_connected`.
        //
        // The custom-element reaction queue delivers this callback after the
        // enclosing reaction ends — and when a `<tonk-display>` render pass is
        // that enclosing reaction, its diff may have already detached this
        // element again by then. Mounting anyway builds an editor inside an
        // orphan: the store's rows render into the element that IS in the
        // document, while this instance polls a detached subtree forever.
        // (Diagnostic signature: `connected=false` with rows present in the
        // document but none under the host.)
        let host = this.clone();
        let closures = self.closures.clone();
        let observer = self.observer.clone();
        let mutation = self.mutation.clone();
        spawn_local(async move {
            if !host.is_connected() {
                return;
            }
            mount(&host, closures, observer, mutation);
        });
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        // The context arriving is the cue to mount: `mount` bails when it
        // cannot resolve one, so without this a notebook whose `with` is
        // stamped post-mount would stay on its error message forever.
        self.connected_callback(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.observer.borrow_mut().take() {
            observer.disconnect();
        }
        self.mutation.borrow_mut().take();
        self.closures.borrow_mut().clear();
    }
}

/// Build the editor and start watching for rows. Split out of
/// `connected_callback` so the deferred, connectedness-guarded path is the
/// only way in.
fn mount(
    this: &HtmlElement,
    closures: Closures,
    observer_slot: ObserverCell,
    mutation_slot: MutationClosure,
) {
    {
        // Mount ONCE. `connectedCallback` fires on every re-attach, and
        // `<tonk-display>` stamps `with` after mounting the view — so a second
        // pass here would build a second provider and a second editor, and the
        // fresh empty one would win the projection. Keyed on the editor
        // already being present rather than on a flag, so a re-attach that
        // kept the subtree is recognized as such.
        if this.query_selector("tonk-prose").ok().flatten().is_some() {
            return;
        }
        let Some((repo, branch)) = resolve_context(this) else {
            this.set_inner_html(
                "<div class=\"tonk-notebook\">\
                   <section class=\"error\">no repository in context \
                   (nest under a with=&quot;branch@repo&quot; element)</section>\
                 </div>",
            );
            return;
        };

        // A prior pass may have left the no-context message; clear it now
        // that the context resolved, or it sits above the editor forever.
        if let Ok(Some(message)) = this.query_selector(".tonk-notebook > .error") {
            message.remove();
        }

        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };

        // The blocks arrive as hidden `.notebook-block-row` nodes rendered by a
        // nested `<tonk-display>` (the wiki's data-pane pattern), so they stay
        // reactive. They are not there yet at connect — the display resolves
        // asynchronously — so the projection is (re)built whenever they change.
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
        let _ = prose.set_attribute("placeholder", "Write, and add a ```dialog-yaml block…");

        // Attach the provider now, but hold the EDITOR back until
        // `<tonk-code>` is defined.
        //
        // Prose decides per code block, at draw time, whether to mount a real
        // `<tonk-code>` node view or fall back to a plain CodeMirror
        // (`code-block.ts:320`), and it never re-decides. Its bundle is
        // imported asynchronously by the guest, so a prose editor mounted
        // first draws every fence as the fallback: no `<tonk-code>`, hence no
        // LSP client, no diagnostics, no autocomplete, and nothing for this
        // element to hang a result on — a glorified markdown viewer.
        let _ = this.append_child(&provider);

        let notebook = Rc::new(Notebook {
            host: this.clone(),
            prose,
            repo,
            branch,
            closures: closures.clone(),
            cells: RefCell::new(HashMap::new()),
            blocks: RefCell::new(Vec::new()),
            projected: RefCell::new(String::new()),
            projected_once: std::cell::Cell::new(false),
            settling: std::cell::Cell::new(false),
        });

        notebook.install_editor_listeners();

        // Fences appear asynchronously: the prose core is lazy-loaded, so the
        // node views (and their `<tonk-code>` elements) do not exist at
        // connect. Watch the subtree and bind whatever fences appear —
        // covering both the initial render and every fence added later by
        // typing. `ready` alone would miss the latter.
        notebook.observe(observer_slot, mutation_slot);

        // Now mount the editor, once its embedded-editor dependency exists.
        let registry = window().map(|w| w.custom_elements());
        match registry.and_then(|r| r.when_defined("tonk-code").ok()) {
            Some(defined) => {
                let provider = provider.clone();
                let prose = notebook.prose.clone();
                spawn_local(async move {
                    let _ = JsFuture::from(defined).await;
                    let _ = provider.append_child(&prose);
                });
            }
            // No registry (not a browser) — mount anyway rather than never.
            None => {
                let _ = provider.append_child(&notebook.prose);
            }
        }
    }
}

/// Shared notebook state: the prose document, where to evaluate, and the
/// per-fence cells bound so far.
struct Notebook {
    /// The `<tonk-notebook>` element itself — where the block rows live and
    /// where the edit commands are dispatched from.
    host: HtmlElement,
    prose: Element,
    repo: String,
    branch: String,
    closures: Closures,
    /// Fence wrappers already wired, keyed by the cell id stamped on them.
    /// Keeps a re-scan from binding the same fence twice.
    cells: RefCell<HashMap<String, Rc<Cell>>>,
    /// The blocks currently projected into the editor, in document order.
    /// An edit is diffed against these, so only what moved is written.
    blocks: RefCell<Vec<Block>>,
    /// The document text last handed to the editor. Guards against writing
    /// back the editor's own echo of a store update.
    projected: RefCell<String>,
    /// Whether the store's blocks have been projected into the editor yet.
    /// Until they have, an edit has nothing truthful to diff against.
    projected_once: std::cell::Cell<bool>,
    /// True while this element is mutating its own DOM. The observer watches
    /// the editor subtree, and binding a fence writes into it (a result slot,
    /// a `source` attribute), so without this each bind re-enters the
    /// observer callback that triggered it.
    settling: std::cell::Cell<bool>,
}

impl Notebook {
    /// Watch the prose subtree and bind fences as they appear.
    fn observe(self: &Rc<Self>, slot: ObserverCell, retained: MutationClosure) {
        // Project and bind whatever is already present (the observer only
        // reports future mutations, and the rows may have landed already).
        self.project_blocks();
        self.bind_fences();

        let notebook = self.clone();
        let watched = slot.clone();
        let callback = Closure::wrap(Box::new(move || {
            if notebook.settling.get() {
                return;
            }
            notebook.settling.set(true);
            notebook.project_blocks();
            notebook.bind_fences();
            notebook.settling.set(false);
            // Re-register the pane watch every tick: the pane may have only
            // just arrived, and observing an already-observed target with the
            // same options is a no-op.
            if let (Some(observer), Ok(Some(pane))) = (
                watched.borrow().as_ref(),
                notebook.host.query_selector(".notebook-data"),
            ) {
                let init = MutationObserverInit::new();
                init.set_child_list(true);
                init.set_subtree(true);
                init.set_character_data(true);
                init.set_attributes(true);
                let _ = observer.observe_with_options(&pane, &init);
            }
        }) as Box<dyn FnMut()>);

        let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
            return;
        };
        let init = MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        // Rows carry their data in `data-*` attributes, so a store update that
        // rewrites an existing row is an ATTRIBUTE change, not a child list
        // one. Without this a block edit never reaches the projection.
        init.set_attributes(true);
        init.set_character_data(true);
        let _ = observer.observe_with_options(&self.prose, &init);
        // The hidden block rows are siblings of the editor, under the host.
        // Watch the ROW CONTAINER, not the host: the editor also lives under
        // the host, so a subtree observer there would see `project_blocks`'s
        // own write into the editor and re-fire itself forever.
        //
        // The container is rendered by a nested `<tonk-display>` that resolves
        // asynchronously, so it is usually absent at connect. Watch the host's
        // direct children (childList WITHOUT subtree, which the editor's own
        // mutations never reach) until it appears, then watch it properly.
        // Watch the ROW PANE's whole subtree. The rows render deep inside it,
        // from nested `<tonk-display>`s that resolve on their own schedule —
        // often hundreds of milliseconds after connect, and always after any
        // bounded retry would have given up.
        //
        // The pane is a static child of the view template, so it is normally
        // here already. When it is not, watch the host's DIRECT children so
        // its arrival is noticed and the pane can then be watched properly.
        // Deliberately not a host SUBTREE watch: that would see the editor's
        // own mutations and re-enter the projection that caused them.
        if let Ok(Some(pane)) = self.host.query_selector(".notebook-data") {
            let _ = observer.observe_with_options(&pane, &init);
        }
        let shallow = MutationObserverInit::new();
        shallow.set_child_list(true);
        let _ = observer.observe_with_options(&self.host, &shallow);

        *retained.borrow_mut() = Some(callback);
        *slot.borrow_mut() = Some(observer);

        // The rows can also land BEFORE the observer registers — the nested
        // displays resolve on their own schedule, and a MutationObserver only
        // reports future mutations. Re-check on a few animation frames so a
        // race that lost is still caught; each pass is idempotent (an
        // unchanged projection is a no-op) and they stop as soon as one
        // projects.
        self.clone().retry_projection(RETRIES);
    }

    /// Re-attempt the projection for a few frames, in case the rows landed
    /// before the observer was watching.
    fn retry_projection(self: Rc<Self>, remaining: u32) {
        if remaining == 0 || self.projected_once.get() {
            return;
        }
        let notebook = self.clone();
        let callback = Closure::once_into_js(move || {
            if !notebook.settling.get() {
                notebook.settling.set(true);
                notebook.project_blocks();
                notebook.bind_fences();
                notebook.settling.set(false);
            }
            notebook.retry_projection(remaining - 1);
        });
        if let Some(window) = window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.unchecked_ref(),
                RETRY_MS,
            );
        }
    }

    /// Read the hidden block rows and project them into the editor.
    ///
    /// Called whenever the rows change, which covers both the first render
    /// and every store update. Writing the same text twice is a no-op in the
    /// editor (`setMarkdown` returns early when the markdown already matches),
    /// so an echo of our own write does not disturb the caret.
    fn project_blocks(self: &Rc<Self>) {
        let Ok(rows) = self.host.query_selector_all(".notebook-block-row") else {
            return;
        };
        // Rows arrive in entity order, not document order. The notebook's
        // `order` attribute is the sequence; anything the order does not name
        // is appended, so a block whose row landed before the order updated
        // still shows rather than vanishing.
        let mut by_entity: HashMap<String, String> = HashMap::new();
        let mut arrival: Vec<String> = Vec::new();
        for index in 0..rows.length() {
            let Some(row) = rows
                .item(index)
                .and_then(|n| n.dyn_into::<HtmlElement>().ok())
            else {
                continue;
            };
            let dataset = row.dataset();
            let (Some(entity), Some(source)) = (dataset.get("block"), dataset.get("source")) else {
                continue;
            };
            if by_entity.insert(entity.clone(), source).is_none() {
                arrival.push(entity);
            }
        }
        if by_entity.is_empty() {
            return;
        }

        // The order arrives as a hidden row, not an attribute: the space view
        // that mounts this element has the REPLICA as its model, so it cannot
        // bind a notebook's fields. Fall back to the attribute for a caller
        // that can supply one directly.
        let order = self
            .host
            .query_selector(".notebook-row")
            .ok()
            .flatten()
            .and_then(|row| row.dyn_into::<HtmlElement>().ok())
            .and_then(|row| row.dataset().get("order"))
            .or_else(|| self.host.get_attribute("order"))
            .unwrap_or_default();
        let mut ordered: Vec<Block> = Vec::new();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for entity in order.lines().map(str::trim).filter(|e| !e.is_empty()) {
            if let Some(source) = by_entity.get(entity) {
                seen.insert(entity);
                ordered.push(Block {
                    entity: entity.to_owned(),
                    source: source.clone(),
                });
            }
        }
        for entity in &arrival {
            if !seen.contains(entity.as_str()) {
                ordered.push(Block {
                    entity: entity.clone(),
                    source: by_entity[entity].clone(),
                });
            }
        }

        let sources: Vec<String> = ordered.iter().map(|b| b.source.clone()).collect();
        let document = project(&sources);
        *self.blocks.borrow_mut() = ordered;
        self.projected_once.set(true);
        if document == *self.projected.borrow() {
            return;
        }
        *self.projected.borrow_mut() = document.clone();
        // `.value`, not text content: the light-DOM text is read once at
        // mount, while the property routes through `setMarkdown`, which
        // narrows to the blocks that actually differ and leaves the caret
        // alone. Writing text content on a live editor would reset it.
        //
        // `commit` compares against `projected` (set just above), so the
        // `change` this write provokes finds nothing to write back.
        let _ = js_sys::Reflect::set(&self.prose, &"value".into(), &JsValue::from_str(&document));
    }

    /// Commit the editor's current document: split it into blocks, diff
    /// against what was projected, and dispatch one command per change.
    ///
    /// Called when the caret leaves a block (and on blur), not on every
    /// keystroke — a revision should say what the author finished, not what
    /// their keyboard did.
    fn commit(self: &Rc<Self>) {
        // `<tonk-prose>` exposes the document's markdown on `.value`.
        let Some(document) = reflect_string(self.prose.as_ref(), "value") else {
            return;
        };
        if document == *self.projected.borrow() {
            return;
        }

        // Never commit before the store's blocks have been projected. Until
        // then `blocks` is empty, so `reconcile` reads every block as newly
        // created: the edit mints fresh entities, leaves the real ones
        // untouched, and writes an order naming only the new ones — which is
        // exactly "I typed something and on reload it was gone".
        //
        // An explicit flag, not `blocks.is_empty()`: a genuinely empty
        // notebook has no blocks either, and must still accept its first.
        if !self.projected_once.get() {
            return;
        }
        let next = split(&document);
        let edit = reconcile(&self.blocks.borrow(), &next);

        for (entity, source) in &edit.changed {
            self.dispatch_edit(entity, source);
        }
        // A created block needs an entity before it can be written. Mint one
        // from the notebook's own subject plus a counter, so re-running an
        // identical edit does not mint a second entity for the same block.
        let subject = self
            .host
            .dataset()
            .get("subject")
            .unwrap_or_else(|| "id:notebook".to_owned());
        let mut minted: Vec<String> = Vec::new();
        for (nth, source) in edit.created.iter().enumerate() {
            let entity = format!("{subject}/block-{}-{nth}", edit.order.len());
            self.dispatch_edit(&entity, source);
            minted.push(entity);
        }

        if edit.reordered || !edit.created.is_empty() || !edit.removed.is_empty() {
            let mut fresh = minted.iter();
            let order: Vec<String> = edit
                .order
                .iter()
                .filter_map(|slot| match slot {
                    Some(entity) => Some(entity.clone()),
                    None => fresh.next().cloned(),
                })
                .collect();
            self.dispatch_reorder(&subject, &order.join("\n"));
        }

        *self.projected.borrow_mut() = document;
    }

    /// Dispatch one `block/edit` command: `{source}` on the detail, with
    /// `data-subject` naming the block the rule writes to.
    ///
    /// The event is `blockedit`, NOT `change`: the inner `<tonk-prose>`
    /// dispatches its own bubbling `change` carrying `{value, content}`,
    /// which would reach the same handler and fail to resolve `source`.
    fn dispatch_edit(&self, entity: &str, source: &str) {
        let _ = self.host.dataset().set("subject", entity);
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"source".into(), &source.into());
        self.emit("blockedit", &detail);
    }

    /// Dispatch one `notebook/reorder` command carrying the new order.
    fn dispatch_reorder(&self, subject: &str, order: &str) {
        let _ = self.host.dataset().set("subject", subject);
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"order".into(), &order.into());
        self.emit("reorder", &detail);
    }

    /// Fire a bubbling CustomEvent off the host, which the view has wired to
    /// a command (`onchange=block/edit`, `onreorder=notebook/reorder`).
    fn emit(&self, name: &str, detail: &js_sys::Object) {
        let init = web_sys::CustomEventInit::new();
        init.set_detail(detail);
        init.set_bubbles(true);
        if let Ok(event) = CustomEvent::new_with_event_init_dict(name, &init) {
            let _ = self.host.dispatch_event(&event);
        }
    }

    /// Watch the caret so a block commits when the author leaves it, and
    /// flush on blur so leaving the editor entirely does not lose the edit.
    fn install_editor_listeners(self: &Rc<Self>) {
        // The block the caret last sat in, as a top-level child index.
        let last: Rc<std::cell::Cell<i32>> = Rc::new(std::cell::Cell::new(-1));

        // `<tonk-prose>` dispatches only `ready` and `change`, and
        // `selectionchange` fires on `document`, never on an element — so the
        // caret is sampled on each debounced `change` rather than watched
        // directly. An edit that stays inside one block therefore does not
        // commit; moving to another block does, on that block's first
        // keystroke. Blur covers leaving without typing again.
        let notebook = self.clone();
        let tracked = last.clone();
        let on_change = Closure::wrap(Box::new(move |event: Event| {
            // The inner editor's `change` is ITS event, and it bubbles and is
            // composed. Left alone it reaches the host, where the view has
            // wired `onblockedit`/command handlers, and its `{value, content}`
            // detail fails to resolve `source`. Worse, `project_blocks`'
            // `.value` write goes through `setMarkdown`, which dispatches a
            // transaction and so fires this event too — the notebook's own
            // projection would raise an edit command for a change nobody made.
            // Stop it here; the notebook re-emits its own `blockedit` per
            // changed block on commit.
            event.stop_propagation();
            let index = notebook.caret_block_index().unwrap_or(-1);
            if tracked.get() >= 0 && index != tracked.get() {
                notebook.commit();
            }
            tracked.set(index);
        }) as Box<dyn FnMut(Event)>);
        let _ = self
            .prose
            .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
        self.closures.borrow_mut().push(on_change);

        // Leaving the editor is also leaving the block. `focusout` (not
        // `blur`) because blur does not bubble out of the editor's inner
        // contenteditable to the host element.
        let notebook = self.clone();
        let on_blur = Closure::wrap(Box::new(move |_event: Event| {
            notebook.commit();
        }) as Box<dyn FnMut(Event)>);
        let _ = self
            .prose
            .add_event_listener_with_callback("focusout", on_blur.as_ref().unchecked_ref());
        self.closures.borrow_mut().push(on_blur);
    }

    /// The top-level block index the caret sits in, read off the raw
    /// ProseMirror view the editor handle exposes.
    fn caret_block_index(&self) -> Option<i32> {
        let editor = js_sys::Reflect::get(&self.prose, &"editor".into()).ok()?;
        if editor.is_falsy() {
            return None;
        }
        let view = js_sys::Reflect::get(&editor, &"view".into()).ok()?;
        let state = js_sys::Reflect::get(&view, &"state".into()).ok()?;
        let selection = js_sys::Reflect::get(&state, &"selection".into()).ok()?;
        let head = js_sys::Reflect::get(&selection, &"$head".into()).ok()?;
        // `index(0)` is the caret's position among the doc's top-level
        // children — exactly the block it sits in.
        let index = js_sys::Reflect::get(&head, &"index".into()).ok()?;
        let index: js_sys::Function = index.dyn_into().ok()?;
        index
            .call1(&head, &JsValue::from_f64(0.0))
            .ok()?
            .as_f64()
            .map(|n| n as i32)
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
            let language = editor.get_attribute("language").unwrap_or_default();
            // A bare fence has no language at all; treat it as a cell.
            if !language.is_empty() && !CELL_LANGUAGES.contains(&language.as_str()) {
                continue;
            }
            // Neither `dialog` nor the empty string names a pack; point the
            // editor at the real grammar so it highlights instead of erroring.
            if language != CELL_LANGUAGE {
                let _ = editor.set_attribute("language", CELL_LANGUAGE);
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

        // Tab inside a cell belongs to the editor (accept a completion, else
        // indent), but the embedded editor sits in a ProseMirror node view
        // whose host is focusable, so an unhandled Tab moves focus out of the
        // document instead. Swallow it here: CodeMirror's own keymap has
        // already run by the time this fires on the host, so preventing the
        // default only stops the focus move.
        let closure = Closure::wrap(Box::new(move |event: Event| {
            let Some(keyboard) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
                return;
            };
            if keyboard.key() == "Tab" {
                event.prevent_default();
            }
        }) as Box<dyn FnMut(Event)>);
        let _ =
            editor.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        notebook.closures.borrow_mut().push(closure);

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
