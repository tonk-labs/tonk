//! Rendering a cell's result the way a notebook wants it.
//!
//! The inspector's [`crate::render`] targets a full-height panel: it dumps
//! every match as notation text, unbounded. In a notebook that is wrong twice
//! over — the output sits between two paragraphs of prose, so it must be
//! compact and it must not push the next block off the screen. A query
//! returning 116 concepts should not produce 116 screens of YAML.
//!
//! So this renders an Observable-style output instead:
//!
//! - **A summary line** naming what came back, always. That alone is often
//!   the whole answer ("42 results") and it is one line tall.
//! - **A gallery of cards**, one per result, capped. Each card names its
//!   entity and its fields, and a card is small enough that a dozen fit in
//!   the space the notation dump gave to one.
//! - **`<tonk-display>` per card**, so a result is drawn by its concept's
//!   own view when the space defines one, and by the display's notation
//!   fallback when it does not. The model comes from the query's label, so
//!   this is the ordinary case rather than a special one — and a concept
//!   that gains a view later starts using it without a change here.
//! - **Nothing at all for an empty result** beyond the summary, so a cell
//!   that matches nothing stays one line.
//!
//! The cap matters more than it looks: an uncapped gallery is the same
//! failure as the notation dump, just prettier. What is dropped is stated in
//! the summary rather than silently truncated.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::render::{
    COMMAND_LABEL, CONCEPT_LABEL, RULE_LABEL, concept_descriptor, rule_definition,
};
use crate::response::{EvaluateResponse, QueryMatchBlock, QueryResult};

/// How many result cards to render before summarising the rest.
///
/// Enough to see the shape of a result set, few enough that the cell stays
/// shorter than the prose around it.
const CARD_CAP: usize = 12;

/// Escape text for interpolation into HTML.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render a cell's output: a failure, or a response, or nothing.
///
/// `with` is the notebook's own routing context (`branch@repo`), stamped
/// onto every `<tonk-display>` a card mounts. It has to be passed down
/// rather than inherited: results render INSIDE prose's shadow root, and
/// context resolution reads an element's own `with` rather than walking
/// ancestors — so a card that is not stamped resolves no repository, and
/// whatever its view mounts renders "no repository in context".
pub fn render(failure: Option<&str>, response: Option<&EvaluateResponse>, with: &str) -> String {
    if let Some(message) = failure {
        return format!(
            "<div class=\"nb-out nb-out--error\"><span class=\"nb-out__icon\">!</span>\
               <span class=\"nb-out__text\">{}</span></div>",
            esc(message)
        );
    }
    let Some(response) = response else {
        return String::new();
    };

    // The blocks a commit produced, else the ones it read.
    let blocks = if response.matches_after.is_empty() {
        &response.matches_before
    } else {
        &response.matches_after
    };
    if blocks.is_empty() {
        return String::new();
    }
    blocks
        .iter()
        .map(|block| render_block(block, with))
        .collect()
}

/// One query's results: a summary line plus a capped gallery.
fn render_block(block: &QueryMatchBlock, with: &str) -> String {
    let count = block.results.len();
    let summary = format!(
        "<div class=\"nb-out__summary\">\
           <span class=\"nb-out__label\">{}</span>\
           <span class=\"nb-out__count\">{}</span>\
         </div>",
        esc(&block.label),
        match count {
            0 => "no results".to_owned(),
            1 => "1 result".to_owned(),
            n => format!("{n} results"),
        }
    );
    if count == 0 {
        return format!("<div class=\"nb-out\">{summary}</div>");
    }

    let cards: Vec<String> = block
        .results
        .iter()
        .map(|result| render_card(result, &block.label, with))
        .collect();
    let (shown, rest) = cards.split_at(count.min(CARD_CAP));
    // The rest folds away, Observable-style: one line saying how many,
    // and a click opens them in place. Native `<details>`, so it needs no
    // script and keeps its state while the cell is not re-run.
    let more = if rest.is_empty() {
        String::new()
    } else {
        format!(
            "<details class=\"nb-out__rest\">\
               <summary class=\"nb-out__more\">{} more</summary>\
               <div class=\"nb-out__gallery\">{}</div>\
             </details>",
            rest.len(),
            rest.concat()
        )
    };
    format!(
        "<div class=\"nb-out\">{summary}<div class=\"nb-out__gallery\">{}</div>{more}</div>",
        shown.concat()
    )
}

