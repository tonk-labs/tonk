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

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;

/// Render an entity as a notation document. `this` is the entity
/// URI and `fields` its projected values — the two pieces a
/// [`Conclusion`][tonk_schema::conclusion::Conclusion] or an
/// evaluate `QueryResult` carries. `head` is the concept's short
/// name (e.g. `"greeting"`), used as the assertion head.
/// `bookmark` is an optional name to write as `&bookmark` after
/// the head's `:`.
pub fn format(
    this: &str,
    fields: &BTreeMap<String, Ipld>,
    head: &str,
    bookmark: Option<&str>,
) -> String {
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
    out.push_str(this);
    out.push('\n');

    for (name, value) in fields {
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

/// Indent (in spaces) of a top-level field line. Fields sit one
/// level under the head; block-scalar content sits one level
/// under the field.
const FIELD_INDENT: usize = 2;

fn write_field(out: &mut String, name: &str, value: &Ipld) {
    for _ in 0..FIELD_INDENT {
        out.push(' ');
    }
    out.push_str(name);
    out.push_str(": ");
    write_value(out, value, FIELD_INDENT);
    out.push('\n');
}

/// Render `value` after the `name: ` of a field whose key sits at
/// `indent` spaces. `indent` is only consulted for multi-line
/// strings, which become YAML literal block scalars whose content
/// lines align one level deeper.
fn write_value(out: &mut String, value: &Ipld, indent: usize) {
    match value {
        Ipld::Null => out.push('_'),
        Ipld::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Ipld::Integer(n) => out.push_str(&n.to_string()),
        // `{f:?}` keeps the decimal point (`41.0`, not `41`), so the
        // dump re-parses as a float. (Integers print bare: the wire
        // flattens signedness into one Ipld integer, so the spelling
        // of a stored signed value is not recoverable here.)
        Ipld::Float(f) => out.push_str(&format!("{f:?}")),
        Ipld::String(s) => write_string(out, s, indent),
        Ipld::Bytes(_) | Ipld::Link(_) => {
            // No notation surface for these — fall back to dag-json
            // so the value is at least machine-decodable.
            out.push_str(&dag_json_string(value));
        }
        Ipld::List(items) => {
            // Notation has no inline list syntax; fall back to a
            // bracketed form so the value is at least legible. Real
            // list rendering would use repeated `field:` lines
            // (cardinality > 1), but cardinality information isn't
            // on the Conclusion.
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, indent);
            }
            out.push(']');
        }
        Ipld::Map(_) => {
            // Same — no inline map syntax. Serialize as compact
            // dag-json so the user at least sees the shape.
            out.push_str(&dag_json_string(value));
        }
    }
}

