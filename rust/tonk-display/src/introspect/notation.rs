//! Rendering what a display resolved back into the notation an author
//! would have typed.
//!
//! The panel's job is to show the same thing the inspector shows: a
//! `concept!:` or `command!:` declaration, and an entity as a `head!:`
//! assertion. Reading a table of fields is a translation step; reading
//! the notation is not.
//!
//! Output is a [`Document`] — lines of tokenized spans, each line
//! optionally attributed to a field — rather than a string. That is
//! what buys the two things a string could not: the panel colours
//! spans by token, and it keys each line to a field name, which is the
//! same key a concept row, a template span and a page marker already
//! use. So resting on `content:` in the data lights the slot that
//! rendered it, and resting on `{content}` in the template lights the
//! line here.
//!
//! `<tonk-notation>` is the other way to do the colouring, and it
//! knows the real grammar. What it cannot do is tell the panel which
//! line is which field, because it renders from a text blob. This
//! keeps the mapping and accepts a simpler tokenizer.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use serde::{Deserialize, Serialize};

/// What a span is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Token {
    /// The assertion head — `concept!:`, `prose!:`.
    Head,
    /// An `&anchor` naming the declaration.
    Anchor,
    /// A field or property name, including its colon.
    Key,
    /// A quoted or bare scalar.
    Value,
    /// An entity URI.
    Entity,
    /// A comment.
    Comment,
    /// Anything else — punctuation, indentation.
    Plain,
}

/// One coloured run within a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// How to colour it.
    pub token: Token,
    /// The text.
    pub text: String,
}

impl Span {
    fn new(token: Token, text: impl Into<String>) -> Self {
        Self {
            token,
            text: text.into(),
        }
    }
}

/// One line of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// The spans, in order.
    pub spans: Vec<Span>,
    /// The field this line is about, if any — the key that ties it to
    /// a slot on the page and to the same field elsewhere in the panel.
    pub field: Option<String>,
    /// The dialog attribute the field projects (`io.gozala.prose/content`)
    /// — the relation under the name. A field name is local to a
    /// concept; the attribute is what the fact is actually stored
    /// under, so it is the handle for anything that wants to ask about
    /// the same relation elsewhere.
    pub attribute: Option<String>,
}

impl Line {
    /// The line as plain text, for tests and for copying.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

/// A rendered notation document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The lines, in order.
    pub lines: Vec<Line>,
}

impl Document {
    /// The whole document as plain text.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(Line::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn push(&mut self, field: Option<&str>, spans: Vec<Span>) {
        self.lines.push(Line {
            spans,
            field: field.map(str::to_owned),
            attribute: None,
        });
    }

    /// Stamp the relation onto every line already attributed to
    /// `field`.
    fn relate(&mut self, field: &str, attribute: &str) {
        for line in &mut self.lines {
            if line.field.as_deref() == Some(field) {
                line.attribute = Some(attribute.to_owned());
            }
        }
    }
}

/// Field name -> the dialog attribute it projects, read out of a
/// concept descriptor. What the data panel needs to show the relation
/// beside the value.
pub fn relations(descriptor_json: &str) -> BTreeMap<String, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(descriptor_json) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for block in ["with", "maybe"] {
        let Some(map) = value.get(block).and_then(|v| v.as_object()) else {
            continue;
        };
        for (field, spec) in map {
            // `the` is a string for a plain attribute and an object
            // (`{domain, keyed}`) for a keyed collection; the domain
            // is the relation in that case.
            let attribute = match spec.get("the") {
                Some(serde_json::Value::String(text)) => Some(text.clone()),
                Some(serde_json::Value::Object(entry)) => entry
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                _ => None,
            };
            if let Some(attribute) = attribute {
                out.insert(field.clone(), attribute);
            }
        }
    }
    out
}

/// Two spaces per level, as the library files are written.
const INDENT: &str = "  ";

fn indent(level: usize) -> Span {
    Span::new(Token::Plain, INDENT.repeat(level))
}