/// One result as a card.
///
/// A result whose model has a view goes to a `<tonk-display>`, which draws
/// it with that view. Everything else is shown as the notation that would
/// assert it, and the schema heads (`concept:`, `command:`, `rule:`) as
/// their definitions: text you can read, copy and evaluate, rather than a
/// tile of field names over `{2}`.
///
/// The model comes from the query's own label (`person ?alice:` → `person`)
/// when the result does not name one itself, so an ordinary query gets its
/// concept's view rather than the generic listing.
fn render_card(result: &QueryResult, label: &str, with: &str) -> String {
    // A transient result (a command) is in the response but was never
    // written to the branch, so a `<tonk-display>` re-reading it would
    // find no fields and report a missing attribute on a command that
    // ran. It is drawn from the response's own fields instead.
    let model = if result.transient {
        None
    } else {
        model_of(&result.fields).or_else(|| model_from_label(label))
    };
    if let Some(model) = model {
        // A header names the row: its `name` when it has one, and always
        // the entity itself, so among many results each says which it is
        // (and the id can be copied into the next query).
        let named = result
            .fields
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| format!("<span class=\"nb-card__name\">{}</span>", esc(name)))
            .unwrap_or_default();
        return format!(
            "<div class=\"nb-card nb-card--display\">\
               <div class=\"nb-card__title\">{named}<span class=\"nb-card__entity\">{}</span></div>\
               <tonk-display entity=\"{}\" model=\"{}\" with=\"{}\"></tonk-display>\
             </div>",
            esc(&tonk_notation::spell_uri(&result.this)),
            esc(&result.this),
            esc(&model),
            esc(with)
        );
    }
    format!(
        "<div class=\"nb-card nb-card--notation\">\
           <tonk-notation><script type=\"text/tonk-notation\">{}</script></tonk-notation>\
         </div>",
        script_text(&notation(result, label))
    )
}

/// A result as the notation that asserts it: `head!:`, its `this:`, then
/// its fields. A schema row (`concept:`, `command:`, `rule:`) is written
/// as its definition, rebuilt from the stored descriptor, so what the cell
/// shows can be copied into another cell and evaluated.
fn notation(result: &QueryResult, label: &str) -> String {
    let label = label.trim();
    let fields: Vec<(String, Value)> = match label {
        CONCEPT_LABEL | COMMAND_LABEL => concept_descriptor(result, label == CONCEPT_LABEL)
            .map(|map| map.into_iter().collect())
            .unwrap_or_else(|| plain_fields(result)),
        RULE_LABEL => rule_definition(result)
            .map(|map| map.into_iter().collect())
            .unwrap_or_else(|| plain_fields(result)),
        _ => plain_fields(result),
    };
    let head = if label.is_empty() { "_" } else { label };
    let mut out = format!(
        "{head}!:\n  this: {}\n",
        tonk_notation::spell_uri(&result.this)
    );
    for (name, value) in &fields {
        write_entry(&mut out, name, value, 2);
    }
    out.truncate(out.trim_end().len());
    out
}

