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

use std::cell::Cell as Flag;
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
                 <pre class=\"notation-html-source\">{}</pre></div>",
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
.notation-error { color: var(--wa-color-danger-on-quiet, #b42318); }
.notation-stages { display: flex; flex-direction: column; gap: .1rem; margin-top: .25rem; }
.notation-stage summary { cursor: pointer; font-size: .75rem; }
.notation-stage-note { opacity: .6; }
.notation-html-rendered { border: 1px dashed rgba(127,127,127,.5); padding: .5rem; margin: .25rem 0; }
"#;