/// Render one entity as a `head!:` assertion — the data panel.
///
/// `head` is the concept's short name, so the document reads as the
/// assertion that would produce this entity. Field order is the
/// `BTreeMap`'s, which is stable across frames, so a value changing
/// does not reshuffle the panel under the pointer.
pub fn entity(
    head: &str,
    this: &str,
    fields: &BTreeMap<String, Ipld>,
    relations: &BTreeMap<String, String>,
) -> Document {
    let mut document = Document::default();
    document.push(None, vec![Span::new(Token::Head, format!("{head}!:"))]);
    document.push(
        Some("this"),
        vec![
            indent(1),
            Span::new(Token::Key, "this:"),
            Span::new(Token::Plain, " "),
            Span::new(Token::Entity, this),
        ],
    );
    for (name, value) in fields {
        if name == "this" {
            continue;
        }
        write_value(&mut document, name, name, 1, value);
    }
    for (field, attribute) in relations {
        document.relate(field, attribute);
    }
    document
}

/// Render a concept or command declaration — the model and command
/// panels.
///
/// `head` is `concept` or `command`; `name` becomes the `&anchor`.
/// The body comes from the lowered descriptor, since that is what a
/// display actually resolved: the YAML a library file was written in
/// is not stored, so this is the declaration as the runtime holds it
/// rather than as it was typed. The two say the same thing.
pub fn declaration(head: &str, name: Option<&str>, descriptor_json: &str) -> Document {
    let mut document = Document::default();
    let mut head_spans = vec![Span::new(Token::Head, format!("{head}!:"))];
    if let Some(name) = name {
        head_spans.push(Span::new(Token::Plain, " "));
        head_spans.push(Span::new(Token::Anchor, format!("&{name}")));
    }
    document.push(None, head_spans);

    let Ok(value) = serde_json::from_str::<serde_json::Value>(descriptor_json) else {
        document.push(
            None,
            vec![
                indent(1),
                Span::new(Token::Comment, "# descriptor did not parse"),
            ],
        );
        return document;
    };

    if let Some(text) = value.get("description").and_then(|v| v.as_str()) {
        document.push(
            None,
            vec![
                indent(1),
                Span::new(Token::Key, "description:"),
                Span::new(Token::Plain, " "),
                Span::new(Token::Value, quoted(text)),
            ],
        );
    }

    let relations = relations(descriptor_json);
    for block in ["with", "maybe"] {
        let Some(map) = value.get(block).and_then(|v| v.as_object()) else {
            continue;
        };
        if map.is_empty() {
            continue;
        }
        document.push(
            None,
            vec![indent(1), Span::new(Token::Key, format!("{block}:"))],
        );
        for (field, spec) in map {
            document.push(
                Some(field),
                vec![indent(2), Span::new(Token::Key, format!("{field}:"))],
            );
            // `the`, `as`, `cardinality` first and in that order —
            // the shape a library file is written in — then anything
            // else the descriptor carries, so nothing is hidden.
            let mut seen = Vec::new();
            for key in ["the", "as", "cardinality", "description"] {
                if let Some(entry) = spec.get(key) {
                    write_property(&mut document, field, key, entry);
                    seen.push(key);
                }
            }
            if let Some(entries) = spec.as_object() {
                for (key, entry) in entries {
                    if !seen.contains(&key.as_str()) {
                        write_property(&mut document, field, key, entry);
                    }
                }
            }
        }
    }
    for (field, attribute) in &relations {
        document.relate(field, attribute);
    }
    document
}

/// One `key: value` property line under a field.
fn write_property(document: &mut Document, field: &str, key: &str, value: &serde_json::Value) {
    let text = match value {
        serde_json::Value::String(text) => {
            if key == "the" {
                Span::new(Token::Entity, text.clone())
            } else {
                Span::new(Token::Value, text.clone())
            }
        }
        other => Span::new(Token::Value, other.to_string()),
    };
    document.push(
        Some(field),
        vec![
            indent(3),
            Span::new(Token::Key, format!("{key}:")),
            Span::new(Token::Plain, " "),
            text,
        ],
    );
}

