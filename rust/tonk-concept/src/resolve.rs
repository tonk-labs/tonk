//! Phase-1 helpers: parse the `source` attribute, build the wire
//! `Query` for the concept-of-concept lookup, and turn the
//! resulting `ConceptDescriptor` plus filters into the actual
//! subscription [`Query`].

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::json;
use tonk_schema::query::Query;

/// Parsed `source` attribute. `name_or_uri` is the bookmark name
/// or concept entity URI; `filters` are the `?key=value`
/// constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSource {
    /// Bookmark name (e.g. `person`) or concept entity URI
    /// (anything containing `:`).
    pub name_or_uri: String,
    /// `key=value` filters to apply to the live subscription
    /// query as constants on those terms. Bare `&key` (no `=`)
    /// entries are ignored — projection is implicit.
    pub filters: BTreeMap<String, String>,
}

impl ParsedSource {
    /// True if `name_or_uri` looks like an entity URI (contains
    /// `:`) rather than a bookmark name.
    pub fn is_uri(&self) -> bool {
        self.name_or_uri.contains(':')
    }
}

/// Parse `"person?name=Alice&age"` into
/// `("person", { "name": "Alice" })`.
///
/// Decoding is the URLSearchParams flavour: `+` is space, `%xx`
/// is hex-decoded. Bare keys without `=` are dropped (we only
/// filter on constants).
pub fn parse_source(input: &str) -> ParsedSource {
    let (head, query) = match input.split_once('?') {
        Some((h, q)) => (h, q),
        None => (input, ""),
    };
    let mut filters = BTreeMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        filters.insert(decode_form(k), decode_form(v));
    }
    ParsedSource {
        name_or_uri: head.to_owned(),
        filters,
    }
}

/// Decode form-urlencoded text — `+` → space, `%xx` → byte. Bad
/// percent escapes pass through verbatim (no panic).
fn decode_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'+' => out.push(' '),
            b'%' => {
                let hi = bytes.next();
                let lo = bytes.next();
                if let (Some(h), Some(l)) = (hi, lo)
                    && let (Some(hd), Some(ld)) = (hex_digit(h), hex_digit(l))
                {
                    out.push(char::from(hd * 16 + ld));
                } else {
                    // Malformed — include literally.
                    out.push('%');
                    if let Some(c) = hi {
                        out.push(c as char);
                    }
                    if let Some(c) = lo {
                        out.push(c as char);
                    }
                }
            }
            _ => out.push(b as char),
        }
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Build the Phase-1 wire query that asks the concept-of-concept
/// view for the row whose `name` is `parsed.name_or_uri` (when a
/// bookmark) or whose `this` equals it (when a URI).
///
/// Reads back as `(this, name, source)` — the descriptor JSON is
/// in `source`.
pub fn phase1_query(parsed: &ParsedSource) -> Query {
    let mut terms = json!({
        "this":   { "?": { "name": "this" } },
        "name":   { "?": { "name": "name" } },
        "source": { "?": { "name": "source" } }
    });
    if parsed.is_uri() {
        terms["this"] = json!(parsed.name_or_uri.clone());
    } else {
        terms["name"] = json!(parsed.name_or_uri.clone());
    }
    let body = json!({
        "terms": terms,
        "predicate": {
            "with": {
                "concept":     { "the": "dialog.meta/concept",     "as": "Entity", "cardinality": "one" },
                "name":        { "the": "dialog.meta/name",        "as": "Text",   "cardinality": "one" },
                "description": { "the": "dialog.meta/description", "as": "Text",   "cardinality": "one" },
                "source":      { "the": "dialog.meta/source",      "as": "Text",   "cardinality": "one" }
            }
        }
    });
    serde_json::from_value(body).expect("phase1 query body is well-formed")
}

