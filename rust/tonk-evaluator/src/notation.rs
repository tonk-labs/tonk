//! Query matches as asserted notation: the form `tonk eval` prints and agent
//! hosts return, a re-submittable document an agent can read and edit.

use std::fmt::Write as _;

use tonk_notation::spell_uri;

use crate::evaluate::{QueryMatchBlock, QueryResult};

/// Render every non-empty [`QueryMatchBlock`] as a stack of
/// notation expressions, joined with `---` document markers.
/// Empty blocks (zero results for a source expression) are
/// skipped — emitting an empty section would land an unparseable
/// document in front of the agent.
pub fn render_matches(blocks: &[QueryMatchBlock]) -> String {
    let mut sections: Vec<String> = Vec::new();
    for block in blocks {
        if block.results.is_empty() {
            continue;
        }
        let mut section = String::new();
        for result in &block.results {
            render_one(&mut section, &block.label, result);
        }
        sections.push(section.trim_end_matches('\n').to_string());
    }
    sections.join("\n---\n")
}

/// One match — `<label>:` head followed by a body whose first
/// entry is `this: <entity>` and the rest is `<field>: <value>`
/// pairs. Matches the head/body grammar so the output is a
/// re-submittable notation document.
pub fn render_one(out: &mut String, label: &str, result: &QueryResult) {
    let _ = writeln!(out, "{label}:");
    let _ = writeln!(out, "  this: {this}", this = spell_uri(&result.this));
    for (field, value) in &result.fields {
        if field.ends_with("/key") {
            continue;
        }
        // A collection entry rides the wire as the value plus a
        // sibling `<field>/key`; print it back in the entry form
        // (`show: {ui: "..."}`), so the output re-evaluates to the
        // same fact.
        match result
            .fields
            .get(&format!("{field}/key"))
            .and_then(|key| key.as_str())
        {
            Some(key) => {
                let _ = writeln!(
                    out,
                    "  {field}: {{{key}: {value}}}",
                    value = render_value(value)
                );
            }
            None => {
                let _ = writeln!(out, "  {field}: {rendered}", rendered = render_value(value));
            }
        }
    }
}

/// Render a JSON-shaped attribute value as an asserted-notation
/// scalar literal.
///
/// Strings are always double-quoted so the output round-trips
/// unambiguously through saphyr — bare strings work too in most
/// cases, but a value like `"true"` would otherwise reparse as a
/// boolean.
pub fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "~".into(),
        serde_json::Value::Bool(b) => b.to_string(),
        // Every number rides the wire as a JSON number and prints
        // plainly: a literal's spelling carries no type.
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => quote_string(s),
        // Arrays / objects shouldn't normally appear here —
        // dialog attribute values are scalars — but fall back to
        // JSON if they do, so the output is at least lossless.
        other => other.to_string(),
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_prints_collection_entries_in_the_entry_form() {
        let mut result = QueryResult {
            this: "concept:zX".into(),
            fields: Default::default(),
            transient: false,
        };
        result
            .fields
            .insert("show".into(), serde_json::json!("<tonk-inspector />\n"));
        result
            .fields
            .insert("show/key".into(), serde_json::json!("ui"));
        let mut out = String::new();
        render_one(&mut out, "view!", &result);
        assert_eq!(
            out, "view!:\n  this: concept:zX\n  show: {ui: \"<tonk-inspector />\\n\"}\n",
            "the key nests back; the sibling /key field does not leak",
        );
    }

    #[test]
    fn it_renders_number_spellings() {
        // Every number prints plainly, never with a `+`; floats keep
        // their decimal point.
        assert_eq!(render_value(&serde_json::json!(41u64)), "41");
        assert_eq!(render_value(&serde_json::json!(-7)), "-7");
        assert_eq!(render_value(&serde_json::json!(41.5)), "41.5");
        assert_eq!(render_value(&serde_json::json!(41.0)), "41.0");
        // Strings quote — including number-shaped text, which is text.
        assert_eq!(render_value(&serde_json::json!("41a")), "\"41a\"");
        assert_eq!(render_value(&serde_json::json!("+41")), "\"+41\"");
        assert_eq!(render_value(&serde_json::json!("+")), "\"+\"");
    }
}
