//! Demo (notation tour): a ```notation fence in a `<tonk-notebook>` as a
//! cell the worker evaluates with the Dialog notation's own pipeline.
//!
//! A YAML cell (`dialog-yaml`) runs on every clean frame of its language
//! server. The notation has no language server in tonk, so a notation cell
//! runs when asked: its Run button, or Cmd/Ctrl+Enter (`<tonk-code>`'s
//! `run`). The cells above it in the notebook are its context: their
//! definitions are in scope, the way a later paragraph reads an earlier one.
//!
//! The worker answers with JSON of its own (`notation_tour.rs` in the
//! worker): what each top-level form did, and each stage of the pipeline
//! (read, expanded, analysis, compiled), which this module draws as a
//! result and a row of collapsible sections.
//!
//! Everything here is demo scaffolding for the notation tour; delete this
//! file, its `mod` line and the two hooks in `notebook.rs` to remove it.

use std::cell::{Cell as Flag, RefCell};
use std::rc::Rc;

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, window};

use crate::element::reflect_string;

/// Fence info words that make a notation cell.
pub(crate) const LANGUAGES: [&str; 2] = ["notation", "dialog-notation"];

/// The `<tonk-code>` language a notation cell is highlighted as: the
/// notation is written in s-expressions, and the editor ships Clojure's.
pub(crate) const HIGHLIGHT: &str = "clojure";

/// The LSP `languageId` a notation cell reports (`<tonk-code>`'s
/// `lsp-language`), so it registers with the notebook's language server
/// as what it is, not as the `clojure` it is highlighted as. The server
/// answers no diagnostics and no completions for it yet: those wait for
/// a dialog query that checks the notation.
pub(crate) const LSP_LANGUAGE: &str = "dialog-notation";

/// The next notation cell's LSP buffer number, global like the YAML
/// cells' (`NEXT_CELL` in `notebook.rs`): it names the buffer and nothing
/// else.
static NEXT_BUFFER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What a notation cell posts to `/evaluate`.
pub(crate) const CONTENT_TYPE: &str = "application/x-dialog-notation+json";

/// The marker a bound notation fence carries.
const MARKER: &str = "notationCell";

/// Whether a fence's language makes it a notation cell.
pub(crate) fn is_notation(language: &str) -> bool {
    LANGUAGES.contains(&language)
}

/// The worker's answer to one cell.
#[derive(Deserialize, Default)]
struct Answer {
    /// Each top-level form of the cell, and what it did.
    #[serde(default)]
    forms: Vec<FormOutcome>,
    /// The pipeline's stages over the cell.
    #[serde(default)]
    stages: Vec<Stage>,
    /// Why nothing ran, when the cell was refused whole.
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct FormOutcome {
    /// The form as written.
    #[serde(default)]
    form: String,
    /// What kind of form: `select`, `assert`, `rule`, `concept`, ...
    #[serde(default)]
    kind: String,
    /// One line on what it did.
    #[serde(default)]
    summary: String,
    /// The columns of the rows it answered, if it asked.
    #[serde(default)]
    columns: Vec<String>,
    /// The rows it answered, each a value per column.
    #[serde(default)]
    rows: Vec<Vec<String>>,
    /// The rows as the notation prints them, when they are not a table.
    #[serde(default)]
    printed: Vec<String>,
    /// Markup it rendered, for a view.
    #[serde(default)]
    html: Option<String>,
    /// Why the notation refused it.
    #[serde(default)]
    refused: Option<Refusal>,
    /// What the analyzer warns about it.
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Deserialize, Default)]
struct Refusal {
    message: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
    #[serde(default)]
    excerpt: Option<String>,
}

#[derive(Deserialize, Default)]
struct Stage {
    /// `read`, `expanded`, `analysis`, `compiled`.
    name: String,
    /// One line on what the stage is.
    #[serde(default)]
    note: String,
    /// What it gave, as text.
    #[serde(default)]
    text: String,
}