/// The result's own fields, minus `this`, which the head already names.
fn plain_fields(result: &QueryResult) -> Vec<(String, Value)> {
    result
        .fields
        .iter()
        .filter(|(name, _)| name.as_str() != "this")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// One `key: value` line at `indent`; a nested mapping or sequence goes on
/// the lines below it, one entry per line, the way it would be typed.
fn write_entry(out: &mut String, key: &str, value: &Value, indent: usize) {
    out.push_str(&" ".repeat(indent));
    write_key(out, key);
    write_after_key(out, key, value, indent);
}

fn write_after_key(out: &mut String, key: &str, value: &Value, indent: usize) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push('\n');
            for (name, value) in map {
                write_entry(out, name, value, indent + 2);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            out.push('\n');
            for item in items {
                out.push_str(&" ".repeat(indent + 2));
                out.push('-');
                match item {
                    Value::Object(map) if !map.is_empty() => {
                        for (index, (name, value)) in map.iter().enumerate() {
                            if index == 0 {
                                out.push(' ');
                                write_key(out, name);
                                write_after_key(out, name, value, indent + 4);
                            } else {
                                write_entry(out, name, value, indent + 4);
                            }
                        }
                    }
                    other => write_after_key(out, key, other, indent + 2),
                }
            }
        }
        // Text with a line break or a `"` is a literal block scalar, so
        // neither has to be escaped: `|` keeps the one trailing newline,
        // `|-` says there is none. A URI or variable never needs it.
        Value::String(text)
            if text.contains('\n') || (text.contains('"') && !looks_like_uri(text)) =>
        {
            let (chomp, body) = match text.strip_suffix('\n') {
                Some(body) => ("|", body),
                None => ("|-", text.as_str()),
            };
            out.push_str(&format!(" {chomp}\n"));
            for line in body.split('\n') {
                if !line.is_empty() {
                    out.push_str(&" ".repeat(indent + 2));
                    out.push_str(line);
                }
                out.push('\n');
            }
        }
        other => {
            out.push(' ');
            out.push_str(&scalar(key, other));
            out.push('\n');
        }
    }
}

/// A mapping key: bare when it reads as one, quoted otherwise.
fn write_key(out: &mut String, key: &str) {
    let bare = !key.is_empty()
        && !key.starts_with(['-', '?'])
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '!' | '?'));
    if bare {
        out.push_str(key);
    } else {
        out.push_str(&serde_json::to_string(key).unwrap_or_default());
    }
    out.push(':');
}

/// A single-line value as notation: an entity bare, a type anchor under
/// `as` bare, other text quoted so it parses back as text.
fn scalar(key: &str, value: &Value) -> String {
    match value {
        Value::Null => "_".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Object(_) => "{}".to_owned(),
        Value::Array(_) => "[]".to_owned(),
        Value::String(text) => {
            let anchor = key == "as"
                && !text.is_empty()
                && text.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '/' | '-')
                });
            if anchor || (text.starts_with('?') && !text.contains(char::is_whitespace)) {
                text.clone()
            } else if looks_like_uri(text) {
                tonk_notation::spell_uri(text).into_owned()
            } else {
                serde_json::to_string(text).unwrap_or_default()
            }
        }
    }
}

/// A value to write bare as an entity: no whitespace, and either a scheme
/// (`did:key:…`, `id:foo`) or a reverse-dotted attribute id
/// (`xyz.tonk.view/greeting`). The same rule `<tonk-display>`'s notation
/// fallback uses, so the two read alike.
fn looks_like_uri(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && (text.contains(':') || (text.contains('.') && text.contains('/')))
}

/// Notation as the text of a `<script>`: nothing in it is decoded, so
/// only a closing tag needs breaking up.
fn script_text(text: &str) -> String {
    text.replace("</script", "<\\/script")
}

/// The model a query's label names, when it is one a view could be defined
/// for. A label is the source expression's head (`person ?alice:` →
/// `person`); the built-in meta heads are excluded because their rows are
/// schema, not domain data, and a `<tonk-display>` for them would resolve
/// nothing and render its own dump twice over.
fn model_from_label(label: &str) -> Option<String> {
    const META: [&str; 6] = ["concept", "attribute", "command", "rule", "name", "view"];
    let label = label.trim();
    if label.is_empty() || META.contains(&label) {
        return None;
    }
    Some(label.to_owned())
}

/// The `model` a result names, if it names one — the cue that a
/// `<tonk-display>` can draw it rather than this listing its fields.
fn model_of(fields: &BTreeMap<String, serde_json::Value>) -> Option<String> {
    let value = fields.get("model")?;
    let model = value.as_str()?;
    if model.is_empty() {
        return None;
    }
    Some(model.to_owned())
}

#[cfg(test)]
mod tests {
    // These tests run in the browser: the wasm test runner
    // otherwise looks for Node.js, which CI's web leg has not
    // got. The directive is crate-global, but declaring it per
    // module keeps it from vanishing when one module goes.
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;
    use serde_json::json;

    fn result(this: &str, fields: &[(&str, serde_json::Value)]) -> QueryResult {
        QueryResult {
            this: this.to_owned(),
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            transient: false,
        }
    }

