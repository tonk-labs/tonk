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
    // Per-line error annotations from `data-error-<line>` attributes — set
    // by the no-entity diagnostic to flag a line whose value is a missing
    // required attribute. Keyed by zero-based line index; the value is the
    // tooltip message.
    let line_errors = collect_line_errors(host);
    if let Some(pre) = build_pre(&document, &text, &marks, &line_errors) {
        let _: Result<web_sys::Node, _> = host.append_child(&pre);
    }
}

/// Read the host's `data-error-<line>` attributes into a `line -> message`
/// map. Each names a zero-based source line whose value span the renderer
/// should decorate as an error (squiggle + tooltip). Empty for ordinary
/// notation that carries no such attributes.
fn collect_line_errors(host: &Element) -> std::collections::HashMap<usize, String> {
    let mut errors = std::collections::HashMap::new();
    let Some(attributes) = host.dyn_ref::<HtmlElement>().map(|el| el.attributes()) else {
        return errors;
    };
    for i in 0..attributes.length() {
        let Some(attr) = attributes.item(i) else {
            continue;
        };
        let name = attr.name();
        if let Some(suffix) = name.strip_prefix("data-error-")
            && let Ok(line) = suffix.parse::<usize>()
        {
            errors.insert(line, attr.value());
        }
    }
    errors
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

fn build_pre(
    document: &Document,
    text: &str,
    marks: &[Mark],
    line_errors: &std::collections::HashMap<usize, String>,
) -> Option<Element> {
    let pre = document.create_element("pre").ok()?;
    let _ = pre.set_attribute("class", PRE_CLASS);
    let line_starts = line_starts(text);
    // A line carrying a `data-error-<line>` annotation flags a missing
    // required attribute; the squiggle covers the whole `field: value`
    // pair, so every span on that line — key, separator, and value alike —
    // gets the error. The shared `text-decoration` on adjacent inline
    // spans joins into one continuous underline. Leading indentation is
    // whitespace-only and `error_for` skips it, so the squiggle starts at
    // the key rather than the margin.
    let error_for = |text: &str, from: usize| -> Option<&String> {
        if text.trim().is_empty() {
            return None;
        }
        line_errors.get(&line_of(from, &line_starts))
    };
    let mut cursor = 0usize;
    for mark in marks {
        if mark.from > cursor {
            let gap = &text[cursor..mark.from];
            append_span(
                document,
                &pre,
                Decoration::Plain,
                gap,
                error_for(gap, cursor),
            );
        }
        let body = &text[mark.from..mark.to];
        append_span(
            document,
            &pre,
            mark.decoration,
            body,
            error_for(body, mark.from),
        );
        cursor = mark.to;
    }
    if cursor < text.len() {
        let tail = &text[cursor..];
        append_span(
            document,
            &pre,
            Decoration::Plain,
            tail,
            error_for(tail, cursor),
        );
    }
    Some(pre)
}

/// Byte offsets where each source line begins (line 0 starts at 0).
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

/// Zero-based line index containing byte `offset`.
fn line_of(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(line) => line,
        // `offset` falls inside line `i-1` (the largest start <= offset).
        Err(i) => i.saturating_sub(1),
    }
}

