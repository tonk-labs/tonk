//! `<tonk-notation>` — syntax-highlighted display of dialog-yaml
//! notation text. Mirrors the `<wa-markdown>` authoring shape: the
//! source notation goes inside a `<script type="text/tonk-notation">`
//! child, which is inert by virtue of its unknown MIME type, so the
//! browser doesn't execute it and it doesn't render visually.
//!
//! ```html
//! <tonk-notation>
//!   <script type="text/tonk-notation">greeting!: &demo
//!     this: did:key:zX
//!     message: "Hello"</script>
//! </tonk-notation>
//! ```
//!
//! The element observes the source script for text changes and
//! re-renders a sibling `<pre class="tonk-notation-pre">` whose
//! `<span>`s carry decoration class names matching the dialog-yaml
//! editor pack — see [`notation_tokens`][crate::notation_tokens] for
//! the tokenizer and class-name table. A single palette in
//! `styles.css` colors both the editor and these read-only renders.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlElement, MutationObserver, MutationObserverInit, window};

use crate::notation_tokens::{Decoration, Mark, collect_marks};

/// MIME type the source `<script>` carries. Matches the
/// `<wa-markdown>` convention's `text/markdown`; the suffix
/// distinguishes our notation from any other carrier.
const SOURCE_MIME: &str = "text/tonk-notation";

/// Class on the rendered `<pre>` we mount as a sibling of the
/// source `<script>`. Tagged so we can find and remove it on
/// re-render without disturbing other children the author might
/// have placed inside the element.
const PRE_CLASS: &str = "tonk-notation-pre";

struct Inner {
    /// Watches the host's direct children for the source
    /// `<script>` being added, removed, or swapped. Cheap; fires
    /// rarely. Kept separate from `source_observer` because we
    /// have to rebind `source_observer` whenever the source
    /// element identity changes.
    _host_observer: MutationObserver,
    /// Watches the source `<script>`'s text for edits. `None`
    /// until a source script is found; rebound when the source
    /// is replaced. Watching only this element (not the host
    /// subtree) is what keeps the renderer's own `<pre>`
    /// insertions from re-entrantly triggering re-renders.
    source_observer: Option<MutationObserver>,
    /// The `<script>` we're currently observing. Held so we can
    /// detect when the script element identity actually changes
    /// and skip rebinding when it doesn't.
    source_el: Option<Element>,
    /// The closure backing `source_observer`. Kept alive (and
    /// reused across rebinds) so the JS-side function pointer
    /// stays valid even when we reassign the observer.
    _source_closure: Closure<dyn FnMut()>,
}

#[derive(Default)]
pub struct TonkNotation {
    inner: RefCell<Option<Rc<RefCell<Inner>>>>,
}

impl CustomElement for TonkNotation {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();

        // State first — created empty so the two observer
        // closures can capture an `Rc` to it. The first sync
        // tick (below) populates `source_el` and the source
        // observer.
        let state: Rc<RefCell<Inner>> = Rc::new_cyclic(|weak: &Weak<RefCell<Inner>>| {
            // Host observer — fires when the host's direct children
            // change. We use it to detect the source `<script>`
            // being added, removed, or replaced. Our own `<pre>`
            // insertions also trip it (the browser doesn't let us
            // filter by tag), so the callback checks whether the
            // current source-script identity matches what we last
            // saw and short-circuits if it hasn't moved.
            let host_for_cb = host.clone();
            let weak_for_host = weak.clone();
            let host_cb = Closure::wrap(Box::new(move || {
                let Some(state) = weak_for_host.upgrade() else {
                    return;
                };
                let current = source_script(&host_for_cb);
                let same = {
                    let inner = state.borrow();
                    match (inner.source_el.as_ref(), current.as_ref()) {
                        (Some(a), Some(b)) => a.is_same_node(Some(b.as_ref())),
                        (None, None) => true,
                        _ => false,
                    }
                };
                if same {
                    // Our own `<pre>` churn — ignore. Without this
                    // guard the append in `render` re-triggers
                    // this callback, which calls `render`, ad
                    // infinitum.
                    return;
                }
                sync(&host_for_cb, &state);
            }) as Box<dyn FnMut()>);
            let host_observer = MutationObserver::new(host_cb.as_ref().unchecked_ref())
                .expect("MutationObserver::new(host)");
            host_cb.forget();
            let host_opts = MutationObserverInit::new();
            host_opts.set_child_list(true);
            let _ = host_observer.observe_with_options(host.as_ref(), &host_opts);

            // Source observer — bound lazily inside `sync` once we
            // know which `<script>` to watch. The closure is built
            // once and kept alive in `Inner._source_closure`; the
            // `MutationObserver` itself may be re-created when the
            // source element identity changes.
            let host_for_source = host.clone();
            let weak_for_source = weak.clone();
            let source_cb = Closure::wrap(Box::new(move || {
                if weak_for_source.upgrade().is_some() {
                    // Source text changed — re-render but don't
                    // touch the source observer binding (the script
                    // identity didn't change).
                    render(&host_for_source);
                }
            }) as Box<dyn FnMut()>);

            RefCell::new(Inner {
                _host_observer: host_observer,
                source_observer: None,
                source_el: None,
                _source_closure: source_cb,
            })
        });
        *self.inner.borrow_mut() = Some(state.clone());

