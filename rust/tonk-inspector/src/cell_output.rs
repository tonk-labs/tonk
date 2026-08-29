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

use crate::response::{EvaluateResponse, QueryMatchBlock, QueryResult};

/// How many result cards to render before summarising the rest.
///
/// Enough to see the shape of a result set, few enough that the cell stays
/// shorter than the prose around it.
const CARD_CAP: usize = 12;

/// How many fields to show on one card before eliding.
const FIELD_CAP: usize = 6;

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

    let shown: String = block
        .results
        .iter()
        .take(CARD_CAP)
        .map(|result| render_card(result, &block.label, with))
        .collect();
    // State what was dropped. A silently truncated gallery reads as the whole
    // answer, which is worse than a long one.
    let more = if count > CARD_CAP {
        format!(
            "<div class=\"nb-out__more\">and {} more</div>",
            count - CARD_CAP
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"nb-out\">{summary}<div class=\"nb-out__gallery\">{shown}</div>{more}</div>"
    )
}

/// One result as a card.
///
/// The entity goes to a `<tonk-display>`, which resolves the model's view
/// and — when nothing defines one — falls back to a notation dump of its
/// own. That fallback is the reason this does not list fields itself: the
/// display already knows how to present a result nothing has a view for,
/// and doing it here means a concept that LATER gains a view keeps
/// rendering as a field list.
///
/// The model comes from the query's own label (`person ?alice:` → `person`)
/// when the result does not name one itself, so an ordinary query gets its
/// concept's view rather than the generic listing.
fn render_card(result: &QueryResult, label: &str, with: &str) -> String {
    // Prefer a `name` over the entity URI: `db:attribute` and `attribute`
    // are the same row, and the readable one belongs in the title. The URI
    // stays as the tooltip, so nothing is lost.
    let named = result
        .fields
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let title = esc(named.unwrap_or_else(|| short_entity(&result.this)));
    if let Some(model) = model_of(&result.fields).or_else(|| model_from_label(label)) {
        return format!(
            "<div class=\"nb-card nb-card--display\">\
               <div class=\"nb-card__title\">{title}</div>\
               <tonk-display entity=\"{}\" model=\"{}\" with=\"{}\"></tonk-display>\
             </div>",
            esc(&result.this),
            esc(&model),
            esc(with)
        );
    }
    let fields: String = result
        .fields
        .iter()
        // `name` is the title now; repeating it as a row wastes a line on
        // every card.
        .filter(|(key, _)| key.as_str() != "name")
        .take(FIELD_CAP)
        .map(|(name, value)| {
            format!(
                "<div class=\"nb-card__field\">\
                   <span class=\"nb-card__key\">{}</span>\
                   <span class=\"nb-card__value\">{}</span>\
                 </div>",
                esc(name),
                esc(&scalar(value))
            )
        })
        .collect();
    let elided = if result.fields.len() > FIELD_CAP {
        format!(
            "<div class=\"nb-card__more\">+{}</div>",
            result.fields.len() - FIELD_CAP
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"nb-card\" title=\"{}\">\
           <div class=\"nb-card__title\">{title}</div>{fields}{elided}\
         </div>",
        esc(&result.this)
    )
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

/// A one-line rendering of a field value. Objects and arrays are summarised
/// rather than dumped: a card is a glance, not a document.
fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "—".to_owned(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            // A field whose value is itself serialized JSON (a concept's
            // `source`, for one) shows as `{"description":"…` — punctuation,
            // not information. Say what it is instead.
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                return match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(parsed) => scalar(&parsed),
                    Err(_) => "{…}".to_owned(),
                };
            }
            // Long prose would blow the card open.
            if trimmed.chars().count() > 60 {
                let head: String = trimmed.chars().take(59).collect();
                format!("{head}…")
            } else {
                trimmed.to_owned()
            }
        }
        serde_json::Value::Array(items) => match items.len() {
            0 => "[]".to_owned(),
            n => format!("[{n}]"),
        },
        serde_json::Value::Object(map) => match map.len() {
            0 => "{}".to_owned(),
            n => format!("{{{n}}}"),
        },
    }
}