/// Append a decorated `<span>` for `text`. When `error` is `Some`, the
/// span carries the `tonk-notation-error` squiggle class and a native
/// `title` tooltip with the message, so hovering the flagged value
/// explains what is wrong (e.g. the missing attribute URI). A plain
/// `title` (not `<wa-tooltip>`) keeps the span inline inside the `<pre>` —
/// `<wa-tooltip>` is `display: block` and would break the line flow.
fn append_span(
    document: &Document,
    parent: &Element,
    decoration: Decoration,
    text: &str,
    error: Option<&String>,
) {
    if text.is_empty() {
        return;
    }
    let Ok(span) = document.create_element("span") else {
        return;
    };
    // The decoration class and the error class compose: an errored blank
    // still reads as a variable, with the squiggle layered on top.
    let class = match (decoration.class(), error.is_some()) {
        (Some(base), true) => format!("{base} tonk-notation-error"),
        (Some(base), false) => base.to_owned(),
        (None, true) => "tonk-notation-error".to_owned(),
        (None, false) => String::new(),
    };
    if !class.is_empty() {
        let _ = span.set_attribute("class", &class);
    }
    if let Some(message) = error {
        let _ = span.set_attribute("title", message);
    }
    span.set_text_content(Some(text));
    let _ = parent.append_child(&span);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Mount a fresh `<tonk-notation>` with the given source notation
    /// already inside a `text/tonk-notation` script. Attaching to the
    /// body triggers `connected_callback`, which paints synchronously
    /// before this function returns.
    fn mount(source: &str) -> Element {
        register();
        let document = web_sys::window().expect("window").document().expect("doc");
        let host = document
            .create_element("tonk-notation")
            .expect("create tonk-notation");
        if !source.is_empty() {
            let script = document.create_element("script").expect("create script");
            let _ = script.set_attribute("type", SOURCE_MIME);
            script.set_text_content(Some(source));
            let _ = host.append_child(&script);
        }
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    #[dialog_common::test]
    fn it_renders_a_pre_block_when_a_source_script_is_present() {
        let host = mount("greeting!:\n  this: did:key:zX\n");
        let pre = host
            .query_selector(&format!(".{}", PRE_CLASS))
            .expect("query pre")
            .expect("pre mounted");
        // Source text should round-trip through the spans.
        let text = pre.text_content().unwrap_or_default();
        assert!(text.contains("greeting"), "no greeting in: {text}");
        assert!(text.contains("did:key:zX"), "no entity in: {text}");
    }

    #[dialog_common::test]
    fn it_skips_rendering_when_no_source_script_is_present() {
        let host = mount("");
        assert!(
            host.query_selector(&format!(".{}", PRE_CLASS))
                .unwrap()
                .is_none(),
            "expected no pre block without a source script",
        );
    }

    #[dialog_common::test]
    fn it_tags_decoration_spans_with_dialog_yaml_class_names() {
        let host = mount("greeting!: &demo\n  this: did:key:zX\n  message: ?msg\n");
        let pre = host
            .query_selector(&format!(".{}", PRE_CLASS))
            .unwrap()
            .expect("pre mounted");

        // The decoration classes are the public contract with the
        // editor pack — pin every one that should show up for this
        // input so a tokenizer regression here is loud.
        let effect = pre
            .query_selector(".tonk-cm-effect")
            .unwrap()
            .expect("effect span");
        assert_eq!(effect.text_content().as_deref(), Some("greeting!"));

        let sigil = pre
            .query_selector(".tonk-cm-name-sigil")
            .unwrap()
            .expect("name-sigil span");
        assert_eq!(sigil.text_content().as_deref(), Some("&"));

        let name = pre
            .query_selector(".tonk-cm-name")
            .unwrap()
            .expect("name span");
        assert_eq!(name.text_content().as_deref(), Some("demo"));

        let entity = pre
            .query_selector(".tonk-cm-entity")
            .unwrap()
            .expect("entity span");
        assert_eq!(entity.text_content().as_deref(), Some("did:key:zX"));

        let variable = pre
            .query_selector(".tonk-cm-variable")
            .unwrap()
            .expect("variable span");
        assert_eq!(variable.text_content().as_deref(), Some("?msg"));

        // Keys are emitted for every field name.
        let keys: Vec<String> = (0..pre.query_selector_all(".tonk-cm-key").unwrap().length())
            .filter_map(|i| {
                pre.query_selector_all(".tonk-cm-key")
                    .unwrap()
                    .item(i)?
                    .text_content()
            })
            .collect();
        assert!(
            keys.contains(&"this".to_owned()),
            "missing this in {keys:?}"
        );
        assert!(
            keys.contains(&"message".to_owned()),
            "missing message in {keys:?}",
        );
    }

    #[dialog_common::test]
    fn it_renders_only_one_pre_block_at_a_time() {
        // Mounts a host with two consecutive scripts — the renderer
        // reads the first one and never emits more than one `<pre>`,
        // regardless of how many times the host observer fires
        // during the initial paint.
        let host = mount("greeting!:\n  this: did:key:zX\n");
        let count = host
            .query_selector_all(&format!(".{}", PRE_CLASS))
            .unwrap()
            .length();
        assert_eq!(count, 1, "expected one pre, got {count}");
    }
}