/// Bind one ```notation fence: a Run button, a result node, and the
/// `run` keystroke.
pub(crate) fn bind(
    root: &Element,
    wrapper: &HtmlElement,
    editor: &Element,
    with: &str,
    closures: &Rc<std::cell::RefCell<Vec<Closure<dyn FnMut(Event)>>>>,
) {
    if editor.get_attribute("language").as_deref() != Some(HIGHLIGHT) {
        let _ = editor.set_attribute("language", HIGHLIGHT);
    }
    if wrapper.dataset().get(MARKER).is_some() {
        return;
    }
    let _ = wrapper.dataset().set(MARKER, "");
    let _ = wrapper.set_attribute("data-cell-language", "notation");
    register(editor, with);
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(bar) = document.create_element("div") else {
        return;
    };
    bar.set_class_name("notation-cell-bar");
    bar.set_inner_html(
        "<wa-button type=\"button\" class=\"notation-cell-run\" size=\"small\" \
           variant=\"brand\" appearance=\"outlined\" \
           title=\"Run this notation cell (Cmd/Ctrl+Enter)\">\
           <wa-icon slot=\"start\" name=\"play\" variant=\"solid\"></wa-icon>Run\
         </wa-button>\
         <span class=\"notation-cell-state\"></span>",
    );
    let Ok(result) = document.create_element("div") else {
        return;
    };
    result.set_class_name("notation-cell-result");
    let _ = wrapper.append_child(&bar);
    let _ = wrapper.append_child(&result);

    let running = Rc::new(Flag::new(false));
    let run = {
        let root = root.clone();
        let wrapper = wrapper.clone();
        let editor = editor.clone();
        let result = result.clone();
        let bar = bar.clone();
        let with = with.to_owned();
        let running = running.clone();
        let held = closures.clone();
        move || {
            if running.get() {
                return;
            }
            let Some(source) = reflect_string(editor.as_ref(), "value") else {
                return;
            };
            let context = context_before(&root, &wrapper);
            let body = serde_json::json!({ "source": source, "context": context }).to_string();
            running.set(true);
            let _ = wrapper.set_attribute("data-cell-state", "running");
            set_state(&bar, "running…");
            let result = result.clone();
            let wrapper = wrapper.clone();
            let editor = editor.clone();
            let bar = bar.clone();
            let with = with.clone();
            let running = running.clone();
            let held = held.clone();
            spawn_local(async move {
                let started = js_sys::Date::now();
                let answered =
                    tonk_host::consumer::evaluate_as(&editor, &body, CONTENT_TYPE, true).await;
                let took = js_sys::Date::now() - started;
                match answered {
                    Ok(value) => {
                        let text = js_sys::JSON::stringify(&value)
                            .ok()
                            .and_then(|s| s.as_string())
                            .unwrap_or_default();
                        let answer: Answer =
                            serde_json::from_str(&text).unwrap_or_else(|e| Answer {
                                error: Some(format!("the worker's answer did not decode: {e}")),
                                ..Answer::default()
                            });
                        let refused = answer.error.is_some()
                            || answer.forms.iter().any(|form| form.refused.is_some());
                        result.set_inner_html(&render(&answer, &with));
                        wire_watch(&result, &editor, &body, &with, &held);
                        let state = if refused { "refused" } else { "ran" };
                        let _ = wrapper.set_attribute("data-cell-state", state);
                        set_state(&bar, &format!("{state} in {took:.0} ms"));
                    }
                    Err(error) => {
                        result.set_inner_html(&format!(
                            "<div class=\"notation-error\">{}</div>",
                            escape(&error.message)
                        ));
                        let _ = wrapper.set_attribute("data-cell-state", "failed");
                        set_state(&bar, "failed");
                    }
                }
                running.set(false);
            });
        }
    };
    let run = Rc::new(run);

    if let Ok(Some(button)) = bar.query_selector(".notation-cell-run") {
        let run = run.clone();
        let click = Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
            event.stop_propagation();
            run();
        }) as Box<dyn FnMut(Event)>);
        let _ = button.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        closures.borrow_mut().push(click);
    }
    let keyed = {
        let run = run.clone();
        Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
            event.stop_propagation();
            run();
        }) as Box<dyn FnMut(Event)>)
    };
    let _ = editor.add_event_listener_with_callback("run", keyed.as_ref().unchecked_ref());
    closures.borrow_mut().push(keyed);
}

fn set_state(bar: &Element, text: &str) {
    if let Ok(Some(state)) = bar.query_selector(".notation-cell-state") {
        state.set_text_content(Some(text));
    }
}

