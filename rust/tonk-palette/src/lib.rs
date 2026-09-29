//! The command palette's glue between tonk's data and `dialog-palette`.
//!
//! The `<tonk-palette>` element (an `element!:` in the profile library)
//! holds ordinary subscriptions — to `palette/verb`, `palette/argument`,
//! `palette/role`, `palette/noun`, the attributes those arguments name,
//! and the rows and `label` facet of every noun concept — and hands the
//! rows it has, as delivered, to [`propose`]. This crate joins them into
//! a [`dialog_palette::Registry`], parses the input, and answers with
//! ranked proposals, each carrying the transient claim that runs it.
//!
//! Nothing here reads a store: every row comes from the element's
//! subscriptions, so a change on the branch reaches the next keystroke
//! without any invalidation of its own.

use std::collections::{BTreeMap, BTreeSet};

use dialog_palette::{
    Argument, Candidate, Context, Grammar, Memory, Noun, Parse, Registry, Value, Verb, parse,
};
use ipld_core::ipld::Ipld;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

/// A subscription row as the bridge delivers it: the matched entity and
/// its projected fields. A cardinality-many field arrives as one row per
/// value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// The matched entity.
    pub this: String,
    /// Field values by term name.
    #[serde(default)]
    pub fields: BTreeMap<String, serde_json::Value>,
}

impl Row {
    fn text(&self, field: &str) -> Option<&str> {
        self.fields.get(field).and_then(serde_json::Value::as_str)
    }
}

/// A noun concept's rows and the `label` facet that renders them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConceptRows {
    /// The concept's `label` view template, when it has one.
    #[serde(default)]
    pub label: Option<String>,
    /// The concept's rows.
    #[serde(default)]
    pub rows: Vec<Row>,
}

/// Everything read from one branch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// The routing context the rows came from (`main@did:key:…`), echoed
    /// on each proposal so its claim is transacted on the same branch.
    pub branch: String,
    /// `palette/verb` rows: `this` is the command, `name` a word for it.
    #[serde(default)]
    pub verbs: Vec<Row>,
    /// `palette/argument` rows: `command`, `field`, `role`, `noun`.
    #[serde(default)]
    pub arguments: Vec<Row>,
    /// `palette/role` rows: `name`.
    #[serde(default)]
    pub roles: Vec<Row>,
    /// `palette/noun` rows: `this` is the concept, `name` a word for it.
    #[serde(default)]
    pub nouns: Vec<Row>,
    /// Attribute rows for argument fields: `id` (the selector) and
    /// `type` (the value type).
    #[serde(default)]
    pub attributes: Vec<Row>,
    /// Noun concepts, by entity.
    #[serde(default)]
    pub concepts: BTreeMap<String, ConceptRows>,
}

/// One keystroke's worth of input.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// What was typed.
    pub input: String,
    /// How many proposals to return.
    #[serde(default = "default_max")]
    pub max: usize,
    /// Where the palette was opened.
    #[serde(default)]
    pub context: Context,
    /// The branches read.
    #[serde(default)]
    pub sources: Vec<Source>,
    /// Suggestion memory: rows of `{input, verb, count}` in `fields`.
    #[serde(default)]
    pub memory: Vec<Row>,
}

fn default_max() -> usize {
    8
}

/// A ranked parse, ready to show and to run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// The branch to transact the claim on.
    pub branch: String,
    /// The command entity.
    pub command: String,
    /// The parse.
    pub parse: Parse,
    /// The transact request that runs it; `None` while an argument is
    /// empty.
    pub claim: Option<serde_json::Value>,
}

/// Separates a branch from a command in a verb id, so one command
/// declared on two branches stays two verbs.
const JOIN: char = '\u{1f}';

/// A field's command-side facts: its selector and value type.
#[derive(Debug, Clone)]
struct Field {
    selector: String,
    kind: String,
}

/// Parse `request.input` against the rows in `request` and return the
/// best proposals, best first.
pub fn propose(request: &Request) -> Vec<Proposal> {
    let (registry, fields) = registry(&request.sources);
    let memory = memory(&request.memory);
    let context = labelled(&request.context, &registry);
    parse(
        &Grammar::english(),
        &registry,
        &memory,
        &context,
        &request.input,
        request.max,
    )
    .into_iter()
    .map(|parse| {
        let (branch, command) = parse
            .verb
            .split_once(JOIN)
            .map(|(branch, command)| (branch.to_owned(), command.to_owned()))
            .unwrap_or_default();
        let claim = claim(&parse, &fields);
        Proposal {
            branch,
            command,
            parse,
            claim,
        }
    })
    .collect()
}

/// The context with each entity given the text a person reads for it:
/// a page knows which entity it shows, not how that entity is labelled,
/// and an anaphor substituted by an unlabelled entity would read as "".
fn labelled(context: &Context, registry: &Registry) -> Context {
    let label = |selection: &Option<dialog_palette::Selection>| {
        selection.clone().map(|mut selection| {
            if selection.text.is_empty()
                && let Some(entity) = &selection.entity
                && let Some(candidate) = registry
                    .candidates
                    .values()
                    .flatten()
                    .find(|candidate| &candidate.entity == entity)
            {
                selection.text = candidate.label.clone();
            }
            selection
        })
    };
    Context {
        selection: label(&context.selection),
        this: label(&context.this),
    }
}