/// Build the Phase-2 subscription query: terms map binds `this`
/// to a fresh variable, every descriptor field to either a
/// constant from `filters` or a fresh variable for projection.
///
/// `descriptor_json` is the raw JSON the worker put in the
/// `source` field of the Phase-1 conclusion.
pub fn phase2_query(
    descriptor_json: &str,
    filters: &BTreeMap<String, String>,
) -> Result<Query, serde_json::Error> {
    let predicate: serde_json::Value = serde_json::from_str(descriptor_json)?;
    // Discover the field names from the descriptor's `with` map
    // so we can put one term per field into the wire query.
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, serde_json::Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    for field in with.keys() {
        let term = match filters.get(field) {
            Some(v) => json!(v),
            None => json!({ "?": { "name": field } }),
        };
        terms.insert(field.clone(), term);
    }
    let body = json!({
        "terms": terms,
        "predicate": predicate
    });
    serde_json::from_value(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_a_bare_name() {
        let parsed = parse_source("person");
        assert_eq!(parsed.name_or_uri, "person");
        assert!(parsed.filters.is_empty());
        assert!(!parsed.is_uri());
    }

    #[test]
    fn it_parses_a_uri() {
        let parsed = parse_source("did:key:zPerson");
        assert_eq!(parsed.name_or_uri, "did:key:zPerson");
        assert!(parsed.is_uri());
    }

    #[test]
    fn it_parses_a_name_with_one_filter() {
        let parsed = parse_source("person?name=Alice");
        assert_eq!(parsed.name_or_uri, "person");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice".to_string()));
    }

    #[test]
    fn it_parses_a_filter_with_projection_marker() {
        // The bare `&age` is ignored — projection is implicit.
        let parsed = parse_source("person?name=Alice&age");
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.filters.get("name"), Some(&"Alice".to_string()));
    }

    #[test]
    fn it_decodes_percent_encoded_filter_values() {
        let parsed = parse_source("person?name=Alice%20B");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice B".to_string()));
    }

    #[test]
    fn it_decodes_plus_as_space() {
        let parsed = parse_source("person?name=Alice+B");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice B".to_string()));
    }

    #[test]
    fn it_builds_a_phase1_query_filtered_by_name() {
        let parsed = parse_source("person");
        let q = phase1_query(&parsed);
        // The `name` term should be a constant string, not a var.
        let name = q.terms.get("name").expect("name term");
        let value: serde_json::Value = serde_json::to_value(name).unwrap();
        assert_eq!(value, json!("person"));
    }

    #[test]
    fn it_builds_a_phase1_query_filtered_by_uri() {
        let parsed = parse_source("did:key:zPerson");
        let q = phase1_query(&parsed);
        let this = q.terms.get("this").expect("this term");
        let value: serde_json::Value = serde_json::to_value(this).unwrap();
        assert_eq!(value, json!("did:key:zPerson"));
    }

    #[test]
    fn it_builds_a_phase2_query_projecting_every_field() {
        let descriptor = r#"{"with":{
            "name": { "the": "person/name", "as": "Text",   "cardinality": "one" },
            "age":  { "the": "person/age",  "as": "UnsignedInteger", "cardinality": "one" }
        }}"#;
        let q = phase2_query(descriptor, &BTreeMap::new()).unwrap();
        // Variable terms for every field plus `this`.
        assert!(q.terms.contains("this"));
        assert!(q.terms.contains("name"));
        assert!(q.terms.contains("age"));
    }

    #[test]
    fn it_builds_a_phase2_query_constraining_filtered_fields() {
        let descriptor = r#"{"with":{
            "name": { "the": "person/name", "as": "Text", "cardinality": "one" }
        }}"#;
        let mut filters = BTreeMap::new();
        filters.insert("name".to_string(), "Alice".to_string());
        let q = phase2_query(descriptor, &filters).unwrap();
        let term = q.terms.get("name").expect("name term");
        let value: serde_json::Value = serde_json::to_value(term).unwrap();
        assert_eq!(value, json!("Alice"));
    }
}