/// The sources of the notation cells above `wrapper`, in document order.
fn context_before(root: &Element, wrapper: &HtmlElement) -> Vec<String> {
    let mut context = Vec::new();
    let Ok(cells) = root.query_selector_all("[data-cell-language=notation]") else {
        return context;
    };
    for index in 0..cells.length() {
        let Some(cell) = cells
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        else {
            continue;
        };
        if cell.is_same_node(Some(wrapper)) {
            break;
        }
        if let Ok(Some(editor)) = cell.query_selector("tonk-code")
            && let Some(value) = reflect_string(editor.as_ref(), "value")
        {
            context.push(value);
        }
    }
    context
}

/// The answer as markup: each form's outcome, then the stages.
fn render(answer: &Answer, _with: &str) -> String {
    let mut out = String::new();
    if let Some(error) = &answer.error {
        out.push_str(&format!(
            "<div class=\"notation-error\" data-outcome=\"error\">{}</div>",
            escape(error)
        ));
    }
    for form in &answer.forms {
        out.push_str(&format!(
            "<div class=\"notation-form\" data-kind=\"{}\" data-outcome=\"{}\">",
            escape(&form.kind),
            if form.refused.is_some() {
                "refused"
            } else {
                "ok"
            }
        ));
        out.push_str(&format!(
            "<div class=\"notation-form-head\"><code class=\"notation-kind\">{}</code> {}</div>",
            escape(&form.kind),
            escape(&form.summary)
        ));
        for warning in &form.warnings {
            out.push_str(&format!(
                "<div class=\"notation-warning\"><strong>warning</strong> {}</div>",
                escape(warning)
            ));
        }
        if let Some(refused) = &form.refused {
            let place = match (refused.line, refused.column) {
                (Some(line), Some(column)) => format!(" at line {line}, column {column}"),
                (Some(line), None) => format!(" at line {line}"),
                _ => String::new(),
            };
            out.push_str(&format!(
                "<div class=\"notation-refusal\"><strong>refused{}</strong>: {}",
                escape(&place),
                escape(&refused.message)
            ));
            if let Some(excerpt) = &refused.excerpt {
                out.push_str(&format!("<pre>{}</pre>", escape(excerpt)));
            }
            out.push_str("</div>");
        }
        if !form.columns.is_empty() {
            out.push_str("<table class=\"notation-rows\"><thead><tr>");
            for column in &form.columns {
                out.push_str(&format!("<th>{}</th>", escape(column)));
            }
            out.push_str("</tr></thead><tbody>");
            for row in &form.rows {
                out.push_str("<tr>");
                for value in row {
                    out.push_str(&format!("<td>{}</td>", escape(value)));
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
        } else if !form.printed.is_empty() {
            out.push_str("<pre class=\"notation-printed\">");
            for line in &form.printed {
                out.push_str(&escape(line));
                out.push('\n');
            }
            out.push_str("</pre>");
        }
        if let Some(html) = &form.html {
            out.push_str(&format!(
                "<div class=\"notation-html\"><div class=\"notation-html-rendered\">{html}</div>\
                 <pre class=\"notation-html-source\">{}</pre>\
                 <wa-button type=\"button\" class=\"notation-watch\" size=\"small\" \
                   appearance=\"outlined\" title=\"Keep this view current: the worker \
                   streams the driver's patches as the space changes\">Watch live</wa-button></div>",
                escape(html)
            ));
        }
        out.push_str("</div>");
    }
    if !answer.stages.is_empty() {
        out.push_str("<div class=\"notation-stages\">");
        for stage in &answer.stages {
            out.push_str(&format!(
                "<details class=\"notation-stage\" data-stage=\"{}\"><summary>{} \
                 <span class=\"notation-stage-note\">{}</span></summary><pre>{}</pre></details>",
                escape(&stage.name),
                escape(&stage.name),
                escape(&stage.note),
                escape(&stage.text)
            ));
        }
        out.push_str("</div>");
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Styles for notation cells, injected beside the notebook's own.
pub(crate) const CSS: &str = r#"
.notation-cell-bar { display: flex; align-items: center; gap: .5rem; margin: .25rem 0; }
.notation-cell-state { font-size: .75rem; opacity: .7; }
.notation-cell-result { font-size: .85rem; }
.notation-form { margin: .35rem 0; }
.notation-form-head { opacity: .85; }
.notation-kind { font-weight: 600; }
.notation-rows { border-collapse: collapse; margin: .25rem 0; }
.notation-rows th, .notation-rows td { border: 1px solid rgba(127,127,127,.35); padding: .15rem .5rem; text-align: left; }
.notation-refusal { color: var(--wa-color-danger-on-quiet, #b42318); }
.notation-refusal pre, .notation-stage pre, .notation-printed, .notation-html-source { white-space: pre-wrap; font-size: .75rem; margin: .25rem 0; }
.notation-warning { color: var(--wa-color-warning-on-quiet, #93370d); }
.notation-error { color: var(--wa-color-danger-on-quiet, #b42318); }
.notation-stages { display: flex; flex-direction: column; gap: .1rem; margin-top: .25rem; }
.notation-stage summary { cursor: pointer; font-size: .75rem; }
.notation-stage-note { opacity: .6; }
.notation-html-rendered [class="done"], .notation-live [class="done"] { text-decoration: line-through; opacity: .6; }
.notation-live { border: 1px solid var(--wa-color-brand-border-normal, #6a8); padding: .5rem; margin: .25rem 0; }
.notation-html-rendered { border: 1px dashed rgba(127,127,127,.5); padding: .5rem; margin: .25rem 0; }
"#;

thread_local! {
    /// The live views' subscriptions, held for the page's life.
    static LIVE: RefCell<Vec<tonk_host::consumer::Subscription>> = const { RefCell::new(Vec::new()) };
}

/// Wire the "Watch live" button of a view the cell rendered: it opens a
/// subscription on the branch's query route whose frames are the
/// driver's, and keeps a copy of the view current by its patches.
fn wire_watch(
    result: &Element,
    editor: &Element,
    body: &str,
    with: &str,
    closures: &Rc<std::cell::RefCell<Vec<Closure<dyn FnMut(Event)>>>>,
) {
    let Ok(Some(button)) = result.query_selector(".notation-watch") else {
        return;
    };
    let Ok(cell) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let live_body = serde_json::json!({ "notation_live": cell }).to_string();
    let with = with.to_owned();
    let editor = editor.clone();
    let host = button.parent_element();
    let click = Closure::wrap(Box::new(move |event: Event| {
        event.prevent_default();
        event.stop_propagation();
        let Some(host) = host.clone() else {
            return;
        };
        if host
            .query_selector(".notation-live")
            .ok()
            .flatten()
            .is_some()
        {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(live) = document.create_element("div") else {
            return;
        };
        live.set_class_name("notation-live");
        let _ = live.set_attribute("with", &with);
        let _ = live.set_attribute("data-frames", "0");
        let _ = live.set_attribute("data-patches", "0");
        let _ = live.set_inner_html("<em>watching…</em>");
        let _ = host.append_child(&live);
        install_frames(&live, &live_body);
        let Ok(query) = js_sys::JSON::parse(&live_body) else {
            return;
        };
        let _ = &editor;
        spawn_local(async move {
            let tag = JsValue::from_str("notation-live");
            match tonk_host::consumer::subscribe_claimed(&live, &query, Some(&tag)).await {
                Ok(subscription) => LIVE.with(|held| held.borrow_mut().push(subscription)),
                Err(error) => live.set_inner_html(&format!(
                    "<div class=\"notation-error\">{}</div>",
                    escape(&error.message)
                )),
            }
        });
    }) as Box<dyn FnMut(Event)>);
    let _ = button.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
    closures.borrow_mut().push(click);
}

/// The host delivers each frame to the consumer's `reset` (a frame with
/// a kind it does not know is delivered whole), and errors to `error`.
fn install_frames(live: &Element, live_body: &str) {
    install_events(live, live_body);
    let target = live.clone();
    let reset = Closure::wrap(Box::new(move |frame: JsValue, _opts: JsValue| {
        let text = js_sys::JSON::stringify(&frame)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_default();
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        apply_frame(&target, &frame);
    }) as Box<dyn FnMut(JsValue, JsValue)>);
    let target = live.clone();
    let error = Closure::wrap(Box::new(move |detail: JsValue, _opts: JsValue| {
        let message = js_sys::Reflect::get(&detail, &"message".into())
            .ok()
            .and_then(|m| m.as_string())
            .unwrap_or_else(|| "the live view failed".into());
        let _ = target.set_attribute("data-error", &message);
    }) as Box<dyn FnMut(JsValue, JsValue)>);
    let _ = js_sys::Reflect::set(live, &"reset".into(), reset.as_ref());
    let _ = js_sys::Reflect::set(live, &"update".into(), reset.as_ref());
    let _ = js_sys::Reflect::set(live, &"error".into(), error.as_ref());
    reset.forget();
    error.forget();
}

fn bump(live: &Element, name: &str, by: u64) {
    let now = live
        .get_attribute(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let _ = live.set_attribute(name, &(now + by).to_string());
}

/// Apply one frame of the driver: the first is the view whole, the rest
/// are patches, each applied where its address says.
fn apply_frame(live: &Element, frame: &serde_json::Value) {
    if let Some(error) = frame.get("error").and_then(|e| e.as_str()) {
        let _ = live.set_attribute("data-error", error);
        return;
    }
    if let Some(took) = frame.get("took_ms").and_then(|t| t.as_f64()) {
        let _ = live.set_attribute("data-last-ms", &format!("{took:.2}"));
    }
    if frame.get("first").and_then(|f| f.as_bool()) == Some(true) {
        live.set_inner_html(
            frame
                .get("html")
                .and_then(|h| h.as_str())
                .unwrap_or_default(),
        );
        bump(live, "data-frames", 1);
        return;
    }
    let Some(patches) = frame.get("patches").and_then(|p| p.as_array()) else {
        return;
    };
    let mut applied = 0;
    for patch in patches {
        if apply_patch(live, patch).is_some() {
            applied += 1;
        } else {
            let _ = live.set_attribute("data-missed", &patch.to_string());
        }
    }
    bump(live, "data-frames", 1);
    bump(live, "data-patches", applied);
}

/// The instance an address names, from the live view's root.
fn find(live: &Element, at: &serde_json::Value) -> Option<Element> {
    let mut current = live.clone();
    for step in at.as_array()? {
        let slot = step.get(0)?.as_u64()?.to_string();
        let key = step.get(1)?.as_str()?;
        let children = current.children();
        let mut next = None;
        for index in 0..children.length() {
            let child = children.item(index)?;
            if child.get_attribute("data-ns").as_deref() == Some(slot.as_str())
                && child.get_attribute("data-nk").as_deref() == Some(key)
            {
                next = Some(child);
                break;
            }
        }
        current = next?;
    }
    Some(current)
}

/// The comment that opens slot `marker` (`s2`, `t0`) of `element`.
fn marker(element: &Element, marker: &str) -> Option<web_sys::Node> {
    let nodes = element.child_nodes();
    for index in 0..nodes.length() {
        let node = nodes.item(index)?;
        if node.node_type() == web_sys::Node::COMMENT_NODE
            && node.node_value().as_deref() == Some(marker)
        {
            return Some(node);
        }
    }
    None
}

fn apply_patch(live: &Element, patch: &serde_json::Value) -> Option<()> {
    match patch.get("op")?.as_str()? {
        "insert" => {
            let parent = find(live, patch.get("parent")?)?;
            let slot = patch.get("slot")?.as_u64()?;
            let index = patch.get("index")?.as_u64()? as u32;
            let html = patch.get("html")?.as_str()?;
            let document = window()?.document()?;
            let holder = document.create_element("div").ok()?;
            holder.set_inner_html(html);
            let element: web_sys::Node = holder.first_element_child()?.into();
            // The instances already in the slot, in order.
            let children = parent.children();
            let mut siblings = Vec::new();
            for at in 0..children.length() {
                let child = children.item(at)?;
                if child.get_attribute("data-ns").as_deref() == Some(slot.to_string().as_str()) {
                    siblings.push(child);
                }
            }
            if let Some(before) = siblings.get(index as usize) {
                parent.insert_before(&element, Some(before)).ok()?;
            } else if let Some(last) = siblings.last() {
                parent
                    .insert_before(&element, last.next_sibling().as_ref())
                    .ok()?;
            } else if let Some(open) = marker(&parent, &format!("s{slot}")) {
                parent
                    .insert_before(&element, open.next_sibling().as_ref())
                    .ok()?;
            } else {
                parent.append_child(&element).ok()?;
            }
        }
        "remove" => find(live, patch.get("at")?)?.remove(),
        "text" => {
            let element = find(live, patch.get("at")?)?;
            let slot = patch.get("slot")?.as_u64()?;
            let text = patch.get("text")?.as_str()?;
            let open = marker(&element, &format!("t{slot}"))?;
            match open.next_sibling() {
                Some(node) if node.node_type() == web_sys::Node::TEXT_NODE => {
                    node.set_node_value(Some(text));
                }
                next => {
                    let node = window()?.document()?.create_text_node(text);
                    element.insert_before(&node, next.as_ref()).ok()?;
                }
            }
        }
        "attribute" => {
            let element = find(live, patch.get("at")?)?;
            element
                .set_attribute(patch.get("name")?.as_str()?, patch.get("value")?.as_str()?)
                .ok()?;
        }
        _ => return None,
    }
    Some(())
}

/// Events of the live view: a click (or change, or submit) on an
/// element the view bound an event on posts that event's command, its
/// fields filled from the row or from the live event, as a notation cell
/// whose context is the view cell's own. What it writes comes back to the
/// view as the driver's patches.
fn install_events(live: &Element, live_body: &str) {
    let Ok(body) = serde_json::from_str::<serde_json::Value>(live_body) else {
        return;
    };
    let cell = &body["notation_live"];
    let mut context: Vec<serde_json::Value> =
        cell["context"].as_array().cloned().unwrap_or_default();
    context.push(cell["source"].clone());
    for kind in ["click", "change", "submit"] {
        let root = live.clone();
        let context = context.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let Some(mut target) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            let attribute = format!("data-notation-on-{kind}");
            let bound = loop {
                if let Some(bound) = target.get_attribute(&attribute) {
                    break Some((target.clone(), bound));
                }
                if target.is_same_node(Some(&root)) {
                    break None;
                }
                match target.parent_element() {
                    Some(parent) => target = parent,
                    None => break None,
                }
            };
            let Some((element, bound)) = bound else {
                return;
            };
            if kind == "submit" {
                event.prevent_default();
            }
            let Ok(bound) = serde_json::from_str::<serde_json::Value>(&bound) else {
                return;
            };
            let Some(text) = command_text(&bound, &event, &element) else {
                return;
            };
            let _ = root.set_attribute("data-posted", &text);
            let request = serde_json::json!({
                "source": text,
                "context": context,
            })
            .to_string();
            let root = root.clone();
            spawn_local(async move {
                if let Err(error) =
                    tonk_host::consumer::evaluate_as(&root, &request, CONTENT_TYPE, true).await
                {
                    let _ = root.set_attribute("data-error", &error.message);
                }
            });
        }) as Box<dyn FnMut(Event)>);
        let _ = live.add_event_listener_with_callback_and_bool(
            kind,
            listener.as_ref().unchecked_ref(),
            true,
        );
        listener.forget();
    }
}

/// `(assert (command label: value ...))` for a bound event that fired.
fn command_text(bound: &serde_json::Value, event: &Event, element: &Element) -> Option<String> {
    let command = bound["command"].as_str()?;
    let mut out = format!("(assert ({command}");
    for field in bound["fields"].as_array()? {
        let label = field.get(0)?.as_str()?;
        let given = field.get(1)?;
        let value = if let Some(notation) = given["notation"].as_str() {
            notation.to_owned()
        } else {
            let steps = given["path"].as_array()?;
            let mut current: JsValue = event.clone().into();
            for (at, step) in steps.iter().enumerate() {
                let step = step.as_str()?;
                current = if at == 0 && step == "currentTarget" {
                    element.clone().into()
                } else {
                    js_sys::Reflect::get(&current, &JsValue::from_str(step)).ok()?
                };
            }
            match current.as_f64() {
                Some(number) => number.to_string(),
                None => {
                    serde_json::Value::String(current.as_string().unwrap_or_default()).to_string()
                }
            }
        };
        out.push_str(&format!(" {label}: {value}"));
    }
    out.push_str("))");
    Some(out)
}

/// Register a notation cell's editor with the notebook's language server:
/// a buffer of its own under `with` (`branch@repo`), reported as
/// [`LSP_LANGUAGE`], and announced to the provider the way the YAML cells
/// are, since the editor can be mounted before it is inside the provider.
fn register(editor: &Element, with: &str) {
    let (branch, repo) = with.split_once('@').unwrap_or((with, ""));
    let id = NEXT_BUFFER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let source = format!("tonk-buffer:///{repo}/{branch}/notation-{id}");
    // The language first: setting `source` is what announces the editor.
    let _ = editor.set_attribute("lsp-language", LSP_LANGUAGE);
    let _ = editor.set_attribute("source", &source);
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &"source".into(), &source.as_str().into());
    let _ = js_sys::Reflect::set(&detail, &"language".into(), &LSP_LANGUAGE.into());
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    init.set_bubbles(true);
    init.set_composed(true);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("tonk-code-connect", &init) {
        let _ = editor.dispatch_event(&event);
    }
}
