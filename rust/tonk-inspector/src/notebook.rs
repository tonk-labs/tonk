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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    CustomEvent, Element, Event, HtmlElement, MutationObserver, MutationObserverInit, window,
};

use crate::blocks::{Block, assign_keys, insert_notation, project, reconcile, split, title_of};
use crate::cell_output::render as render_result;
use crate::element::{evaluate, reflect_string, resolve_context};

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

/// Styles for a cell's output, injected into the editor's shadow root.
///
/// Deliberately spare: an output sits between two paragraphs of prose, so it
/// reads as an annotation on the cell rather than a panel of its own. Colours
/// come from the Web Awesome tokens the rest of the app uses, with fallbacks
/// so an output is still legible where the tokens are absent.
const OUTPUT_CSS: &str = r#"
/* A cell is part of the prose, not a widget sitting in it — so no frame.
   What marks it instead is a faint wash: enough that the block reads as a
   distinct surface, far too little to draw the eye away from the text.
   `color-mix` against the page's own foreground makes it an inverse of
   whatever background is in play, so it works in either theme without two
   sets of colours; the fallback is a flat translucent white for browsers
   without it.

   Done through `<tonk-code>`'s own variables, so the element is unchanged
   everywhere else it is used. */
.md-code-block tonk-code {
  --tonk-code-border: transparent;
  --tonk-code-radius: 4px;
  --tonk-code-bg: rgba(127, 127, 127, 0.06);
  --tonk-code-bg: color-mix(in srgb, currentColor 6%, transparent);
}
/* The cell you are in reads a shade stronger — enough to locate yourself,
   not enough to announce itself. */
.md-code-block:focus-within tonk-code {
  --tonk-code-bg: rgba(127, 127, 127, 0.11);
  --tonk-code-bg: color-mix(in srgb, currentColor 11%, transparent);
}
/* Callouts in a cell's output.
 *
 * A full callout is right on a page, where landing on a raw default or an
 * empty result is a surprise worth explaining. In cell output it is noise:
 * you ran a query to see data, and it dwarfs the result it labels — a
 * sentence in a padded box above two lines of notation.
 *
 * The "no view for this model" notice goes entirely: seeing the default
 * rendering IS the expected outcome here. The rest shrink to a thin bar.
 */
.notebook-cell-result [data-tonk-display-default-notice] {
  display: none;
}
.notebook-cell-result wa-callout {
  --wa-callout-padding: var(--wa-space-2xs, 0.25rem) var(--wa-space-xs, 0.5rem);
  font-size: var(--wa-font-size-xs, 0.75rem);
  line-height: 1.35;
  margin-block: var(--wa-space-2xs, 0.25rem);
}
.notebook-cell-result wa-callout::part(icon) {
  font-size: var(--wa-font-size-s, 0.875rem);
}

/* `<tonk-notation>`'s palette.
 *
 * The element renders into the light DOM and takes its colours from the
 * app stylesheet — which does not cross into this shadow root, so a cell's
 * notation output arrived correctly tokenized and entirely grey. The
 * classes come from the same table `styles.css` uses; the colours come
 * from `<tonk-code>`'s variables, which ARE in scope here, so editor and
 * output stay in step.
 */
/* The palette itself, ON the element.
 *
 * `styles.css` declares these roles on `.query-notation` — a container the
 * inspector provides and a cell's output does not — so every
 * `var(--tonk-code-*)` here resolved to nothing and fell back to inherited
 * grey. Declaring them on `tonk-notation` makes the element carry its own
 * colours wherever it is mounted, which is what a passive renderer should
 * do.
 *
 * Values track the Bauhaus roles in `styles.css`: yellow/structural for
 * keys, alarm for effects, and so on.
 */