/// Join every source's rows into one registry, and remember each
/// argument field's selector and type for building claims.
fn registry(sources: &[Source]) -> (Registry, BTreeMap<String, Field>) {
    let mut registry = Registry::default();
    let mut fields = BTreeMap::new();
    for source in sources {
        let roles: BTreeMap<&str, &str> = source
            .roles
            .iter()
            .filter_map(|row| Some((row.this.as_str(), row.text("name")?)))
            .collect();
        let attributes: BTreeMap<&str, Field> = source
            .attributes
            .iter()
            .filter_map(|row| {
                Some((
                    row.this.as_str(),
                    Field {
                        selector: row.text("id")?.to_owned(),
                        kind: row.text("type").unwrap_or("Text").to_owned(),
                    },
                ))
            })
            .collect();
        let mut noun_words: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for row in &source.nouns {
            if let Some(name) = row.text("name") {
                noun_words
                    .entry(row.this.as_str())
                    .or_default()
                    .insert(name);
            }
        }

        let mut names: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for row in &source.verbs {
            if let Some(name) = row.text("name") {
                let words = names.entry(row.this.as_str()).or_default();
                if !words.iter().any(|word| word == name) {
                    words.push(name.to_owned());
                }
            }
        }

        for (command, mut words) in names {
            words.sort();
            let mut arguments = Vec::new();
            let mut seen = BTreeSet::new();
            for row in source
                .arguments
                .iter()
                .filter(|row| row.text("command") == Some(command))
            {
                let (Some(field), Some(role)) = (
                    row.text("field").and_then(|field| attributes.get(field)),
                    row.text("role").and_then(|role| roles.get(role)),
                ) else {
                    continue;
                };
                // One argument per role, as in Ubiquity.
                if !seen.insert(*role) {
                    continue;
                }
                let noun = row.text("noun");
                let label = noun
                    .and_then(|noun| noun_words.get(noun))
                    .and_then(|words| words.iter().next())
                    .map(|word| (*word).to_owned())
                    .unwrap_or_else(|| field_name(&field.selector).to_owned());
                arguments.push(Argument {
                    role: (*role).to_owned(),
                    field: field.selector.clone(),
                    noun: match noun {
                        Some(concept) => Noun::Concept(concept.to_owned()),
                        None => Noun::Text,
                    },
                    label,
                });
                fields.insert(field.selector.clone(), field.clone());
            }
            registry.verbs.push(Verb {
                id: format!("{}{JOIN}{command}", source.branch),
                names: words,
                arguments,
            });
        }

        for (concept, rows) in &source.concepts {
            let candidates = registry.candidates.entry(concept.clone()).or_default();
            for candidate in candidates_of(rows) {
                if !candidates.iter().any(|row| row.entity == candidate.entity) {
                    candidates.push(candidate);
                }
            }
        }
    }
    (registry, fields)
}

/// The name half of a `domain/name` selector.
fn field_name(selector: &str) -> &str {
    selector.rsplit('/').next().unwrap_or(selector)
}

/// A concept's rows as candidates, each labelled by rendering the
/// concept's `label` facet with the row — the template `<tonk-display
/// view=label>` would render — and reading its text.
fn candidates_of(concept: &ConceptRows) -> Vec<Candidate> {
    let segments = concept.label.as_deref().map(tonk_template::parse_segments);
    let mut seen = BTreeSet::new();
    concept
        .rows
        .iter()
        .filter(|row| seen.insert(row.this.clone()))
        .filter_map(|row| {
            let label = match &segments {
                Some(segments) => {
                    let fields: BTreeMap<String, Ipld> = row
                        .fields
                        .iter()
                        .map(|(name, value)| (name.clone(), ipld(value)))
                        .collect();
                    text(&tonk_template::render_segments(
                        segments, &row.this, &fields,
                    ))
                }
                None => row
                    .fields
                    .values()
                    .find_map(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            };
            (!label.is_empty()).then(|| Candidate {
                entity: row.this.clone(),
                label,
            })
        })
        .collect()
}

fn ipld(value: &serde_json::Value) -> Ipld {
    match value {
        serde_json::Value::Null => Ipld::Null,
        serde_json::Value::Bool(value) => Ipld::Bool(*value),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(|n| Ipld::Integer(i128::from(n)))
            .unwrap_or_else(|| Ipld::Float(number.as_f64().unwrap_or_default())),
        serde_json::Value::String(value) => Ipld::String(value.clone()),
        serde_json::Value::Array(values) => Ipld::List(values.iter().map(ipld).collect()),
        serde_json::Value::Object(map) => Ipld::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), ipld(value)))
                .collect(),
        ),
    }
}

/// Rendered template → the text a person reads: tags dropped, entities
/// decoded, whitespace collapsed.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn memory(rows: &[Row]) -> Memory {
    let mut memory = Memory::default();
    for row in rows {
        if let (Some(input), Some(verb)) = (row.text("input"), row.text("verb")) {
            let count = row
                .fields
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            memory.set(input, verb, u32::try_from(count).unwrap_or(u32::MAX));
        }
    }
    memory
}

/// The transact request that asserts the parse's command, with its
/// fields' own selectors, as the FAB's inline claims do. Only for a
/// complete parse: a command missing a field matches nothing.
fn claim(parse: &Parse, fields: &BTreeMap<String, Field>) -> Option<serde_json::Value> {
    let mut with = serde_json::Map::new();
    let mut parameters = serde_json::Map::new();
    for filled in &parse.arguments {
        let field = fields.get(&filled.field)?;
        let key = field_name(&field.selector).to_owned();
        let value = match filled.value.as_ref()? {
            Value::Entity(entity) => entity.clone(),
            Value::Text(text) => text.clone(),
        };
        with.insert(
            key.clone(),
            json!({ "the": field.selector, "as": field.kind }),
        );
        parameters.insert(key, json!(value));
    }
    Some(json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": { "kind": "transient", "concept": { "with": with } },
                "parameters": parameters
            }
        }]
    }))
}

#[cfg(test)]
mod tests;