/// The readable tail of an entity URI. `did:key:z6Mk…` and
/// `id:notebook/scratch` are both long and mostly prefix; a card wants the
/// part that distinguishes one result from the next.
fn short_entity(entity: &str) -> &str {
    if let Some(rest) = entity.rsplit('/').next()
        && !rest.is_empty()
        && rest != entity
    {
        return rest;
    }
    // A `did:key:` has no slash; show its tail rather than the scheme every
    // row shares.
    if entity.len() > 24
        && let Some(tail) = entity.get(entity.len() - 12..)
    {
        return tail;
    }
    entity
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
        }
    }

    #[dialog_common::test]
    fn it_renders_nothing_without_a_response() {
        assert_eq!(render(None, None, "main@id:repo"), "");
    }

    #[dialog_common::test]
    fn it_summarises_an_empty_result_in_one_line() {
        let block = QueryMatchBlock {
            label: "person".to_owned(),
            results: Vec::new(),
        };
        let html = render_block(&block, "main@id:repo");
        assert!(html.contains("no results"));
        assert!(!html.contains("nb-out__gallery"), "no gallery for nothing");
    }

    /// The failure the whole module exists to prevent: a large result set
    /// must not render a card per result.
    #[dialog_common::test]
    fn it_caps_the_gallery_and_says_what_it_dropped() {
        let results: Vec<QueryResult> = (0..40)
            .map(|n| result(&format!("id:thing/{n}"), &[("name", json!("x"))]))
            .collect();
        let block = QueryMatchBlock {
            label: "thing".to_owned(),
            results,
        };
        let html = render_block(&block, "main@id:repo");
        assert_eq!(html.matches("nb-card__title").count(), CARD_CAP);
        assert!(html.contains("and 28 more"));
        assert!(html.contains("40 results"));
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

    /// A meta head's rows are schema, not domain data: no view is defined
    /// for `concept` or `attribute`, so the card lists fields rather than
    /// mounting a display that would only dump them again.
    #[dialog_common::test]
    fn it_lists_fields_for_a_meta_head() {
        let html = render_card(
            &result("id:x", &[("name", json!("Alice"))]),
            "concept",
            "main@id:repo",
        );
        assert!(!html.contains("<tonk-display"));
        assert!(html.contains("Alice"));
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

    /// A description would otherwise blow the card open.
    #[dialog_common::test]
    fn it_truncates_a_long_string() {
        let long = "a".repeat(200);
        let rendered = scalar(&json!(long));
        assert!(rendered.chars().count() <= 80);
        assert!(rendered.ends_with('…'));
    }

    /// Nested values are summarised, not dumped.
    #[dialog_common::test]
    fn it_summarises_containers() {
        assert_eq!(scalar(&json!([1, 2, 3])), "[3]");
        assert_eq!(scalar(&json!({"a": 1})), "{1}");
        assert_eq!(scalar(&json!(null)), "—");
    }

    /// `db:attribute` and `attribute` are the same row; the readable one
    /// belongs in the title, and repeating it as a field wastes a line.
    #[dialog_common::test]
    fn it_titles_a_card_by_its_name() {
        let html = render_card(
            &result(
                "db:attribute",
                &[("name", json!("attribute")), ("transient", json!(false))],
            ),
            "concept",
            "main@id:repo",
        );
        assert!(html.contains(">attribute</div>"));
        assert!(
            !html.contains("nb-card__key\">name"),
            "name is not repeated"
        );
        assert!(
            html.contains("title=\"db:attribute\""),
            "URI kept as tooltip"
        );
    }

    /// A field holding serialized JSON showed as `{"description":"…` —
    /// punctuation, not information.
    #[dialog_common::test]
    fn it_summarises_a_json_encoded_field() {
        let encoded = json!(r#"{"description":"x","with":{"a":1}}"#);
        assert_eq!(scalar(&encoded), "{2}");
    }

    #[dialog_common::test]
    fn it_shortens_an_entity_to_its_distinguishing_tail() {
        assert_eq!(short_entity("id:notebook/scratch/3"), "3");
        assert_eq!(short_entity("short"), "short");
    }

    #[dialog_common::test]
    fn it_reports_a_failure_without_a_gallery() {
        let html = render(Some("boom"), None, "main@id:repo");
        assert!(html.contains("nb-out--error"));
        assert!(html.contains("boom"));
    }
}