/// One field of an entity: a scalar on its own line, a list as an
/// indented run, everything else as compact JSON.
fn write_value(document: &mut Document, field: &str, label: &str, level: usize, value: &Ipld) {
    match value {
        Ipld::List(items) => {
            document.push(
                Some(field),
                vec![indent(level), Span::new(Token::Key, format!("{label}:"))],
            );
            for item in items {
                document.push(
                    Some(field),
                    vec![
                        indent(level + 1),
                        Span::new(Token::Plain, "- "),
                        scalar(item),
                    ],
                );
            }
        }
        Ipld::Map(entries) => {
            document.push(
                Some(field),
                vec![indent(level), Span::new(Token::Key, format!("{label}:"))],
            );
            for (key, entry) in entries {
                write_value(document, field, key, level + 1, entry);
            }
        }
        other => document.push(
            Some(field),
            vec![
                indent(level),
                Span::new(Token::Key, format!("{label}:")),
                Span::new(Token::Plain, " "),
                scalar(other),
            ],
        ),
    }
}

/// A scalar, spelled the way it would be typed.
fn scalar(value: &Ipld) -> Span {
    match value {
        Ipld::String(text) if is_entity(text) => Span::new(Token::Entity, text.clone()),
        Ipld::String(text) => Span::new(Token::Value, quoted(text)),
        Ipld::Integer(number) => Span::new(Token::Value, number.to_string()),
        Ipld::Float(number) => Span::new(Token::Value, number.to_string()),
        Ipld::Bool(flag) => Span::new(Token::Value, if *flag { "true" } else { "false" }),
        Ipld::Null => Span::new(Token::Value, "null"),
        other => Span::new(
            Token::Value,
            serde_ipld_dagjson::to_vec(other)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default(),
        ),
    }
}

/// Whether a string reads as an entity URI rather than prose. Entities
/// are written bare in notation; everything else is quoted.
fn is_entity(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && text
            .split_once(':')
            .is_some_and(|(scheme, rest)| !scheme.is_empty() && !rest.is_empty())
}