fn dag_json_string(value: &Ipld) -> String {
    serde_ipld_dagjson::to_vec(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

fn write_string(out: &mut String, s: &str, indent: usize) {
    // Entity URIs are bare; everything else is quoted. A URI is
    // anything with a `:` and no whitespace — that catches `did:`,
    // `id:`, `db:`, attribute URIs, etc.
    if looks_like_uri(s) {
        out.push_str(s);
        return;
    }
    // A string that would need escaping inside a double-quoted
    // scalar — one carrying a newline or a `"` — renders as a YAML
    // literal block scalar (`|`) instead. Block style lets both
    // stand bare, which reads far better than `\n`/`\"`. Plain
    // single-line strings stay double-quoted.
    if s.contains('\n') || s.contains('"') {
        write_block_scalar(out, s, indent);
        return;
    }
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Write `s` as a YAML literal block scalar. The `|` indicator
/// goes on the field line; every content line is indented one
/// level (two spaces) past the field's key indent.
fn write_block_scalar(out: &mut String, s: &str, indent: usize) {
    let content_indent = indent + 2;
    // `|-` strips the final newline so a value without a trailing
    // newline round-trips; `|` (clip) keeps exactly one. Pick the
    // chomping indicator from whether the source ends in `\n`.
    if s.ends_with('\n') {
        out.push('|');
    } else {
        out.push_str("|-");
    }
    for line in s.split('\n') {
        out.push('\n');
        // Empty lines stay empty — trailing indentation on a blank
        // line is just noise and some YAML linters flag it.
        if !line.is_empty() {
            for _ in 0..content_indent {
                out.push(' ');
            }
            out.push_str(line.trim_end_matches('\r'));
        }
    }
}

/// True when `s` should render as a bare entity URI rather than a
/// quoted string — a scheme-prefixed value (`did:key:…`, `id:foo`)
/// or a reverse-dotted attribute URI (`xyz.tonk.view/greeting`).
/// Public so consumers classifying values (e.g. the grouped
/// evaluate view) decorate URIs the same way this formatter does.
pub fn looks_like_uri(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    // Scheme-prefixed (`did:key:…`, `id:foo`, `db:concept`) — a
    // colon marks the URI scheme.
    if s.contains(':') {
        return true;
    }
    // Reverse-dotted attribute URI (`xyz.tonk.view/greeting`,
    // `db.name/referent`) — both a dotted prefix and a `/`
    // separator. Plain string values rarely look like this, so
    // emitting them bare is the right default.
    s.contains('.') && s.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    /// Convert a `serde_json::Value` into the equivalent [`Ipld`]
    /// for test setup. `to_ipld(&Value::Null)` errors with "Unit
    /// is not supported", so we walk the shape ourselves to keep
    /// the test cases free of that wart.
    fn json_to_ipld(value: &Value) -> Ipld {
        match value {
            Value::Null => Ipld::Null,
            Value::Bool(b) => Ipld::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ipld::Integer(i as i128)
                } else if let Some(u) = n.as_u64() {
                    Ipld::Integer(u as i128)
                } else if let Some(f) = n.as_f64() {
                    Ipld::Float(f)
                } else {
                    Ipld::Null
                }
            }
            Value::String(s) => Ipld::String(s.clone()),
            Value::Array(items) => Ipld::List(items.iter().map(json_to_ipld).collect()),
            Value::Object(map) => Ipld::Map(
                map.iter()
                    .map(|(k, v)| (k.clone(), json_to_ipld(v)))
                    .collect(),
            ),
        }
    }

    fn fields(pairs: &[(&str, Value)]) -> BTreeMap<String, Ipld> {
        let mut map = BTreeMap::new();
        for (k, v) in pairs {
            map.insert((*k).to_owned(), json_to_ipld(v));
        }
        map
    }

    #[test]
    fn it_emits_a_head_and_this_field() {
        let out = format("did:key:zX", &fields(&[]), "greeting", None);
        assert_eq!(out, "greeting!:\n  this: did:key:zX\n");
    }

    #[test]
    fn it_emits_an_anchor_when_a_bookmark_is_given() {
        let out = format("did:key:zX", &fields(&[]), "greeting", Some("demo"));
        assert_eq!(out, "greeting!: &demo\n  this: did:key:zX\n");
    }

    #[test]
    fn it_quotes_plain_string_values() {
        let f = fields(&[("message", json!("Hello, world"))]);
        let out = format("did:key:zX", &f, "greeting", None);
        assert!(out.contains("message: \"Hello, world\"\n"));
    }

    #[test]
    fn it_leaves_uri_values_unquoted() {
        let f = fields(&[("model", json!("xyz.tonk.view/greeting"))]);
        let out = format("did:key:zX", &f, "view", None);
        assert!(out.contains("model: xyz.tonk.view/greeting\n"));
    }

    #[test]
    fn it_emits_numbers_and_bools_bare() {
        let f = fields(&[("count", json!(42)), ("active", json!(true))]);
        let out = format("did:key:zX", &f, "concept", None);
        assert!(out.contains("count: 42\n"));
        assert!(out.contains("active: true\n"));
    }

    #[test]
    fn it_skips_duplicate_this_field() {
        let f = fields(&[("this", json!("did:key:zX")), ("message", json!("Hi"))]);
        let out = format("did:key:zX", &f, "greeting", None);
        // `this:` appears exactly once.
        assert_eq!(out.matches("this:").count(), 1);
    }

    #[test]
    fn it_emits_null_as_blank() {
        // `null` round-trips to `_` so the rendered notation matches
        // a hand-typed retraction. Other JSON-null sources (missing
        // attribute, explicit null) all collapse to the same shape.
        let f = fields(&[("message", Value::Null)]);
        let out = format("did:key:zX", &f, "greeting", None);
        assert!(out.contains("message: _\n"), "unexpected output: {out}");
    }

    #[test]
    fn it_preserves_field_order_alphabetically() {
        // `fields` is a `BTreeMap`, so iteration order is
        // alphabetical by key. Pin that so the rendered notation is
        // deterministic regardless of insertion order at the source.
        let f = fields(&[
            ("zebra", json!("z")),
            ("apple", json!("a")),
            ("mango", json!("m")),
        ]);
        let out = format("did:key:zX", &f, "fruit", None);
        let apple = out.find("apple:").expect("apple field present");
        let mango = out.find("mango:").expect("mango field present");
        let zebra = out.find("zebra:").expect("zebra field present");
        assert!(apple < mango && mango < zebra, "out of order:\n{out}");
    }

    #[test]
    fn it_emits_no_extra_fields_for_an_empty_conclusion() {
        // Just `head!:` + `this:` — nothing else. The trailing
        // newline after `this:` is the document terminator.
        let out = format("did:key:zX", &fields(&[]), "greeting", None);
        assert_eq!(out.lines().count(), 2);
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn it_handles_id_uris_as_unquoted_values() {
        // `id:` and `db:` URIs come back from the worker for
        // built-in concept references — they need to render as
        // bare URIs, not strings.
        let f = fields(&[
            ("kind", json!("id:greeting")),
            ("schema", json!("db:concept")),
        ]);
        let out = format("did:key:zX", &f, "thing", None);
        assert!(out.contains("kind: id:greeting\n"), "got: {out}");
        assert!(out.contains("schema: db:concept\n"), "got: {out}");
    }

    #[test]
    fn it_renders_a_single_line_quoted_string_as_a_block_scalar() {
        // A `"` anywhere — even on one line — pushes the value to
        // block style so the quotes stand bare instead of `\"`.
        let f = fields(&[("message", json!("She said \"hi\""))]);
        let out = format("did:key:zX", &f, "greeting", None);
        assert!(out.contains("message: |-\n"), "got: {out}");
        assert!(out.contains("\n    She said \"hi\"\n"), "got: {out}");
        assert!(!out.contains("\\\""), "no escaped quotes: {out}");
    }

    #[test]
    fn it_renders_multiline_strings_as_a_block_scalar() {
        // A newline-bearing string becomes a YAML literal block
        // scalar — content indented one level past the `message`
        // key (4 spaces), not an unreadable `\n`-escaped one-liner.
        let f = fields(&[("message", json!("line one\nline two"))]);
        let out = format("did:key:zX", &f, "greeting", None);
        assert!(out.contains("message: |-\n"), "got: {out}");
        assert!(out.contains("\n    line one\n"), "got: {out}");
        assert!(out.contains("\n    line two\n"), "got: {out}");
        assert!(!out.contains("\\n"), "should not escape newlines: {out}");
    }

    #[test]
    fn it_keeps_a_trailing_newline_with_clip_chomping() {
        // A source string that ends in `\n` uses `|` (clip) so the
        // final newline round-trips; one without uses `|-` (strip).
        let f = fields(&[("body", json!("paragraph\n"))]);
        let out = format("did:key:zX", &f, "doc", None);
        assert!(out.contains("body: |\n"), "expected clip indicator: {out}");
    }

    #[test]
    fn it_leaves_quotes_bare_inside_a_block_scalar() {
        // Block scalars need no escaping — a multi-line string with
        // quotes renders them literally rather than `\"`.
        let f = fields(&[("quote", json!("she said\n\"hello\""))]);
        let out = format("did:key:zX", &f, "note", None);
        assert!(out.contains("\n    \"hello\""), "got: {out}");
        assert!(!out.contains("\\\""), "no escaped quotes in block: {out}");
    }
}