tonk-notation {
  --tonk-code-fg: var(--wa-color-text-normal);
  --tonk-code-key: var(--tonk-bauhaus-yellow, #c89a2b);
  --tonk-code-effect: var(--tonk-bauhaus-alarm, #a8302a);
  --tonk-code-name-sigil: var(--wa-color-text-quiet);
  --tonk-code-name: var(--tonk-bauhaus-blue, #3d6da8);
  --tonk-code-entity: var(--tonk-bauhaus-blue, #3d6da8);
  --tonk-code-variable: var(--tonk-bauhaus-grey, #7a7268);
  --tonk-code-font: var(--wa-font-family-code, ui-monospace, monospace);
  --tonk-code-font-size: var(--wa-font-size-s, 0.875rem);
}
tonk-notation .tonk-notation-pre {
  margin: 0;
  background: transparent;
  color: var(--tonk-code-fg);
  font-family: var(--tonk-code-font);
  font-size: var(--tonk-code-font-size);
  line-height: 1.5;
  white-space: pre;
  overflow: auto;
}
tonk-notation .tonk-cm-key { color: var(--tonk-code-key); }
tonk-notation .tonk-cm-effect {
  color: var(--tonk-code-effect);
  font-weight: bold;
}
tonk-notation .tonk-cm-name-sigil { color: var(--tonk-code-name-sigil); }
tonk-notation .tonk-cm-name { color: var(--tonk-code-name); }
tonk-notation .tonk-cm-entity {
  color: var(--tonk-code-entity);
  text-decoration: underline;
  text-decoration-color: var(--tonk-code-entity);
  text-decoration-thickness: 1px;
  text-underline-offset: 2px;
}
tonk-notation .tonk-cm-variable {
  color: var(--tonk-code-variable);
  font-style: italic;
}
.md-code-block {
  margin: 0.5rem 0;
  /* The positioning context the zap anchors to. Without it the button's
     `position: absolute` resolves against some ancestor further up and it
     lands under the cell instead of on it. */
  position: relative;
}
/* The zap floats over the cell's bottom-right corner, half-overlapping its
   edge — the placement the inspector uses. Its own container must not
   reserve space, or the button pushes the output down and stops looking
   like it belongs to the code above it. */
.notebook-cell-held {
  position: static;
  height: 0;
}
.notebook-cell-held .evaluate-play {
  position: absolute;
  bottom: calc(-1 * var(--wa-space-m, 1rem));
  right: var(--wa-space-s, 0.75rem);
  z-index: 1;
}

/* The block the caret is in — the unit a commit writes.
   A block can span several nodes (a heading and the content under it), so
   the mark goes on each of them and they read as one band: the padding
   bleeds into the gutter on both sides, and only the outer edges of the run
   are rounded, so a multi-node block looks like one shape rather than a
   stack of separate ones.

   Quieter than the focused code cell above, deliberately: this says "you
   are here", not "this is selected". */
.nb-block-active {
  background: rgba(127, 127, 127, 0.14);
  background: color-mix(in srgb, currentColor 12%, transparent);
  box-shadow:
    -0.6rem 0 0 0 rgba(127, 127, 127, 0.14),
    0.6rem 0 0 0 rgba(127, 127, 127, 0.14);
  box-shadow:
    -0.6rem 0 0 0 color-mix(in srgb, currentColor 12%, transparent),
    0.6rem 0 0 0 color-mix(in srgb, currentColor 12%, transparent);
}
.nb-block-active:not(.nb-block-active + .nb-block-active) {
  border-start-start-radius: 0.25rem;
  border-start-end-radius: 0.25rem;
}
.nb-block-active:not(:has(+ .nb-block-active)) {
  border-end-start-radius: 0.25rem;
  border-end-end-radius: 0.25rem;
}

.notebook-cell-result { display: block; margin: 0.25rem 0 0.75rem; }
.nb-out {
  font-size: 0.8125rem;
  line-height: 1.45;
  color: var(--wa-color-text-quiet, #5b4953);
}
.nb-out__summary {
  display: flex; align-items: baseline; gap: 0.5rem;
  padding: 0.125rem 0;
}
.nb-out__label {
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  color: var(--wa-color-text-normal, #38182a);
}
.nb-out__count { font-variant-numeric: tabular-nums; }
.nb-out__gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
  gap: 0.375rem;
  margin-top: 0.375rem;
}
.nb-card {
  border: 1px solid var(--wa-color-neutral-border-quiet, rgb(56 24 42 / 18%));
  border-radius: var(--wa-border-radius-m, 0);
  padding: 0.375rem 0.5rem;
  overflow: hidden;
}
.nb-card__title {
  font-family: var(--wa-font-family-code, ui-monospace, monospace);
  color: var(--wa-color-text-normal, #38182a);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  margin-bottom: 0.125rem;
}
.nb-card__field {
  display: flex; gap: 0.375rem;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.nb-card__key { color: var(--wa-color-text-quiet, #5b4953); }
.nb-card__value {
  color: var(--wa-color-text-normal, #38182a);
  overflow: hidden; text-overflow: ellipsis;
}
.nb-card__more, .nb-out__more {
  margin-top: 0.25rem;
  font-style: italic;
}
.nb-out--error {
  display: flex; gap: 0.5rem; align-items: baseline;
  color: var(--wa-color-text-normal, #38182a);
}
.nb-out__icon { font-weight: 700; }
"#;

/// Gap between projection retries, and how many. The observer is the real
/// mechanism; these are the safety net for a pane that is replaced wholesale
/// (which detaches the watched node) rather than filled in place. Measured
/// against a cold load, where the nested displays settle around a second in.
const RETRY_MS: i32 = 120;
const RETRIES: u32 = 25;

/// How long to wait for the editor to settle before committing. Must clear
/// `<tonk-prose>`'s own 400ms change debounce, or a commit reads a prefix of
/// what was typed.
const SETTLE_MS: i32 = 600;

/// Class of the result node this element appends into each fence wrapper.
const RESULT_CLASS: &str = "notebook-cell-result";

/// A bag of retained listener closures, dropped on disconnect.
type Closures = Rc<RefCell<Vec<Closure<dyn FnMut(Event)>>>>;

/// The retained MutationObserver callback — kept alive for the element's
/// lifetime so the observer's closure stays valid.
type MutationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// The retained MutationObserver itself.
type ObserverCell = Rc<RefCell<Option<MutationObserver>>>;

/// A single retained event closure, kept alive for as long as the listener
/// it backs stays registered.
type ListenerCell = RefCell<Option<Closure<dyn FnMut(Event)>>>;

/// The custom element.
#[derive(Default)]
pub struct TonkNotebookElement {
    /// Set the instant a mount is claimed, so the two lifecycle callbacks
    /// that can both fire cannot each spawn one.
    mounting: Rc<std::cell::Cell<bool>>,
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
        // Claim the mount SYNCHRONOUSLY, before deferring. Both this callback
        // and `attribute_changed_callback` spawn a task, and a DOM check
        // inside those tasks is too late: each runs its guard before either
        // has appended anything, so both pass and two editors mount. The
        // second one wins the screen while this element's state points at the
        // first — which is the orphaned-prose symptom, not a separate bug.
        if self.mounting.replace(true) {
            return;
        }
        let host = this.clone();
        let closures = self.closures.clone();
        let observer = self.observer.clone();
        let mutation = self.mutation.clone();
        let mounting = self.mounting.clone();
        spawn_local(async move {
            if !host.is_connected() {
                // Not in the document: release the claim so the re-attach
                // that follows can mount for real.
                mounting.set(false);
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
        // Release the claim: a re-attach must be able to mount again, and its
        // own guard on the already-present provider stops a duplicate.
        self.mounting.set(false);
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
        // pass would build a second provider and a second editor, and the
        // fresh empty one would win the projection.
        //
        // Keyed on the PROVIDER, which is attached synchronously below. The
        // editor is not: it waits for `<tonk-code>` to be defined, so a guard
        // on `tonk-prose` is still false when the next callback arrives and
        // every pass mounts again.
        if this
            .query_selector("tonk-diagnostics-provider")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let Some(context) = resolve_context(this) else {
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
        // A DRAFT notebook's heading is a switcher: type a title and it
        // suggests the notebooks that match, opens the one you pick, or
        // creates the name you typed. A saved notebook's heading only
        // renames - were the switcher live there, renaming onto an
        // existing title would navigate the author out of the document
        // they were editing.
        let draft = this.has_attribute("draft");
        if draft {
            let _ = prose.set_attribute("switcher", "");
            let _ = prose.set_attribute("auto-focus", "");
            let _ = prose.set_attribute("value", "# ");
            let _ = prose.set_attribute("placeholder", "Name a notebook to open or create it...");
        } else {
            let _ = prose.set_attribute("placeholder", "Write, and add a ```dialog-yaml block…");
            // A notebook page is somewhere you came to WRITE, so it opens
            // ready to: focused, caret at the end of the document. Arriving
            // from the switcher, naming a notebook and carrying on typing
            // is then one motion rather than two.
            let _ = prose.set_attribute("auto-focus", "");
            let _ = prose.set_attribute("caret", "end");
        }

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
            repo: context.repo,
            branch: context.branch,
            closures: closures.clone(),
            cells: RefCell::new(HashMap::new()),
            blocks: RefCell::new(Vec::new()),
            projected: RefCell::new(String::new()),
            title: RefCell::new(None),
            next_cell: std::cell::Cell::new(0),
            projected_once: std::cell::Cell::new(false),
            marked: std::cell::Cell::new(-1),
            selection_listener: RefCell::new(None),
            selection_bound: std::cell::Cell::new(false),
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
    /// The title last emitted, so a commit that leaves the heading alone
    /// does not rewrite the notebook's name.
    title: RefCell<Option<String>>,
    /// Next cell id. Monotonic, so an id is never reused by a later fence.
    next_cell: std::cell::Cell<u32>,
    /// Whether the store's blocks have been projected into the editor yet.
    /// Until they have, an edit has nothing truthful to diff against.
    projected_once: std::cell::Cell<bool>,
    /// The block index the highlight currently marks, so marking is a no-op
    /// when the caret has not left its block. Writing `class` is a DOM
    /// mutation that re-fires `selectionchange`; without this the marker
    /// re-enters itself and spins.
    marked: std::cell::Cell<i32>,
    /// The `selectionchange` closure, kept alive and re-registered on the
    /// prose shadow root once the lazy-loaded editor creates one.
    selection_listener: ListenerCell,
    /// Whether that root listener has been added, so it is added once.
    selection_bound: std::cell::Cell<bool>,
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
            // Project on a fresh task, never inline.
            //
            // A MutationObserver callback runs SYNCHRONOUSLY inside the DOM
            // write that triggered it — and ProseMirror writes the editor's
            // DOM from `dispatchTransaction`. Projecting inline therefore
            // called `setMarkdown` in the middle of a ProseMirror
            // transaction, replacing the document under the transaction that
            // was still applying: "TextSelection endpoint not pointing into
            // a node with inline content", then "no position after the
            // top-level node" on the next keypress, with the caret landing
            // past the end of a document that had been swapped beneath it.
            //
            // `settling` guards re-entry from our OWN writes; it cannot
            // guard this, because the re-entry comes from ProseMirror's
            // update rather than ours. Deferring lets the transaction finish
            // and applies the projection to a settled editor.
            let deferred_notebook = notebook.clone();
            let deferred = Closure::once_into_js(move || {
                if deferred_notebook.settling.get() {
                    return;
                }
                deferred_notebook.settling.set(true);
                deferred_notebook.project_blocks();
                deferred_notebook.bind_fences();
                deferred_notebook.settling.set(false);
            });
            if let Some(window) = window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    deferred.unchecked_ref(),
                    0,
                );
            }
            // Re-register the shadow watch every tick: the root only exists
            // once the editor has mounted, which is after this observer was
            // created. Observing an already-observed target with the same
            // options is a no-op.
            if let (Some(observer), Some(root)) =
                (watched.borrow().as_ref(), notebook.prose.shadow_root())
            {
                let init = MutationObserverInit::new();
                init.set_child_list(true);
                init.set_subtree(true);
                let node: web_sys::Node = root.into();
                let _ = observer.observe_with_options(node.unchecked_ref::<Element>(), &init);
            }
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
        // And the shadow root, where the document actually renders. A
        // MutationObserver does not cross a shadow boundary, so watching only
        // the host misses every fence.
        if let Some(root) = self.prose.shadow_root() {
            let node: web_sys::Node = root.into();
            let _ = observer.observe_with_options(node.unchecked_ref::<Element>(), &init);
        }
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
        // Sources come from the block rows, one per block entity, in
        // whatever order they landed. Order comes from the notebook's own
        // `block` sequence, rendered as `.notebook-order__item` rows in
        // position order with the entry's key on each.
        //
        // The rows come from a DIRECTORY display — every block in the
        // space, not just this notebook's — because a block row is
        // rendered per block entity rather than per notebook. Each row
        // carries the notebook it belongs to, so the filter is here:
        // without it a notebook shows every other notebook's blocks,
        // and editing one rewrites blocks it does not own.
        let notebook = self.host.dataset().get("notebook");
        let mut sources: HashMap<String, String> = HashMap::new();
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
            if let Some(notebook) = &notebook
                && dataset.get("notebook").as_deref() != Some(notebook.as_str())
            {
                continue;
            }
            if sources.insert(entity.clone(), source).is_none() {
                arrival.push(entity);
            }
        }
        if sources.is_empty() {
            return;
        }

        let mut ordered: Vec<Block> = Vec::new();
        let mut placed: HashSet<String> = HashSet::new();
        if let Ok(entries) = self.host.query_selector_all(".notebook-order__item") {
            for index in 0..entries.length() {
                let Some(entry) = entries
                    .item(index)
                    .and_then(|n| n.dyn_into::<HtmlElement>().ok())
                else {
                    continue;
                };
                let dataset = entry.dataset();
                let (Some(entity), Some(key)) = (dataset.get("block"), dataset.get("key")) else {
                    continue;
                };
                // An entry whose block has no source row yet is skipped, not
                // rendered blank: its row may simply not have landed.
                let Some(source) = sources.get(&entity) else {
                    continue;
                };
                if placed.insert(entity.clone()) {
                    ordered.push(Block {
                        entity,
                        source: source.clone(),
                        key: Some(key),
                    });
                }
            }
        }
        // A block with a source but no entry (its placement never landed)
        // still shows, at the end, and gets a key on the next commit.
        for entity in arrival {
            if !placed.contains(&entity) {
                let source = sources[&entity].clone();
                ordered.push(Block {
                    entity,
                    source,
                    key: None,
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

        // NEVER overwrite an edit in progress.
        //
        // The editor's current text is the author's; `projected` is what this
        // element last wrote. When they differ, the author has typed since —
        // and writing the store's projection over that discards the typing and
        // jumps the caret to the end. Which is exactly what "I changed the
        // query, it reverted, and my keystrokes ended up in the paragraph
        // below" looks like.
        //
        // The store's version is not lost: `commit` writes the author's text
        // on block exit, the rows update, and the next projection matches.
        // A genuine remote change while typing is deferred to that same
        // moment rather than snatching the document mid-keystroke.
        if let Some(live) = reflect_string(self.prose.as_ref(), "value")
            && !live.is_empty()
            && live != *self.projected.borrow()
        {
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

    /// Record a cell's source as the text that was just evaluated.
    ///
    /// Called with the body the cell ran, so the stored source and the
    /// rendered result are the same text by construction. Writes ONE block —
    /// the one this cell projects from — rather than re-splitting the
    /// document, so it cannot disturb a block the author is editing
    /// elsewhere.
    ///
    /// A cell's fence is the block's whole source, so the stored text is the
    /// body wrapped back in its fence.
    fn record_cell_source(self: &Rc<Self>, cell_id: &str, body: &str) {
        // Which block this cell came from. The cell id indexes the fences in
        // document order, and `blocks` is in that same order, so find the
        // n-th block that is a fence.
        let Ok(index) = cell_id.parse::<usize>() else {
            return;
        };
        let entity = {
            let blocks = self.blocks.borrow();
            let mut fences = blocks
                .iter()
                .filter(|b| b.source.trim_start().starts_with("```"));
            match fences.nth(index) {
                Some(block) => block.entity.clone(),
                None => return,
            }
        };

        let fenced = format!("```{CELL_LANGUAGE}\n{}\n```", body.trim_end());
        // Unchanged is not worth a write, and a write per keystroke is
        // exactly what block storage exists to avoid.
        {
            let blocks = self.blocks.borrow();
            if blocks
                .iter()
                .any(|b| b.entity == entity && b.source == fenced)
            {
                return;
            }
        }
        // Keep the local view in step so the next projection does not read
        // this back as an external change and fight the editor.
        {
            let mut blocks = self.blocks.borrow_mut();
            if let Some(block) = blocks.iter_mut().find(|b| b.entity == entity) {
                block.source = fenced.clone();
            }
        }
        self.dispatch_edit(&entity, &fenced);
    }

    /// Commit once the editor has settled.
    ///
    /// `<tonk-prose>` coalesces edits behind a 400ms debounce, and a fence's
    /// text reaches the prose document only through that path — so reading
    /// immediately yields a stale, often mid-word document. Waiting past the
    /// debounce is what makes the committed text the text that was typed.
    fn commit_when_settled(self: Rc<Self>) {
        let notebook = self;
        let callback = Closure::once_into_js(move || notebook.commit());
        if let Some(window) = window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.unchecked_ref(),
                SETTLE_MS,
            );
        }
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

        // A DRAFT has no notebook to write to yet.
        //
        // Its blocks would be created against no owner, so they would
        // render nowhere and be re-minted on the next keystroke. The
        // author's body text is not lost: naming the notebook creates it,
        // the page navigates there, and what was typed is projected into
        // the real one. Until then the draft is a document in the editor
        // and nothing else.
        if self.host.has_attribute("draft") {
            return;
        }
        let next = split(&document);
        let edit = reconcile(&self.blocks.borrow(), &next);

        for (entity, source) in &edit.changed {
            self.dispatch_edit(entity, source);
        }

        // The document's heading IS the notebook's title, so an edit that
        // changes the heading renames the notebook. Emitted only when it
        // actually changed: a rename on every commit would write a fact per
        // keystroke-flush, and the title is the one field the index reads.
        if let Some(title) = title_of(&document)
            && self.title.borrow().as_deref() != Some(title.as_str())
        {
            self.dispatch_retitle(&title);
            *self.title.borrow_mut() = Some(title);
        }
        // A created block is INSERTED, and the element names no entity.
        //
        // Identity derives from the command body; the position derives from
        // the predecessor's. The element used to mint the entity itself,
        // from the document's block COUNT — a position wearing an identity's
        // clothes, which repeats whenever the count returns to a value it
        // held, so a new block claimed one that already existed and
        // `reconcile` wrote one block's source onto another.
        //
        // Each insert names the block it FOLLOWS. For a run of new blocks
        // only the first has a stored predecessor; the rest follow the block
        // before them in the document, which is itself new. That resolves
        // because the rule reads the predecessor through `block/position`,
        // which covers a position derived in this same commit as readily as
        // a stored one — so the element never has to name an entity that
        // does not exist yet.
        let notebook = self.notebook_entity();
        let stored = self.blocks.borrow();
        // Inserts go as NOTATION, not as one command event per block.
        //
        // A run of new blocks has to say which follows which, and a block
        // created in the same transaction has no entity to name. Notation
        // can name it by variable — `this: ?b0` on one assertion, `head:
        // ?b0` on the next — which an event cannot carry. Backward
        // references are the only kind that analyze, and a run only ever
        // needs to look back.
        if let Some(document) = insert_notation(&notebook, &edit.order, &edit.created) {
            let consumer = self.host.clone();
            spawn_local(async move {
                if let Err(message) = evaluate(&consumer, &document, true).await {
                    web_sys::console::error_1(
                        &format!("notebook: insert failed: {message}").into(),
                    );
                }
            });
        }

        // Placement is for blocks that MOVED. A created block is placed by
        // the insert rules, so re-keying it here would place it twice.
        let order: Vec<(String, Option<String>)> = edit
            .order
            .iter()
            .filter_map(|slot| {
                slot.as_ref().map(|entity| {
                    let key = stored
                        .iter()
                        .find(|block| &block.entity == entity)
                        .and_then(|block| block.key.clone());
                    (entity.clone(), key)
                })
            })
            .collect();
        for (entity, key) in assign_keys(&order) {
            self.dispatch_place(&entity, &notebook, &key);
        }
        for entity in &edit.removed {
            if let Some(key) = stored
                .iter()
                .find(|block| &block.entity == entity)
                .and_then(|block| block.key.clone())
            {
                self.dispatch_remove(entity, &notebook, &key);
            }
        }
        drop(stored);

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
        // The owning notebook rides along on EVERY edit, not just a creating
        // one. A block written with only its source does not match
        // `tonk:notebook/block`, so it renders no row: the order names an
        // entity that never appears, and a newly typed block vanishes on
        // reload. Re-asserting it on an existing block is a no-op.
        let notebook = self.notebook_entity();
        let _ = js_sys::Reflect::set(&detail, &"notebook".into(), &notebook.as_str().into());
        self.emit("blockedit", &detail);
    }

    /// Emit `titlechange`, which the library's `notebook/retitle` command
    /// reads to rename the notebook.
    fn dispatch_retitle(&self, title: &str) {
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"title".into(), &title.into());
        let notebook = self.notebook_entity();
        let _ = js_sys::Reflect::set(&detail, &"notebook".into(), &notebook.as_str().into());
        self.emit("titlechange", &detail);
    }

    /// The notebook these blocks belong to.
    ///
    /// Read from a stored block rather than the host's `data-subject`, which
    /// `dispatch_edit` repoints at whichever entity it is currently writing.
    fn notebook_entity(&self) -> String {
        if let Some(entity) = self.host.dataset().get("notebook") {
            return entity;
        }
        self.host
            .dataset()
            .get("subject")
            .unwrap_or_else(|| "id:notebook/scratch".to_owned())
    }

    /// Dispatch one `block/place` command: put `entity` into `notebook`'s
    /// sequence under `key`.
    fn dispatch_place(&self, entity: &str, notebook: &str, key: &str) {
        let _ = self.host.dataset().set("subject", entity);
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"notebook".into(), &notebook.into());
        let _ = js_sys::Reflect::set(&detail, &"key".into(), &key.into());
        self.emit("place", &detail);
    }

    /// Dispatch one `block/remove` command: retract `entity`'s entry
    /// under `key` from `notebook`'s sequence.
    fn dispatch_remove(&self, entity: &str, notebook: &str, key: &str) {
        let _ = self.host.dataset().set("subject", entity);
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"notebook".into(), &notebook.into());
        // `removed`, not `key`: the two commands must not share a shape, or
        // one event would decode as both.
        let _ = js_sys::Reflect::set(&detail, &"removed".into(), &key.into());
        self.emit("remove", &detail);
    }

    /// Fire a bubbling CustomEvent off the host, which the view has wired to
    /// a command (`onblockedit=block/edit`, `onplace=block/place`,
    /// `onremove=block/remove`).
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
                notebook.clone().commit_when_settled();
            }
            tracked.set(index);
            notebook.mark_active_block();
        }) as Box<dyn FnMut(Event)>);
        let _ = self
            .prose
            .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
        self.closures.borrow_mut().push(on_change);

        // Moving the caret without typing still moves the block, so the
        // highlight tracks `selectionchange` too.
        //
        // On the SHADOW ROOT, not on `document`: the editor's content lives
        // inside `<tonk-prose>`'s shadow tree, and a selection there fires
        // `selectionchange` on the root that contains it. A listener on the
        // document sees the initial focus and nothing after — every caret
        // move within the editor is invisible to it.
        let notebook = self.clone();
        let on_selection = Closure::wrap(Box::new(move |_event: Event| {
            notebook.mark_active_block();
        }) as Box<dyn FnMut(Event)>);
        // On the DOCUMENT, and on the shadow root once it exists.
        //
        // A selection inside a shadow tree fires `selectionchange` on that
        // root, not on the document — but the prose core is lazy-loaded, so
        // at construction there is no root to attach to yet. Registering on
        // the document covers the pre-upgrade window; `retarget_selection`
        // adds the root listener the first time one is seen.
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            let _ = document.add_event_listener_with_callback(
                "selectionchange",
                on_selection.as_ref().unchecked_ref(),
            );
        }
        self.selection_listener.borrow_mut().replace(on_selection);

        // Leaving the editor is also leaving the block. `focusout` (not
        // `blur`) because blur does not bubble out of the editor's inner
        // contenteditable to the host element.
        let notebook = self.clone();
        let on_blur = Closure::wrap(Box::new(move |_event: Event| {
            notebook.clone().commit_when_settled();
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

    /// Style the cell outputs inside the editor's shadow root.
    ///
    /// Results render INSIDE prose's shadow root, which document styles do
    /// not reach — so without this an output is unstyled text at full width,
    /// which is what made a query look like a wall of YAML.
    ///
    /// A small dedicated sheet rather than adopting the page's: the page's
    /// rules are written for a full-height inspector panel, and pulling all
    /// of them across would drag unrelated layout into an editor they were
    /// never written for. These are the notebook's own output styles, and
    /// they are the only thing this element renders.
    ///
    /// Idempotent — keyed on a flag stamped on the root.
    fn adopt_page_styles(&self) {
        let Some(root) = self.prose.shadow_root() else {
            return;
        };
        if js_sys::Reflect::get(&root, &"__tonkNotebookStyled".into())
            .map(|flag| flag.is_truthy())
            .unwrap_or(false)
        {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(style) = document.create_element("style") else {
            return;
        };
        style.set_text_content(Some(OUTPUT_CSS));
        let node: web_sys::Node = root.clone().into();
        if node.append_child(&style).is_ok() {
            let _ = js_sys::Reflect::set(&root, &"__tonkNotebookStyled".into(), &true.into());
        }
    }

    /// Where the editor's document actually lives.
    ///
    /// `<tonk-prose>` renders into a SHADOW ROOT, so the element itself has no
    /// children and a light-DOM query finds nothing — which is why every fence
    /// scan came back empty and the cells looked like plain markdown. Falls
    /// back to the element for a build that ever renders light.
    fn editor_root(&self) -> Element {
        self.prose
            .shadow_root()
            .map(|root| {
                let node: web_sys::Node = root.into();
                node.unchecked_into::<Element>()
            })
            .unwrap_or_else(|| self.prose.clone())
    }

    /// Mark the top-level nodes of the block the caret sits in.
    ///
    /// A notebook block is the commit unit, and it can span several nodes —
    /// a heading rides with the content under it. ProseMirror's own
    /// selection classes are per node, so on their own they show a heading
    /// lit while the paragraph it introduces stays dark, even though the
    /// two save together. This marks the whole run instead, so what is
    /// highlighted is what a commit will write.
    fn mark_active_block(&self) {
        // Only mark when the caret is actually in THIS notebook.
        //
        // The listener is document-wide, and a notebook nests: every result
        // card renders a `<tonk-display>` whose view is itself a notebook, so
        // one document can hold dozens, each with its own listener. Without
        // this check every one of them runs on every caret move and clears
        // `.md-doc > *` — including the marks the notebook the caret is
        // actually in just set. The band appeared and was wiped in the same
        // tick, so nothing ever rendered.
        if !self.holds_selection() {
            return;
        }
        // Writing `class` on a node is itself a DOM mutation, and mutating
        // inside the editor re-fires `selectionchange` — so marking
        // unconditionally re-enters this immediately and spins. Only touch
        // the DOM when the span actually moved.
        let caret = self.caret_block_index().unwrap_or(-1);
        if self.marked.get() == caret {
            return;
        }
        self.marked.set(caret);

        // THIS editor's document, and its children only.
        //
        // `query_selector_all(".md-doc > *")` on the shadow root matches every
        // `.md-doc` under it — and a notebook nests, so the six notebooks a
        // result gallery renders each contribute their own. The node list then
        // runs past this document's blocks and its indices no longer line up
        // with `projected`, so the span marked the wrong nodes and the clear
        // reached into other notebooks' documents.
        let Ok(Some(md_doc)) = self.editor_root().query_selector(".md-doc") else {
            return;
        };
        let nodes = md_doc.children();
        // Clear first: the caret leaving a block has to unmark it even when
        // the new position resolves to nothing.
        for index in 0..nodes.length() {
            if let Some(node) = nodes.item(index) {
                let _ = node.class_list().remove_1("nb-block-active");
            }
        }
        if caret < 0 {
            return;
        }
        let document = self.projected.borrow().clone();
        let Some((start, len)) = crate::blocks::span_at(&document, caret as usize) else {
            return;
        };
        for index in start..start + len {
            if let Some(node) = nodes.item(index as u32) {
                let _ = node.class_list().add_1("nb-block-active");
            }
        }
    }

    /// Whether the selection sits inside this notebook's editor.
    ///
    /// Tested against the EDITOR ROOT, not the `<tonk-prose>` host:
    /// `Node.contains` does not cross a shadow boundary, and the anchor the
    /// selection reports is the text node inside prose's shadow tree — so a
    /// containment test on the host is false even when the caret is right
    /// there. The shadow root does contain it.
    fn holds_selection(&self) -> bool {
        let Some(selection) = window().and_then(|w| w.get_selection().ok().flatten()) else {
            return false;
        };
        let Some(anchor) = selection.anchor_node() else {
            return false;
        };
        let root: web_sys::Node = self.editor_root().into();
        root.contains(Some(&anchor))
    }

    /// Attach the `selectionchange` listener to the prose shadow root, once
    /// it exists. The editor is lazy-loaded, so the root is absent when the
    /// element is constructed and a document listener alone never sees a
    /// caret move inside it.
    fn retarget_selection(&self) {
        if self.selection_bound.get() {
            return;
        }
        let Some(root) = self.prose.shadow_root() else {
            return;
        };
        let listener = self.selection_listener.borrow();
        let Some(listener) = listener.as_ref() else {
            return;
        };
        let target: web_sys::EventTarget = root.into();
        let _ = target
            .add_event_listener_with_callback("selectionchange", listener.as_ref().unchecked_ref());
        self.selection_bound.set(true);
    }

    /// Find every `dialog` fence in the document and bind the ones not yet
    /// bound. Idempotent — the stamped id is what makes a re-scan cheap.
    fn bind_fences(self: &Rc<Self>) {
        let Ok(wrappers) = self.editor_root().query_selector_all(FENCE_SELECTOR) else {
            return;
        };
        self.adopt_page_styles();
        self.retarget_selection();
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
                    // A MONOTONIC counter, not the loop index. Two fences with
                    // the same index at different times are different cells:
                    // an unbound fence appearing at index 0 would reuse the id
                    // of a cell already bound there, collide in `cells`, and be
                    // skipped — left with no LSP source, so no diagnostics and
                    // no evaluation. Which is exactly what a second code block
                    // looked like.
                    //
                    // Only keys the LSP buffer and the cell map; nothing
                    // persists it (see `plan/notebook.md`, open question 4).
                    let id = self.next_cell.get().to_string();
                    self.next_cell.set(self.next_cell.get() + 1);
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
        let source = format!(
            "tonk-buffer:///{}/{}/notebook-{id}",
            notebook.repo, notebook.branch
        );
        let _ = editor.set_attribute("source", &source);

        // Announce the editor to the provider ourselves.
        //
        // `<tonk-code>` fires `tonk-code-connect` from its own
        // `connectedCallback`, but these editors are created by prose's node
        // view INSIDE its shadow root, on prose's schedule — which can be
        // before this element appended the editor into the provider at all.
        // A bubbling announcement made while the tree is still detached
        // reaches nothing, and the provider then has no LSP client for the
        // buffer: no diagnostics, no completion, and so no auto-evaluate.
        //
        // Re-announcing here is safe: the provider keys its documents by
        // `source`, so a repeat under the same URI is idempotent.
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"source".into(), &source.as_str().into());
        let _ = js_sys::Reflect::set(&detail, &"language".into(), &CELL_LANGUAGE.into());
        let init = web_sys::CustomEventInit::new();
        init.set_detail(&detail);
        init.set_bubbles(true);
        init.set_composed(true);
        if let Ok(event) = CustomEvent::new_with_event_init_dict("tonk-code-connect", &init) {
            let _ = editor.dispatch_event(&event);
        }

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

        // Commit when the caret LEAVES this cell.
        //
        // The document-level tracking watches prose's top-level block index,
        // which an edit inside a fence never moves — the caret is in the
        // fence's own CodeMirror, not in the prose document. Without this a
        // cell's edits are never written at all: you change a query, click
        // away, and the next projection restores the old text.
        let leaving = notebook.clone();
        let on_blur = Closure::wrap(Box::new(move |_event: Event| {
            // Let prose settle before reading it. `commit` serializes the
            // PROSE document, and a fence's live text reaches that document
            // through the node view's forwarding, behind prose's own 400ms
            // change debounce. Reading on the blur tick captures the document
            // mid-word: changing `concept:` to `name:` stored `n:`, the state
            // at the instant the first keystroke had propagated.
            //
            // Longer than that debounce, so what is read is the settled
            // document rather than a prefix of it.
            leaving.clone().commit_when_settled();
        }) as Box<dyn FnMut(Event)>);
        let _ =
            editor.add_event_listener_with_callback("focusout", on_blur.as_ref().unchecked_ref());
        notebook.closures.borrow_mut().push(on_blur);

        let cell = Cell {
            editor: editor.clone(),
            result,
        };

        cell.install(notebook, id);
        cell
    }

    /// Listen for the editor's `diagnostics` frame and evaluate on a clean
    /// one. Mirrors the inspector's auto-evaluate: the LSP has just validated
    /// the buffer, so this is the moment the document is worth running.
    fn install(&self, notebook: &Rc<Notebook>, id: &str) {
        let editor = self.editor.clone();
        // In-flight guard: a diagnostics burst must not stack evaluates, and
        // a late reply from a superseded run must not overwrite a newer one.
        let running = Rc::new(std::cell::Cell::new(false));

        let cell_result = self.result.clone();
        let cell_editor = editor.clone();
        let held_closures = notebook.closures.clone();
        let notebook_for_cell = notebook.clone();
        let cell_id = id.to_owned();
        // The notebook's own routing context, handed to every card this cell
        // renders. Results live in prose's shadow root, and context
        // resolution reads an element's own `with` rather than walking
        // ancestors — so a `<tonk-display>` that is not stamped here resolves
        // no repository, and whatever its view mounts renders "no repository
        // in context" in place of the entity.
        let cell_context = format!("{}@{}", notebook.branch, notebook.repo);
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
            // Store the text we are ABOUT TO RUN, not a re-read of the
            // document later.
            //
            // A cell's stored source and its rendered result must be the same
            // text — otherwise a notebook shows an answer to a question it
            // does not contain. Re-reading the prose document at commit time
            // could not guarantee that: prose coalesces edits behind a
            // debounce, so the read raced the typing and stored a prefix
            // (`concept:` edited to `name:` was stored as `n:`).
            //
            // Handing the evaluated body straight to the notebook removes the
            // race rather than timing around it: what ran is what is written.
            notebook_for_cell.record_cell_source(&cell_id, &body);
            // A mutation cell is recognized but never auto-run: there is
            // nowhere for its writes to land yet (checkpoints are a later
            // step), and running it against the live branch is exactly what
            // the design forbids.
            if has_mutation(&body) {
                // A mutation is never run on a diagnostics frame. Offer it
                // instead: the same bolt the inspector uses to commit, so a
                // cell that writes is run deliberately rather than by typing.
                if cell_result
                    .query_selector(".evaluate-play")
                    .ok()
                    .flatten()
                    .is_none()
                {
                    // The same control the inspector uses: a filled pill
                    // that half-overlaps the cell's bottom-right corner. A
                    // bare `<button>` here read as a stray glyph rather
                    // than as the deliberate act it is.
                    cell_result.set_inner_html(
                        "<div class=\"notebook-cell-held\">\
                           <wa-button type=\"button\" class=\"evaluate-play is-visible\" \
                             variant=\"neutral\" appearance=\"filled\" size=\"small\" pill \
                             title=\"Run this cell — it writes (Cmd/Ctrl+Enter)\">\
                             <wa-icon name=\"bolt\" variant=\"solid\"></wa-icon>\
                           </wa-button>\
                         </div>",
                    );
                    if let Some(play) = cell_result.query_selector(".evaluate-play").ok().flatten()
                    {
                        let consumer = cell_editor.clone();
                        let slot = cell_result.clone();
                        let with = cell_context.clone();
                        let run = Closure::wrap(Box::new(move |event: Event| {
                            event.prevent_default();
                            event.stop_propagation();
                            let Some(body) = reflect_string(consumer.as_ref(), "value") else {
                                return;
                            };
                            let slot = slot.clone();
                            let consumer = consumer.clone();
                            let with = with.clone();
                            spawn_local(async move {
                                // `transact: true` — the deliberate act.
                                match evaluate(&consumer, &body, true).await {
                                    Ok(response) => slot.set_inner_html(&render_result(
                                        None,
                                        Some(&response),
                                        &with,
                                    )),
                                    Err(message) => slot.set_inner_html(&render_result(
                                        Some(&message),
                                        None,
                                        &with,
                                    )),
                                }
                            });
                        })
                            as Box<dyn FnMut(Event)>);
                        let _ = play.add_event_listener_with_callback(
                            "click",
                            run.as_ref().unchecked_ref(),
                        );
                        held_closures.borrow_mut().push(run);
                    }
                }
                return;
            }
            if running.get() {
                return;
            }
            running.set(true);
            let slot = cell_result.clone();
            let consumer = cell_editor.clone();
            let in_flight = running.clone();
            let with = cell_context.clone();
            spawn_local(async move {
                match evaluate(&consumer, &body, false).await {
                    Ok(response) => {
                        slot.set_inner_html(&render_result(None, Some(&response), &with))
                    }
                    Err(message) => {
                        slot.set_inner_html(&render_result(Some(&message), None, &with))
                    }
                }
                in_flight.set(false);
            });
        }) as Box<dyn FnMut(Event)>);

        let _ = editor
            .add_event_listener_with_callback("diagnostics", closure.as_ref().unchecked_ref());
        notebook.closures.borrow_mut().push(closure);

        // Cmd/Ctrl+Enter (and Shift+Enter) commit the cell, without
        // reaching for the zap.
        //
        // `<tonk-code>` already binds both and fires `run`; the inspector
        // listens for it and the notebook did not, so the keystroke did
        // nothing here. This is the zap's exact effect — a committing
        // evaluate — so a cell that writes runs deliberately either way.
        let run_editor = editor.clone();
        let run_slot = self.result.clone();
        let run_with = format!("{}@{}", notebook.branch, notebook.repo);
        let on_run = Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
            event.stop_propagation();
            let Some(body) = reflect_string(run_editor.as_ref(), "value") else {
                return;
            };
            let slot = run_slot.clone();
            let consumer = run_editor.clone();
            let with = run_with.clone();
            spawn_local(async move {
                match evaluate(&consumer, &body, true).await {
                    Ok(response) => {
                        slot.set_inner_html(&render_result(None, Some(&response), &with))
                    }
                    Err(message) => {
                        slot.set_inner_html(&render_result(Some(&message), None, &with))
                    }
                }
            });
        }) as Box<dyn FnMut(Event)>);
        let _ = editor.add_event_listener_with_callback("run", on_run.as_ref().unchecked_ref());
        notebook.closures.borrow_mut().push(on_run);
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