    /// A command's entity is returned but never stored, so mounting a
    /// `<tonk-display>` for it re-reads nothing and shows "Concept
    /// mismatch: required attribute missing". It must be drawn from the
    /// response fields instead, with the value the command carried.
    #[dialog_common::test]
    fn it_draws_a_transient_result_from_its_fields_not_the_store() {
        let mut command = result(
            "did:key:z6MkCommand",
            &[("subject", json!("did:key:z6MkCounter"))],
        );
        command.transient = true;
        let html = render_card(&command, "counter/+1", "main@id:repo");
        assert!(
            !html.contains("<tonk-display"),
            "a transient result must not be re-read from the store: {html}"
        );
        assert!(
            html.contains("subject") && html.contains("did:key:z6MkCounter"),
            "the command's field is drawn from the response: {html}"
        );
    }

    /// The counterpart: a persisted result still goes through its
    /// concept's view, so skipping the display is not the new default.
    #[dialog_common::test]
    fn it_still_mounts_a_display_for_a_persisted_result() {
        let counter = result("did:key:z6MkCounter", &[("count", json!("+0"))]);
        let html = render_card(&counter, "counter/model", "main@id:repo");
        assert!(html.contains("<tonk-display"), "persisted result: {html}");
    }

    #[dialog_common::test]
    fn it_renders_nothing_without_a_response() {
        assert_eq!(render(None, None, "main@id:repo"), "");
    }

    #[dialog_common::test]
    fn it_summarises_an_empty_result_in_one_line() {
        let block = QueryMatchBlock {
            graph: None,
            label: "person".to_owned(),
            results: Vec::new(),
        };
        let html = render_block(&block, "main@id:repo");
        assert!(html.contains("no results"));
        assert!(!html.contains("nb-out__gallery"), "no gallery for nothing");
    }

    /// A large result set shows its first cards and folds the rest behind
    /// one line that opens them in place, rather than dropping them.
    #[dialog_common::test]
    fn it_folds_the_results_past_the_cap() {
        let results: Vec<QueryResult> = (0..40)
            .map(|n| result(&format!("id:thing/{n}"), &[("name", json!("x"))]))
            .collect();
        let block = QueryMatchBlock {
            graph: None,
            label: "thing".to_owned(),
            results,
        };
        let html = render_block(&block, "main@id:repo");
        let (shown, folded) = html.split_once("<details").expect("a fold");
        assert_eq!(shown.matches("class=\"nb-card ").count(), CARD_CAP);
        assert_eq!(folded.matches("class=\"nb-card ").count(), 28);
        assert!(folded.contains("28 more</summary>"), "{folded}");
        assert!(html.contains("40 results"));
    }

    /// No fold when everything fits.
    #[dialog_common::test]
    fn it_does_not_fold_a_short_result() {
        let block = QueryMatchBlock {
            graph: None,
            label: "thing".to_owned(),
            results: vec![result("id:thing/1", &[])],
        };
        assert!(!render_block(&block, "main@id:repo").contains("<details"));
    }

    /// A result naming a model is drawn by the space's own view.
    #[dialog_common::test]
    fn it_hands_a_modelled_result_to_a_display() {
        let html = render_card(
            &result("id:notebook/scratch", &[("model", json!("tonk:notebook"))]),
            "concept",
            "main@id:repo",
        );
        assert!(html.contains("<tonk-display"));
        assert!(html.contains("entity=\"id:notebook/scratch\""));
        assert!(html.contains("model=\"tonk:notebook\""));
    }

    /// A `concept:` row is shown as the concept's definition, a
    /// `concept!:` you can copy and evaluate, not a tile listing
    /// `source {2}` and `transient false`.
    #[dialog_common::test]
    fn it_shows_a_concept_as_its_definition() {
        let html = render_card(
            &result(
                "db:person",
                &[
                    (
                        "source",
                        json!(
                            r#"{"description":"A person: someone.","with":{"name":{"the":"xyz.person/name","as":"Text"}}}"#
                        ),
                    ),
                    ("transient", json!(false)),
                ],
            ),
            "concept",
            "main@id:repo",
        );
        assert!(!html.contains("<tonk-display"));
        assert!(html.contains("<tonk-notation>"), "{html}");
        assert!(html.contains("concept!:\n  this: db:person\n"), "{html}");
        assert!(html.contains("  with:\n    name:\n"), "{html}");
        // Key order follows the stored descriptor when `serde_json` keeps
        // insertion order, so each line is checked on its own.
        assert!(html.contains("\n      as: text"), "{html}");
        assert!(html.contains("\n      the: xyz.person/name"), "{html}");
        assert!(
            html.contains("description: \"A person: someone.\""),
            "a string is quoted, so the copy parses: {html}"
        );
        assert!(!html.contains("{2}"), "{html}");
    }