        // First sync: find the source script (if any), bind the
        // source observer to it, and paint once.
        sync(&host, &state);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Drop everything — both MutationObservers will be GC'd
        // and the source closure with them.
        self.inner.borrow_mut().take();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Register the element. Idempotent.
pub fn register() {
    if already_registered() {
        return;
    }
    TonkNotation::define("tonk-notation");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-notation").is_undefined()
}

/// Reconcile observers with the current DOM: locate the source
/// `<script>`, rebind the source observer if the script element
/// changed, and re-render once. Called from `connected_callback`
/// and from the host observer (which fires only on child-list
/// changes — never on the `<pre>` insertions `render` makes).
fn sync(host: &Element, state: &Rc<RefCell<Inner>>) {
    let script = source_script(host);

    // Rebind the source observer only when the script *element*
    // changes — text-only edits don't reach here (they fire the
    // source observer instead).
    {
        let mut inner = state.borrow_mut();
        let same = match (inner.source_el.as_ref(), script.as_ref()) {
            (Some(a), Some(b)) => a.is_same_node(Some(b.as_ref())),
            (None, None) => true,
            _ => false,
        };
        if !same {
            inner.source_observer = None;
            inner.source_el = script.clone();
            if let Some(script_el) = script.as_ref() {
                let observer =
                    MutationObserver::new(inner._source_closure.as_ref().unchecked_ref())
                        .expect("MutationObserver::new(source)");
                let opts = MutationObserverInit::new();
                opts.set_character_data(true);
                opts.set_child_list(true);
                opts.set_subtree(true);
                let _ = observer.observe_with_options(script_el.as_ref(), &opts);
                inner.source_observer = Some(observer);
            }
        }
    }

    render(host);
}

/// Find the source `<script>`, read its text, and (re)mount the
/// rendered `<pre>` as its sibling. Skips when no source script
/// is present.
///
/// `render` mutates the host's child list (removing the previous
/// `<pre>`, appending the new one). The host observer will
/// re-fire as a result; its early-return guard (compares the
/// current source script element against the cached one) is what
/// stops the loop. Don't change either side of that without
/// updating the other.
fn render(host: &Element) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };

    // Drop any prior rendered output. Iterate from the end so
    // removing elements doesn't shift indices we're about to
    // read.
    if let Ok(list) = host.query_selector_all(&format!(".{}", PRE_CLASS)) {
        for i in (0..list.length()).rev() {
            if let Some(node) = list.item(i) {
                let _: Result<web_sys::Node, _> = host.remove_child(&node);
            }
        }
    }

    let Some(text) = source_text(host) else {
        return;
    };
    let marks = collect_marks(&text);
    if let Some(pre) = build_pre(&document, &text, &marks) {
        let _: Result<web_sys::Node, _> = host.append_child(&pre);
    }
}

/// Return the source `<script>` child if present.
fn source_script(host: &Element) -> Option<Element> {
    host.query_selector(&format!("script[type=\"{}\"]", SOURCE_MIME))
        .ok()
        .flatten()
}

/// Read the contents of the source `<script type="text/tonk-notation">`.
/// Returns `None` if no such script is present.
fn source_text(host: &Element) -> Option<String> {
    let script = host
        .query_selector(&format!("script[type=\"{}\"]", SOURCE_MIME))
        .ok()
        .flatten()?;
    script.text_content()
}

fn build_pre(document: &Document, text: &str, marks: &[Mark]) -> Option<Element> {
    let pre = document.create_element("pre").ok()?;
    let _ = pre.set_attribute("class", PRE_CLASS);
    let mut cursor = 0usize;
    for mark in marks {
        if mark.from > cursor {
            append_span(document, &pre, Decoration::Plain, &text[cursor..mark.from]);
        }
        append_span(document, &pre, mark.decoration, &text[mark.from..mark.to]);
        cursor = mark.to;
    }
    if cursor < text.len() {
        append_span(document, &pre, Decoration::Plain, &text[cursor..]);
    }
    Some(pre)
}

fn append_span(document: &Document, parent: &Element, decoration: Decoration, text: &str) {
    if text.is_empty() {
        return;
    }
    let Ok(span) = document.create_element("span") else {
        return;
    };
    if let Some(class) = decoration.class() {
        let _ = span.set_attribute("class", class);
    }
    span.set_text_content(Some(text));
    let _ = parent.append_child(&span);
}
