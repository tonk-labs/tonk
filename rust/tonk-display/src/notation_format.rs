//! Format a [`Conclusion`] back into source notation suitable for
//! `<tonk-notation>`. The output mirrors what a user would type:
//! a `head!:` assertion (optionally followed by `&anchor`) plus a
//! `this:` field for the entity URI and one indented field per
//! projected value.
//!
//! This is **not** a round-tripping serializer for the full
//! notation grammar — it covers exactly the shapes a single
//! `Conclusion` can produce. Multi-cardinality fields (lists),
//! nested concepts, retractions, and quoted strings with embedded
//! newlines are simplified for readability.
//!
//! ```text
//! greeting!: &demo
//!   this: did:key:zX
//!   message: "Hello"
//!   count: 42
//! ```
//!
//! When `bookmark` is `None`, the `&anchor` is omitted.

use serde_json::Value;
use tonk_schema::conclusion::Conclusion;

/// Render `conclusion` as a notation document. `head` is the
/// concept's short name (e.g. `"greeting"`) — used as the
/// assertion head. `bookmark` is an optional name to write as
/// `&bookmark` after the head's `:`.
pub fn format(conclusion: &Conclusion, head: &str, bookmark: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(head);
    out.push_str("!:");
    if let Some(anchor) = bookmark {
        out.push(' ');
        out.push('&');
        out.push_str(anchor);
    }
    out.push('\n');
    out.push_str("  this: ");
    out.push_str(&conclusion.this);
    out.push('\n');

    for (name, value) in &conclusion.fields {
        // `this` is already emitted above; skip it if the query
        // also projected it as a field, otherwise it would appear
        // twice.
        if name == "this" {
            continue;
        }
        write_field(&mut out, name, value);
    }
    out
}

fn write_field(out: &mut String, name: &str, value: &Value) {
    out.push_str("  ");
    out.push_str(name);
    out.push_str(": ");
    write_value(out, value);
    out.push('\n');
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push('_'),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            // Notation has no inline list syntax; fall back to a
            // bracketed JSON-ish form so the value is at least
            // legible. Real list rendering would use repeated
            // `field:` lines (cardinality > 1), but cardinality
            // information isn't on the Conclusion.
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(_) => {
            // Same — no inline map syntax. Serialize as compact
            // JSON so the user at least sees the shape.
            out.push_str(&serde_json::to_string(value).unwrap_or_default());
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    // Entity URIs are bare; everything else is quoted. A URI is
    // anything with a `:` and no whitespace — that catches `did:`,
    // `id:`, `db:`, attribute URIs, etc.
    if looks_like_uri(s) {
        out.push_str(s);
        return;
    }
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn looks_like_uri(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    // Scheme-prefixed (`did:key:…`, `id:foo`, `db:concept`) — a
    // colon marks the URI scheme.
    if s.contains(':') {
        return true;
    }
    // Reverse-dotted attribute URI (`xyz.tonk.view/greeting`,
    // `dialog.name/referent`) — both a dotted prefix and a `/`
    // separator. Plain string values rarely look like this, so
    // emitting them bare is the right default.
    s.contains('.') && s.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_conclusion(this: &str, fields: &[(&str, Value)]) -> Conclusion {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), v.clone());
        }
        Conclusion {
            this: this.to_owned(),
            fields: map,
        }
    }

    #[test]
    fn it_emits_a_head_and_this_field() {
        let c = make_conclusion("did:key:zX", &[]);
        let out = format(&c, "greeting", None);
        assert_eq!(out, "greeting!:\n  this: did:key:zX\n");
    }

    #[test]
    fn it_emits_an_anchor_when_a_bookmark_is_given() {
        let c = make_conclusion("did:key:zX", &[]);
        let out = format(&c, "greeting", Some("demo"));
        assert_eq!(out, "greeting!: &demo\n  this: did:key:zX\n");
    }

    #[test]
    fn it_quotes_plain_string_values() {
        let c = make_conclusion("did:key:zX", &[("message", json!("Hello, world"))]);
        let out = format(&c, "greeting", None);
        assert!(out.contains("message: \"Hello, world\"\n"));
    }

    #[test]
    fn it_leaves_uri_values_unquoted() {
        let c = make_conclusion("did:key:zX", &[("model", json!("xyz.tonk.view/greeting"))]);
        let out = format(&c, "view", None);
        assert!(out.contains("model: xyz.tonk.view/greeting\n"));
    }

    #[test]
    fn it_emits_numbers_and_bools_bare() {
        let c = make_conclusion(
            "did:key:zX",
            &[("count", json!(42)), ("active", json!(true))],
        );
        let out = format(&c, "concept", None);
        assert!(out.contains("count: 42\n"));
        assert!(out.contains("active: true\n"));
    }

    #[test]
    fn it_skips_duplicate_this_field() {
        let c = make_conclusion(
            "did:key:zX",
            &[("this", json!("did:key:zX")), ("message", json!("Hi"))],
        );
        let out = format(&c, "greeting", None);
        // `this:` appears exactly once.
        assert_eq!(out.matches("this:").count(), 1);
    }

    #[test]
    fn it_emits_null_as_blank() {
        // `null` round-trips to `_` so the rendered notation matches
        // a hand-typed retraction. Other JSON-null sources (missing
        // attribute, explicit null) all collapse to the same shape.
        let c = make_conclusion("did:key:zX", &[("message", Value::Null)]);
        let out = format(&c, "greeting", None);
        assert!(out.contains("message: _\n"), "unexpected output: {out}");
    }

    #[test]
    fn it_preserves_field_order_alphabetically() {
        // `Conclusion.fields` is a `BTreeMap`, so iteration order is
        // alphabetical by key. Pin that so the rendered notation is
        // deterministic regardless of insertion order at the source.
        let c = make_conclusion(
            "did:key:zX",
            &[
                ("zebra", json!("z")),
                ("apple", json!("a")),
                ("mango", json!("m")),
            ],
        );
        let out = format(&c, "fruit", None);
        let apple = out.find("apple:").expect("apple field present");
        let mango = out.find("mango:").expect("mango field present");
        let zebra = out.find("zebra:").expect("zebra field present");
        assert!(apple < mango && mango < zebra, "out of order:\n{out}");
    }

    #[test]
    fn it_emits_no_extra_fields_for_an_empty_conclusion() {
        // Just `head!:` + `this:` — nothing else. The trailing
        // newline after `this:` is the document terminator.
        let c = make_conclusion("did:key:zX", &[]);
        let out = format(&c, "greeting", None);
        assert_eq!(out.lines().count(), 2);
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn it_handles_id_uris_as_unquoted_values() {
        // `id:` and `db:` URIs come back from the worker for
        // built-in concept references — they need to render as
        // bare URIs, not strings.
        let c = make_conclusion(
            "did:key:zX",
            &[
                ("kind", json!("id:greeting")),
                ("schema", json!("db:concept")),
            ],
        );
        let out = format(&c, "thing", None);
        assert!(out.contains("kind: id:greeting\n"), "got: {out}");
        assert!(out.contains("schema: db:concept\n"), "got: {out}");
    }

    #[test]
    fn it_escapes_quotes_in_strings() {
        let c = make_conclusion("did:key:zX", &[("message", json!("She said \"hi\""))]);
        let out = format(&c, "greeting", None);
        assert!(out.contains("message: \"She said \\\"hi\\\"\"\n"));
    }
}