    /// A result with no view to draw it is the notation that asserts it,
    /// nested values as indented lines rather than JSON.
    #[dialog_common::test]
    fn it_shows_a_viewless_result_as_notation() {
        let mut command = result(
            "did:key:z6MkCommand",
            &[("block", json!({"N1": "id:nb/1", "N5": "id:nb/2"}))],
        );
        command.transient = true;
        let html = render_card(&command, "notebook/blocks", "main@id:repo");
        assert!(
            html.contains("notebook/blocks!:\n  this: did:key:z6MkCommand\n  block:\n    N1: id:nb/1\n    N5: id:nb/2"),
            "nested values as block lines, not JSON: {html}"
        );
    }

    /// An ordinary query's label IS its model, so the result goes to a
    /// `<tonk-display>` — which draws the concept's view when one is
    /// defined and falls back to its own notation dump when none is.
    /// Listing the fields here instead would freeze the result as a field
    /// list even after someone defines a view for it.
    #[dialog_common::test]
    fn it_hands_a_result_to_a_display_by_its_query_label() {
        let html = render_card(
            &result("id:alice", &[("name", json!("Alice"))]),
            "person",
            "main@id:repo",
        );
        assert!(html.contains("<tonk-display"));
        assert!(html.contains("entity=\"id:alice\""));
        assert!(html.contains("model=\"person\""));
    }

    /// A card's display carries the notebook's routing context.
    ///
    /// Results render inside prose's shadow root, and context resolution
    /// reads an element's own `with` rather than walking ancestors — so an
    /// unstamped card resolves no repository and whatever its view mounts
    /// renders "no repository in context" instead of the entity.
    #[dialog_common::test]
    fn it_stamps_the_context_on_a_card_display() {
        let html = render_card(
            &result("id:alice", &[("name", json!("Alice"))]),
            "person",
            "main@id:repo",
        );
        assert!(html.contains("with=\"main@id:repo\""));
    }

    /// A result naming its own model keeps it: the field is more specific
    /// than the label the query happened to use.
    #[dialog_common::test]
    fn it_prefers_a_named_model_over_the_label() {
        let html = render_card(
            &result("id:x", &[("model", json!("tonk:notebook"))]),
            "person",
            "main@id:repo",
        );
        assert!(html.contains("model=\"tonk:notebook\""));
    }

    /// Every drawn result is headed by its entity, so among many results
    /// each says which it is, with its name when it has one.
    #[dialog_common::test]
    fn it_heads_a_drawn_result_with_its_entity() {
        let html = render_card(
            &result("id:alice", &[("name", json!("Alice"))]),
            "person",
            "main@id:repo",
        );
        assert!(html.contains("nb-card__name\">Alice<"), "{html}");
        assert!(html.contains("nb-card__entity\">id:alice<"), "{html}");
    }

    /// What is shown parses back as notation: copying a definition out of a
    /// cell and evaluating it is the point of showing it this way.
    #[dialog_common::test]
    fn it_writes_notation_that_parses() {
        let text = notation(
            &result(
                "db:person",
                &[(
                    "source",
                    json!(
                        r#"{"description":"Multi\nline: text","with":{"name":{"the":"xyz.person/name","as":"Text"}}}"#
                    ),
                )],
            ),
            "concept",
        );
        let parsed = tonk_notation::parse(&text);
        assert!(
            parsed.diagnostics.is_empty(),
            "{:?}\n{text}",
            parsed.diagnostics
        );
    }

    #[dialog_common::test]
    fn it_reports_a_failure_without_a_gallery() {
        let html = render(Some("boom"), None, "main@id:repo");
        assert!(html.contains("nb-out--error"));
        assert!(html.contains("boom"));
    }
}