/// Quote a scalar, collapsing newlines so one long value cannot run
/// away with the panel. The full text is still on the element's
/// `title`, which the panel sets.
fn quoted(text: &str) -> String {
    let flat = text.replace('\n', "\\n");
    format!("\"{}\"", flat.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, Ipld)]) -> BTreeMap<String, Ipld> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    fn line(document: &Document, index: usize) -> String {
        document.lines[index].text()
    }

    fn no_relations() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn an_entity_reads_as_the_assertion_that_would_make_it() {
        let document = entity(
            "prose",
            "did:key:zMk",
            &fields(&[("content", Ipld::String("Tonk".into()))]),
            &no_relations(),
        );
        assert_eq!(line(&document, 0), "prose!:");
        assert_eq!(line(&document, 1), "  this: did:key:zMk");
        assert_eq!(line(&document, 2), r#"  content: "Tonk""#);
    }

    #[test]
    fn every_value_line_names_the_field_it_belongs_to() {
        let document = entity(
            "prose",
            "did:key:zMk",
            &fields(&[("content", Ipld::String("Tonk".into()))]),
            &no_relations(),
        );
        assert_eq!(document.lines[0].field, None, "the head is not a field");
        assert_eq!(document.lines[1].field.as_deref(), Some("this"));
        assert_eq!(document.lines[2].field.as_deref(), Some("content"));
    }

    #[test]
    fn an_entity_uri_is_written_bare_and_prose_is_quoted() {
        let document = entity(
            "task",
            "did:key:zMk",
            &fields(&[
                ("owner", Ipld::String("did:key:zOther".into())),
                ("title", Ipld::String("Ship it".into())),
            ]),
            &no_relations(),
        );
        assert_eq!(line(&document, 2), "  owner: did:key:zOther");
        assert_eq!(line(&document, 3), r#"  title: "Ship it""#);
    }

    #[test]
    fn a_many_valued_field_lists_its_values_under_one_key() {
        let document = entity(
            "task",
            "did:key:zMk",
            &fields(&[(
                "tags",
                Ipld::List(vec![
                    Ipld::String("red".into()),
                    Ipld::String("blue".into()),
                ]),
            )]),
            &no_relations(),
        );
        assert_eq!(line(&document, 2), "  tags:");
        assert_eq!(line(&document, 3), r#"    - "red""#);
        assert_eq!(line(&document, 4), r#"    - "blue""#);
        assert!(
            document.lines[3..5]
                .iter()
                .all(|line| line.field.as_deref() == Some("tags")),
            "every line of the list highlights with its field"
        );
    }

    #[test]
    fn a_multiline_value_stays_on_one_line() {
        let document = entity(
            "prose",
            "did:key:zMk",
            &fields(&[("content", Ipld::String("a\nb".into()))]),
            &no_relations(),
        );
        assert_eq!(line(&document, 2), r#"  content: "a\nb""#);
    }

    #[test]
    fn a_declaration_reads_as_the_library_writes_it() {
        let descriptor = r#"{
            "description": "A prose document",
            "with": { "content": { "the": "io.gozala.prose/content", "as": "Text",
                                   "cardinality": "one" } }
        }"#;
        let document = declaration("concept", Some("prose"), descriptor);
        assert_eq!(
            document.text(),
            "concept!: &prose\n  \
             description: \"A prose document\"\n  \
             with:\n    \
             content:\n      \
             the: io.gozala.prose/content\n      \
             as: Text\n      \
             cardinality: one"
        );
    }

    #[test]
    fn optional_fields_declare_under_maybe() {
        let descriptor = r#"{
            "with":  { "a": { "as": "Text" } },
            "maybe": { "b": { "as": "Text" } }
        }"#;
        let document = declaration("concept", None, descriptor);
        let text = document.text();
        assert!(text.starts_with("concept!:\n"), "no anchor, no name");
        assert!(text.contains("  with:\n    a:"));
        assert!(text.contains("  maybe:\n    b:"));
    }

    #[test]
    fn a_declaration_line_names_its_field_so_it_can_be_highlighted() {
        let descriptor = r#"{ "with": { "content": { "as": "Text" } } }"#;
        let document = declaration("concept", Some("prose"), descriptor);
        let content: Vec<&Line> = document
            .lines
            .iter()
            .filter(|line| line.field.as_deref() == Some("content"))
            .collect();
        assert_eq!(content.len(), 2, "the field line and its one property");
    }

    #[test]
    fn a_property_the_shape_does_not_know_is_shown_anyway() {
        let descriptor = r#"{ "with": { "a": { "as": "Text", "keyed": "dictionary" } } }"#;
        let text = declaration("concept", None, descriptor).text();
        assert!(text.contains("keyed: dictionary"), "got:\n{text}");
    }

    #[test]
    fn a_value_line_carries_the_relation_its_field_projects() {
        let descriptor = r#"{ "with": { "content": { "the": "io.gozala.prose/content" } } }"#;
        let document = entity(
            "prose",
            "did:key:zMk",
            &fields(&[("content", Ipld::String("Tonk".into()))]),
            &relations(descriptor),
        );
        assert_eq!(
            document.lines[2].attribute.as_deref(),
            Some("io.gozala.prose/content"),
            "the field name is local to the concept; the attribute is the fact"
        );
        assert_eq!(document.lines[1].attribute, None, "`this` projects nothing");
    }

    #[test]
    fn a_keyed_collection_relates_through_its_domain() {
        let descriptor = r#"{
            "with": { "block": { "the": { "domain": "xyz.tonk.prose.block",
                                          "keyed": "dictionary" } } }
        }"#;
        assert_eq!(
            relations(descriptor).get("block").map(String::as_str),
            Some("xyz.tonk.prose.block"),
            "a keyed entry names its relation under `domain`"
        );
    }

    #[test]
    fn a_declaration_relates_its_fields_too() {
        let descriptor = r#"{ "with": { "content": { "the": "io.gozala.prose/content" } } }"#;
        let document = declaration("concept", Some("prose"), descriptor);
        assert!(
            document
                .lines
                .iter()
                .filter(|line| line.field.as_deref() == Some("content"))
                .all(|line| line.attribute.as_deref() == Some("io.gozala.prose/content"))
        );
    }

    #[test]
    fn an_unparseable_descriptor_says_so_rather_than_rendering_nothing() {
        let document = declaration("concept", Some("x"), "not json");
        assert!(document.text().contains("# descriptor did not parse"));
    }
}
