//! Wire-query construction for `<tonk-display>`.
//!
//! - [`view_query`] — resolve a model's presentations: the `view`
//!   concept instance whose `this` IS the model entity, projecting
//!   the `show` dictionary (facet → template).
//! - [`entity_query`] — subscribe to a single entity by URI,
//!   projecting every field in the model concept's descriptor.
//! - [`view_predicate`] — the descriptor JSON of the built-in
//!   `view` concept, the predicate of every view query.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_schema::query::Query;

/// The facet a `<tonk-display>` with an `entity` renders when no
/// explicit `view=` facet is given.
pub const DETAIL_FACET: &str = "ui";

/// The facet a `<tonk-display>` without an `entity` (directory mode)
/// renders when no explicit `view=` facet is given.
pub const DIRECTORY_FACET: &str = "directory";

/// The `show` entry marking a portal document: a `type` entry whose
/// value is `text/html` says the selected template is a full HTML
/// document mounted in an isolated `<tonk-portal>`, not interpolated
/// inline.
pub const TYPE_FACET: &str = "type";

/// Build the live view-resolution query: every `show` entry of the
/// model's `view` instance. The view instance's `this` IS the model
/// entity, so the query pins `this` and projects the whole dictionary
/// — one flat row per facet. The caller folds the rows
/// ([`crate::fold::select_rows`]) into `show: {facet: template}` and
/// picks the facet it renders ([`crate::fold::show_template`]).
pub fn view_query(model_entity: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(model_entity));
    terms.insert("show".into(), json!({ "?": { "name": "show" } }));
    terms.insert("show/key".into(), json!({ "?": { "name": "show/key" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": view_predicate() }))
}

/// The `event` concept's shape, kept in step with the declaration
/// pinned to `tonk:event` in the standard library.
///
/// `where` is a keyed collection like `view`'s `show`, so each source
/// lands as its own fact (`xyz.tonk.event.where/<field>`) with
/// cardinality one — a space can supersede one source without
/// restating the map. Its own domain, so a command field called `type`
/// cannot collide with the `type` attribute.
pub fn event_predicate() -> Value {
    json!({
        "with": {
            "type": {
                "the": "xyz.tonk.event/type",
                "as": "Text",
                "cardinality": "one"
            },
            "where": {
                "the": { "domain": "xyz.tonk.event.where", "keyed": "dictionary" },
                "as": "Text",
                "cardinality": "one"
            }
        }
    })
}

/// Build the query that reads one `event!:` declaration: its `type`
/// and every entry of its `where` map.
///
/// The optional `prevent-default` / `stop-propagation` markers are
/// deliberately absent: they are `maybe:` fields, so pinning them here
/// would make a declaration that omits them match nothing. They are
/// read separately ([`event_flags_query`]).
pub fn event_query(event_entity: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(event_entity));
    terms.insert("type".into(), json!({ "?": { "name": "type" } }));
    terms.insert("where".into(), json!({ "?": { "name": "where" } }));
    terms.insert("where/key".into(), json!({ "?": { "name": "where/key" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": event_predicate() }))
}

/// Build the query that reads a declaration's optional side-effect
/// flags. Separate from [`event_query`] because an `event!:` that
/// declares neither must still resolve.
pub fn event_flags_query(event_entity: &str) -> Result<Query, serde_json::Error> {
    let predicate = json!({
        "with": {
            "prevent-default": {
                "the": "xyz.tonk.event/prevent-default",
                "as": "Boolean",
                "cardinality": "one"
            },
            "stop-propagation": {
                "the": "xyz.tonk.event/stop-propagation",
                "as": "Boolean",
                "cardinality": "one"
            }
        }
    });
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(event_entity));
    terms.insert(
        "prevent-default".into(),
        json!({ "?": { "name": "prevent-default" } }),
    );
    terms.insert(
        "stop-propagation".into(),
        json!({ "?": { "name": "stop-propagation" } }),
    );
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// Whether a `with:` entry declares a keyed collection: its `the` is a
/// `{domain, keyed}` object rather than an attribute string.
///
/// A collection field binds TWO terms — the field and its key operand
/// (`block`, `block/key`) — because an entry is a `(key, value)` pair.
/// Requesting only the field leaves the key unbound, and the wire fold
/// that turns the pair into `{key: value}` then has nothing to fold:
/// the entry arrives as a bare value and every key reads empty.
fn is_collection(spec: &Value) -> bool {
    spec.get("the").is_some_and(Value::is_object)
}

/// The terms one `with:` entry contributes: the field, plus its key
/// operand when the field is a keyed collection.
fn field_terms(field: &str, spec: &Value, terms: &mut IndexMap<String, Value>) {
    terms.insert(field.to_owned(), json!({ "?": { "name": field } }));
    if is_collection(spec) {
        let key = format!("{field}/key");
        terms.insert(key.clone(), json!({ "?": { "name": key } }));
    }
}

/// Build the live entity subscription query: given the model
/// concept's `descriptor_json` (raw JSON from a Phase-1 resolve)
/// and the target `entity` URI, return a query that pins `this` to
/// `entity` and projects every field in the descriptor's `with:`
/// map as a variable.
///
/// Frame size from this subscription is 0 (entity not yet on the
/// branch / removed) or 1.
pub fn entity_query(descriptor_json: &str, entity: &str) -> Result<Query, serde_json::Error> {
    let predicate: Value = serde_json::from_str(descriptor_json)?;
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(entity));
    for (field, spec) in &with {
        field_terms(field, spec, &mut terms);
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// Collect the `cardinality: one` field names from a concept's
/// `descriptor_json` — its `with:` (required) and `maybe:` (optional) blocks.
///
/// These are the **scalar** fields: a single value per subject, not an
/// iteration axis. The renderer's planner uses this set so a scalar field used
/// in a template is a plain substitution rendered once, never an iteration root
/// that clones its host zero times (and drops it) when the value is absent. A
/// malformed descriptor yields an empty set — the value-driven default.
pub fn scalar_field_names(descriptor_json: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Ok(value) = serde_json::from_str::<Value>(descriptor_json) else {
        return out;
    };
    for block in ["with", "maybe"] {
        let Some(map) = value.get(block).and_then(|v| v.as_object()) else {
            continue;
        };
        for (field, spec) in map {
            if spec.get("cardinality").and_then(|c| c.as_str()) == Some("one") {
                out.insert(field.clone());
            }
        }
    }
    out
}

/// The descriptor of the built-in `view` concept.
///
/// One field: `show` — a model's presentations, keyed by facet
/// (`ui`, `directory`, `label`, `title`, …). Each entry is stored as
/// `<model> xyz.tonk.view/<facet> <template>`: the view instance IS
/// the model entity, so there is no `model` back-pointer and no
/// per-kind view concept. Cardinality is `one` per entry, so
/// re-asserting a facet supersedes its template. Kept in step with
/// the bootstrap declaration pinned to `tonk:view`.
pub fn view_predicate() -> Value {
    json!({
        "with": {
            "show": {
                "the": { "domain": "xyz.tonk.view", "keyed": "dictionary" },
                "as": "Text",
                "cardinality": "one"
            }
        }
    })
}

/// Build the live **directory** subscription query: like
/// [`entity_query`] but with `this` left as a variable instead of
/// pinned, so the query matches *every* instance of the model. The
/// worker emits one flat row per (instance, many-value) tuple; the
/// caller groups them by `this`. Used when `<tonk-display>` has no
/// `entity` (directory mode).
pub fn instances_query(descriptor_json: &str) -> Result<Query, serde_json::Error> {
    let predicate: Value = serde_json::from_str(descriptor_json)?;
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    for (field, spec) in &with {
        field_terms(field, spec, &mut terms);
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// True if `s` looks like an entity URI (contains `:`) rather than
/// a bookmark name.
pub fn looks_like_uri(s: &str) -> bool {
    s.contains(':')
}

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

/// Build the name-resolution query: resolve a bookmark `name` to the
/// entity it points at through the **Name concept** — the `id:<name>`
/// entity's `db.name/referent` claim (cardinality one, so at most
/// one match).
///
/// This is the single source of truth for "what does this name refer
/// to": the analyzer publishes a `Name` claim for every `&anchor`,
/// including concepts pinned to a `this:` URI (e.g. `&workspace` +
/// `this: tonk:workspace` → `id:workspace` → `tonk:workspace`). A
/// concept's `db.meta/name` claim, by contrast, is only emitted
/// when its `this:` is *derived*, so resolving a model/view name
/// against `db.meta/name` misses pinned concepts. Resolving names
/// here and feeding the resulting URI to [`phase1_query`] makes model,
/// view, and entity name resolution agree.
///
/// Reads back as `(entity)` — the referent URI.
pub fn name_query(name: &str) -> Query {
    let body = json!({
        "terms": {
            "this":   format!("id:{name}"),
            "entity": { "?": { "name": "entity" } },
        },
        "predicate": {
            "with": {
                "entity": { "the": "db.name/referent", "as": "Entity", "cardinality": "one" }
            }
        }
    });
    serde_json::from_value(body).expect("name query body is well-formed")
}

/// Build the Phase-1 wire query that asks the concept-of-concept
/// view for the row whose `this` equals `parsed.name_or_uri`.
///
/// `parsed.name_or_uri` is expected to be a concept entity URI — a
/// bookmark name should be resolved to its referent via [`name_query`]
/// first. (For backwards compatibility a non-URI value still falls
/// back to a `db.meta/name` filter, but that path misses concepts
/// pinned to a `this:` URI; prefer resolving the name first.)
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
                "concept":     { "the": "db.meta/concept",     "as": "Entity",  "cardinality": "one" },
                "name":        { "the": "db.meta/name",        "as": "Text",    "cardinality": "one" },
                "description": { "the": "db.meta/description", "as": "Text",    "cardinality": "one" },
                "source":      { "the": "db.meta/source",      "as": "Text",    "cardinality": "one" },
                "transient":   { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
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
) -> Result<Query, Phase2Error> {
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
    for (field, spec) in &with {
        let term = match filters.get(field) {
            Some(raw) => {
                let as_type = spec.get("as").and_then(|v| v.as_str());
                coerce_filter_value(field, as_type, raw)?
            }
            None => json!({ "?": { "name": field } }),
        };
        terms.insert(field.clone(), term);
    }
    let body = json!({
        "terms": terms,
        "predicate": predicate
    });
    Ok(serde_json::from_value(body)?)
}

/// Coerce a `?key=value` filter string to a JSON value matching the
/// field's declared type (the descriptor's `as`), so the constant
/// constraint compares against the stored value rather than a string.
///
/// A value that doesn't parse for a numeric or boolean type is an
/// error, not a string fallback: a string constant on a numeric
/// field can never match the stored value, so the query would
/// return nothing and the user couldn't tell a typo'd filter from
/// a genuinely empty result. Surfacing it lets the caller render a
/// visible error instead. Text/Entity/Symbol/Bytes (and unknown
/// types) are inherently string-shaped, so they pass through.
fn coerce_filter_value(
    field: &str,
    as_type: Option<&str>,
    raw: &str,
) -> Result<serde_json::Value, Phase2Error> {
    let coerced = match as_type {
        Some("UnsignedInteger") => raw.parse::<u64>().ok().map(|n| json!(n)),
        Some("SignedInteger") => raw.parse::<i64>().ok().map(|n| json!(n)),
        // Reject non-finite floats too: `serde_json` can't represent
        // `inf`/`NaN` and would silently serialize them as `null`.
        Some("Float") => raw
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| json!(n)),
        Some("Boolean") => match raw {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        // Text, Entity, Symbol, Bytes, or unknown: keep as a string.
        _ => return Ok(json!(raw)),
    };
    coerced.ok_or_else(|| Phase2Error::Filter {
        field: field.to_owned(),
        as_type: as_type.unwrap_or("Text").to_owned(),
        value: raw.to_owned(),
    })
}

/// Why [`phase2_query`] couldn't build a subscription query.
#[derive(Debug)]
pub enum Phase2Error {
    /// The descriptor JSON or the assembled query body didn't
    /// deserialize into the wire `Query` shape.
    Json(serde_json::Error),
    /// A `?field=value` filter targets a numeric or boolean field
    /// but the value doesn't parse as that type.
    Filter {
        /// The descriptor field the filter constrains.
        field: String,
        /// The field's declared `as:` type.
        as_type: String,
        /// The raw filter value that failed to parse.
        value: String,
    },
}

impl std::fmt::Display for Phase2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::Filter {
                field,
                as_type,
                value,
            } => write!(f, "filter `{field}={value}` is not a valid {as_type}"),
        }
    }
}

impl std::error::Error for Phase2Error {}

impl From<serde_json::Error> for Phase2Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn the_view_predicate_is_the_show_dictionary() {
        let p = view_predicate();
        let with = p.get("with").and_then(|v| v.as_object()).expect("with");
        let show = with.get("show").expect("show field");
        assert_eq!(
            show.get("the"),
            Some(&json!({ "domain": "xyz.tonk.view", "keyed": "dictionary" })),
            "templates live as entries of the xyz.tonk.view domain",
        );
        assert!(
            !with.contains_key("model"),
            "the view instance IS the model entity; no back-pointer field"
        );
    }

    #[dialog_common::test]
    fn it_collects_cardinality_one_fields_from_with_and_maybe() {
        // `with` holds a required cardinality-one field, `maybe` an optional
        // cardinality-one field, and a cardinality-many field is excluded.
        let descriptor = r#"{
            "with": {
                "id":    { "the": "xyz.tonk.site/id",   "as": "Text", "cardinality": "one" },
                "items": { "the": "x/items",            "as": "Text", "cardinality": "many" }
            },
            "maybe": {
                "rest":  { "the": "xyz.tonk.site/rest", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let scalars = scalar_field_names(descriptor);
        assert!(scalars.contains("id"), "required cardinality-one field");
        assert!(scalars.contains("rest"), "optional cardinality-one field");
        assert!(!scalars.contains("items"), "cardinality-many excluded");
    }

    #[dialog_common::test]
    fn it_returns_no_scalar_fields_for_invalid_descriptor() {
        assert!(scalar_field_names("not json").is_empty());
    }

    #[dialog_common::test]
    fn it_builds_an_entity_query_pinning_this() {
        let descriptor = r#"{"with":{
            "message": { "the": "greeting/message", "as": "Text", "cardinality": "one" }
        }}"#;
        let q = entity_query(descriptor, "did:key:zGreeting").expect("entity_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("did:key:zGreeting"),
        );
    }

    #[dialog_common::test]
    fn it_projects_every_descriptor_field_in_the_entity_query() {
        let descriptor = r#"{"with":{
            "message":   { "the": "greeting/message",   "as": "Text", "cardinality": "one" },
            "recipient": { "the": "greeting/recipient", "as": "Text", "cardinality": "one" }
        }}"#;
        let q = entity_query(descriptor, "did:key:zGreeting").expect("entity_query");
        assert!(q.terms.contains("message"));
        assert!(q.terms.contains("recipient"));
    }

    /// A keyed-collection field binds its key operand alongside the
    /// field. An entry is a `(key, value)` pair, and the wire fold that
    /// turns the pair into `{key: value}` only fires when BOTH are
    /// bound — so requesting the field alone makes every entry arrive
    /// as a bare value with no key. A notebook then reads all its
    /// blocks as unplaced and re-keys every one of them on each edit,
    /// which duplicates the whole sequence.
    #[dialog_common::test]
    fn it_binds_the_key_operand_of_a_collection_field() {
        let descriptor = r#"{"with":{
            "title": { "the": "xyz.tonk.notebook/title", "as": "Text", "cardinality": "one" },
            "block": {
                "the": { "domain": "xyz.tonk.notebook", "keyed": "sequence" },
                "as": "Entity",
                "cardinality": "many"
            }
        }}"#;

        let entity = entity_query(descriptor, "id:notebook/scratch").expect("entity_query");
        assert!(entity.terms.contains("block"), "the field is bound");
        assert!(
            entity.terms.contains("block/key"),
            "so is its key — without it the entry has no key to fold on"
        );
        assert!(
            !entity.terms.contains("title/key"),
            "a scalar field has no key operand"
        );

        let instances = instances_query(descriptor).expect("instances_query");
        assert!(instances.terms.contains("block"));
        assert!(
            instances.terms.contains("block/key"),
            "the directory query binds it too"
        );
    }

    #[dialog_common::test]
    fn it_distinguishes_uri_from_bookmark() {
        assert!(looks_like_uri("did:key:zAlice"));
        assert!(looks_like_uri("concept:abc"));
        assert!(!looks_like_uri("greeting"));
    }

    #[dialog_common::test]
    fn it_builds_a_view_query_pinning_the_model_entity() {
        // The view instance IS the model entity: `this` is pinned and
        // the whole `show` dictionary flows back, entry and key.
        let q = view_query("concept:zCounter").expect("view_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("concept:zCounter"),
        );
        let show = q.terms.get("show").expect("show term");
        assert_eq!(
            serde_json::to_value(show).unwrap(),
            json!({ "?": { "name": "show" } }),
        );
        assert!(
            q.terms.contains("show/key"),
            "the key operand rides along so the wire fold has a key to fold on"
        );
    }

    #[dialog_common::test]
    fn it_parses_a_bare_name() {
        let parsed = parse_source("person");
        assert_eq!(parsed.name_or_uri, "person");
        assert!(parsed.filters.is_empty());
        assert!(!parsed.is_uri());
    }

    #[dialog_common::test]
    fn it_parses_a_uri() {
        let parsed = parse_source("did:key:zPerson");
        assert_eq!(parsed.name_or_uri, "did:key:zPerson");
        assert!(parsed.is_uri());
    }

    #[dialog_common::test]
    fn it_parses_a_name_with_one_filter() {
        let parsed = parse_source("person?name=Alice");
        assert_eq!(parsed.name_or_uri, "person");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice".to_string()));
    }

    #[dialog_common::test]
    fn it_parses_a_filter_with_projection_marker() {
        // The bare `&age` is ignored — projection is implicit.
        let parsed = parse_source("person?name=Alice&age");
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.filters.get("name"), Some(&"Alice".to_string()));
    }

    #[dialog_common::test]
    fn it_decodes_percent_encoded_filter_values() {
        let parsed = parse_source("person?name=Alice%20B");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice B".to_string()));
    }

    #[dialog_common::test]
    fn it_decodes_plus_as_space() {
        let parsed = parse_source("person?name=Alice+B");
        assert_eq!(parsed.filters.get("name"), Some(&"Alice B".to_string()));
    }

    #[dialog_common::test]
    fn it_builds_a_phase1_query_filtered_by_name() {
        let parsed = parse_source("person");
        let q = phase1_query(&parsed);
        // The `name` term should be a constant string, not a var.
        let name = q.terms.get("name").expect("name term");
        let value: serde_json::Value = serde_json::to_value(name).unwrap();
        assert_eq!(value, json!("person"));
    }

    #[dialog_common::test]
    fn it_builds_a_phase1_query_filtered_by_uri() {
        let parsed = parse_source("did:key:zPerson");
        let q = phase1_query(&parsed);
        let this = q.terms.get("this").expect("this term");
        let value: serde_json::Value = serde_json::to_value(this).unwrap();
        assert_eq!(value, json!("did:key:zPerson"));
    }

    #[dialog_common::test]
    fn it_resolves_a_name_through_the_name_concept() {
        // A bare name resolves via the Name concept: `id:<name>`'s
        // `db.name/referent`. This is what lets `workspace` (a
        // concept pinned to `this: tonk:workspace`, so carrying a Name
        // claim but no `db.meta/name`) resolve at all.
        let q = name_query("workspace");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("id:workspace"),
            "the name query pins `this` to the `id:<name>` entity",
        );
        // It projects `entity` (the referent) and constrains on the
        // Name attribute.
        let entity = q.terms.get("entity").expect("entity term");
        assert_eq!(
            serde_json::to_value(entity).unwrap(),
            json!({ "?": { "name": "entity" } }),
        );
        let predicate = serde_json::to_value(&q.predicate).unwrap();
        assert_eq!(
            predicate
                .pointer("/with/entity/the")
                .and_then(|v| v.as_str()),
            Some("db.name/referent"),
            "the name query constrains on the Name referent attribute",
        );
    }

    #[dialog_common::test]
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

    #[dialog_common::test]
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

    fn filtered_query(as_type: &str, raw: &str) -> Result<Query, Phase2Error> {
        let descriptor =
            format!(r#"{{"with":{{"f":{{"the":"x/f","as":"{as_type}","cardinality":"one"}}}}}}"#);
        let mut filters = BTreeMap::new();
        filters.insert("f".to_string(), raw.to_string());
        phase2_query(&descriptor, &filters)
    }

    fn filtered_term(as_type: &str, raw: &str) -> serde_json::Value {
        let q = filtered_query(as_type, raw).expect("query builds");
        let term = q.terms.get("f").expect("f term");
        serde_json::to_value(term).unwrap()
    }

    // An integer-typed field filtered via `?f=5` must constrain on
    // the number `5`, not the string `"5"` — otherwise it never
    // matches the stored integer value.
    #[dialog_common::test]
    fn it_coerces_an_unsigned_integer_filter_to_a_number() {
        assert_eq!(filtered_term("UnsignedInteger", "5"), json!(5));
    }

    #[dialog_common::test]
    fn it_coerces_a_signed_integer_filter_to_a_number() {
        assert_eq!(filtered_term("SignedInteger", "-5"), json!(-5));
    }

    #[dialog_common::test]
    fn it_coerces_a_float_filter_to_a_number() {
        assert_eq!(filtered_term("Float", "3.5"), json!(3.5));
    }

    #[dialog_common::test]
    fn it_coerces_a_boolean_filter_to_a_bool() {
        assert_eq!(filtered_term("Boolean", "true"), json!(true));
    }

    // Text/Entity/etc. stay strings, as before.
    #[dialog_common::test]
    fn it_keeps_a_text_filter_as_a_string() {
        assert_eq!(filtered_term("Text", "5"), json!("5"));
    }

    // A non-numeric value for a numeric field is rejected rather
    // than coerced to a string constant that could never match the
    // stored integer — that would render a blank result the user
    // couldn't distinguish from a genuinely empty set.
    #[dialog_common::test]
    fn it_rejects_an_unparseable_unsigned_integer_filter() {
        let err = filtered_query("UnsignedInteger", "lots").expect_err("should reject");
        assert!(
            matches!(&err, Phase2Error::Filter { field, as_type, value }
                if field == "f" && as_type == "UnsignedInteger" && value == "lots"),
            "got {err:?}",
        );
    }

    // A negative value can't be an `UnsignedInteger`; it's rejected,
    // not silently kept as a string.
    #[dialog_common::test]
    fn it_rejects_a_negative_value_for_an_unsigned_integer_filter() {
        let err = filtered_query("UnsignedInteger", "-5").expect_err("should reject");
        assert!(matches!(err, Phase2Error::Filter { .. }), "got {err:?}");
    }

    // `inf`/`NaN` parse as `f64` but `serde_json` can't represent
    // them; rejecting avoids a silent `null` constant.
    #[dialog_common::test]
    fn it_rejects_a_non_finite_float_filter() {
        for raw in ["inf", "NaN", "-inf"] {
            let err = filtered_query("Float", raw).expect_err("should reject");
            assert!(
                matches!(err, Phase2Error::Filter { .. }),
                "{raw}: got {err:?}"
            );
        }
    }

    // Boolean coercion is exact: only `true`/`false`. Anything else
    // (`True`, `1`, `yes`) is rejected.
    #[dialog_common::test]
    fn it_rejects_a_non_boolean_value_for_a_boolean_filter() {
        for raw in ["True", "1", "yes"] {
            let err = filtered_query("Boolean", raw).expect_err("should reject");
            assert!(
                matches!(err, Phase2Error::Filter { .. }),
                "{raw}: got {err:?}"
            );
        }
    }
}
