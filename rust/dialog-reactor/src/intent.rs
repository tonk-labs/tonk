//! `intent/suggest` — what a typed command could mean, as query rows.
//!
//! A named query (`{ "predicate": "intent/suggest", "terms": {…} }`),
//! answered here by reading the branch's palette vocabulary with ordinary
//! concept queries and parsing with `dialog-lingo` (through
//! [`tonk_intent`]). The inputs are terms:
//!
//! - `input` — what was typed; `""` asks for what can be done without
//!   saying more (the palette's empty state).
//! - `this` — the entity the page shows: what "this" means, and the
//!   default for an argument it can fill.
//! - `now` — the caller's clock in milliseconds, for arguments that take
//!   the moment the command runs.
//! - `max` — how many rows at most (default 5).
//! - `site` — the tab's site; its recorded `selection` is tried in every
//!   reading, and stands in for "this" and "it".
//! - `expression` — an expression `intent/interpret` recorded. It supplies
//!   the input and the selection, and the readings are those of its
//!   intents, with the values rules derived onto each intent filling the
//!   command's fields.
//!
//! Each row is one reading, best first: `rank`, `score`, `command` (the
//! command entity), `text` (the reading as plain text), `display` (its
//! segments as JSON), `nouns` (role → kind of thing, as JSON), `input`
//! (the words the reading took for its verb, when it took any),
//! `completion` (the input extended by this reading, when it extends it)
//! and, once nothing is missing, `claim` (the transact request that runs
//! it, as JSON). Every row's `this` names the query, not an entity: a
//! reading is not a fact about anything.
//!
//! The shape is the contract, not where it is answered. The rows are what
//! a dialog premise of the same name would bind, so moving the resolution
//! into dialog changes nothing for the page that asks.

use std::collections::{BTreeMap, BTreeSet};

use dialog_query::{Term, Value};
use dialog_repository::Branch;
use futures_util::TryStreamExt as _;
use ipld_core::ipld::Ipld;
use serde_json::{Value as Json, json};
use tonk_intent::{ConceptRows, Proposal, Request, Row, Source};

use crate::env::SelectProvider;
use crate::{Conclusion, FormulaError, Query, project};

/// The query's name, and every row's `this`.
pub const NAME: &str = "intent/suggest";

/// Rows for one `intent/suggest` query on `branch`.
pub async fn suggest<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    query: &Query,
) -> Result<Vec<Conclusion>, FormulaError> {
    let this = text_term(query, "this");
    let now = number_term(query, "now");
    let max = number_term(query, "max").map_or(5, |max| max.max(1.0) as usize);
    let (mut source, memory) = load(branch, env).await?;

    // An interpreted expression (`intent/interpret` recorded it, with an
    // intent per command it could mean) supplies the input and selection,
    // and its intents carry what rules derived for each command's fields.
    // Without one, the terms supply them.
    let (input, selection, commands) = match text_term(query, "expression") {
        Some(expression) => {
            let recorded = expression_input(branch, env, &expression).await?;
            let input = recorded
                .or_else(|| text_term(query, "input"))
                .unwrap_or_default();
            let selection = expression_selection(branch, env, &expression).await?;
            let intents = intents(branch, env, &expression).await?;
            source.fragments = fragments(branch, env, &source, &intents).await?;
            let commands: BTreeSet<String> =
                intents.into_iter().map(|(_, command)| command).collect();
            (input, selection, Some(commands))
        }
        None => {
            let input = text_term(query, "input").unwrap_or_default();
            // The tab's site, whose `selection` (recorded by `site/select`)
            // the parser tries in every reading, as Ubiquity did.
            let selection = match text_term(query, "site") {
                Some(site) => site_selection(branch, env, &site).await?,
                None => None,
            };
            (input, selection, None)
        }
    };

    let request = Request {
        input: input.clone(),
        max,
        context: dialog_lingo::Context {
            selection: selection.map(|text| dialog_lingo::Selection { text, entity: None }),
            this: this.map(|entity| dialog_lingo::Selection {
                text: String::new(),
                entity: Some(entity),
            }),
        },
        sources: vec![source],
        memory,
        now,
    };
    let proposals = if input.is_empty() {
        tonk_intent::menu(&request)
    } else {
        tonk_intent::propose(&request)
    };
    Ok(proposals
        .into_iter()
        .filter(|proposal| {
            commands
                .as_ref()
                .is_none_or(|commands| commands.contains(&proposal.command))
        })
        .take(max)
        .enumerate()
        .map(|(rank, proposal)| row(rank, proposal))
        .collect())
}

/// What `intent/interpret` makes of an input on one branch: the commands
/// it could mean, and the selection of the tab it was typed in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Interpretation {
    /// Every command the input could mean, as its concept entity. With
    /// nothing typed, every command that has a name.
    pub commands: Vec<String>,
    /// The text selected in the tab's page, when there is some.
    pub selection: Option<String>,
}

/// Interpret `input`, typed in the tab whose site is `site`, against the
/// commands `branch` declares.
pub async fn interpret<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    input: &str,
    site: Option<&str>,
) -> Result<Interpretation, FormulaError> {
    let (source, memory) = load(branch, env).await?;
    let selection = match site {
        Some(site) => site_selection(branch, env, site).await?,
        None => None,
    };
    let mut commands: Vec<String> = Vec::new();
    let mut add = |command: String| {
        if !commands.contains(&command) {
            commands.push(command);
        }
    };
    if input.trim().is_empty() {
        for row in &source.verbs {
            add(row.this.clone());
        }
    } else {
        let request = Request {
            input: input.to_owned(),
            max: 20,
            context: dialog_lingo::Context {
                selection: selection
                    .clone()
                    .map(|text| dialog_lingo::Selection { text, entity: None }),
                this: None,
            },
            sources: vec![source],
            memory,
            now: None,
        };
        for proposal in tonk_intent::propose(&request) {
            add(proposal.command);
        }
    }
    Ok(Interpretation {
        commands,
        selection,
    })
}

/// Everything `branch` says about its commands, and the parser's memory.
async fn load<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Result<(Source, Vec<Row>), FormulaError> {
    let mut source = Source {
        branch: String::new(),
        ..Source::default()
    };
    source.verbs = rows(branch, env, named("tonk.dialog.intent.action/name", "many")).await?;
    source.nouns = rows(branch, env, named("tonk.dialog.intent.noun/name", "many")).await?;
    source.roles = rows(branch, env, named("tonk.dialog.intent.role/name", "one")).await?;
    source.attributes = rows(branch, env, attributes()).await?;
    source.arguments = rows(branch, env, arguments()).await?;
    source
        .arguments
        .extend(attribute_arguments(branch, env).await?);
    let nouns: Vec<String> = source
        .arguments
        .iter()
        .filter_map(|row| row.fields.get("noun")?.as_str().map(str::to_owned))
        .collect();
    for concept in nouns {
        if source.concepts.contains_key(&concept) {
            continue;
        }
        let rows = noun(branch, env, &concept).await?;
        source.concepts.insert(concept, rows);
    }
    let memory = rows(branch, env, choices()).await?;
    Ok((source, memory))
}

/// The input recorded for `expression`.
async fn expression_input<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    expression: &str,
) -> Result<Option<String>, FormulaError> {
    first_text(
        branch,
        env,
        expression,
        "tonk.dialog.intent.expression/input",
    )
    .await
}

/// The selection recorded for `expression`, when there was one.
async fn expression_selection<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    expression: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(first_text(
        branch,
        env,
        expression,
        "tonk.dialog.intent.expression/selection",
    )
    .await?
    .filter(|text| !text.is_empty()))
}

/// The one text value of `the` on `this`, if any.
async fn first_text<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    this: &str,
    the: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "value": text(the, "one") } },
            "terms": { "this": this, "value": var("value") }
        }),
    )
    .await?
    .into_iter()
    .find_map(|row| row.fields.get("value")?.as_str().map(str::to_owned)))
}

/// `expression`'s intents, as (intent entity, command entity).
async fn intents<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    expression: &str,
) -> Result<Vec<(String, String)>, FormulaError> {
    Ok(rows(
        branch,
        env,
        json!({
            "predicate": { "with": {
                "expression": entity("tonk.dialog.intent/expression", false),
                "command": entity("tonk.dialog.intent/command", false)
            } },
            "terms": { "this": var("this"), "expression": expression, "command": var("command") }
        }),
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        let command = row.fields.get("command")?.as_str()?.to_owned();
        Some((row.this, command))
    })
    .collect())
}

/// What rules derived onto each intent for its command's entity fields,
/// as `fragments` rows (`command`, `field`, `value`).
///
/// A field's values are the rows of a one-field concept on the field's own
/// attribute, read on the intent. A concept's identity is its attributes
/// (domain, name, type, cardinality), not its name or field names, so this
/// is the same concept a library rule concludes for that field, and the
/// rule answers it.
async fn fragments<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    source: &Source,
    intents: &[(String, String)],
) -> Result<Vec<Row>, FormulaError> {
    let mut found = Vec::new();
    for (intent, command) in intents {
        for argument in source
            .arguments
            .iter()
            .filter(|row| row.fields.get("command").and_then(Json::as_str) == Some(command))
        {
            let Some(field) = argument.fields.get("field").and_then(Json::as_str) else {
                continue;
            };
            let Some(attribute) = source.attributes.iter().find(|row| row.this == field) else {
                continue;
            };
            let kind = attribute.fields.get("type").and_then(Json::as_str);
            let (Some(selector), Some("Entity")) =
                (attribute.fields.get("id").and_then(Json::as_str), kind)
            else {
                continue;
            };
            let cardinality = attribute
                .fields
                .get("cardinality")
                .and_then(Json::as_str)
                .unwrap_or("one");
            let values = rows(
                branch,
                env,
                json!({
                    "predicate": { "with": { "value": {
                        "the": selector, "as": "Entity", "cardinality": cardinality
                    } } },
                    "terms": { "this": intent, "value": var("value") }
                }),
            )
            .await?;
            for value in values {
                let Some(value) = value.fields.get("value").and_then(Json::as_str) else {
                    continue;
                };
                let mut fragment = BTreeMap::new();
                fragment.insert("command".to_owned(), json!(command));
                fragment.insert("field".to_owned(), json!(field));
                fragment.insert("value".to_owned(), json!(value));
                found.push(Row {
                    this: String::new(),
                    fields: fragment,
                });
            }
        }
    }
    Ok(found)
}

/// One reading as a row.
fn row(rank: usize, proposal: Proposal) -> Conclusion {
    let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
    fields.insert("rank".into(), Ipld::Integer(rank as i128));
    fields.insert("score".into(), Ipld::Float(proposal.parse.score));
    fields.insert("command".into(), Ipld::String(proposal.command.clone()));
    fields.insert("text".into(), Ipld::String(proposal.parse.display_text()));
    fields.insert("display".into(), json_text(&proposal.parse.display));
    fields.insert("nouns".into(), json_text(&proposal.nouns));
    if let Some(input) = &proposal.parse.input {
        fields.insert("input".into(), Ipld::String(input.clone()));
    }
    if let Some(completion) = &proposal.completion {
        fields.insert("completion".into(), Ipld::String(completion.clone()));
    }
    if let Some(claim) = &proposal.claim {
        fields.insert("claim".into(), Ipld::String(claim.to_string()));
    }
    Conclusion {
        this: format!("db:{NAME}"),
        fields,
    }
}

/// A structured value as JSON text: a row's values are scalars.
fn json_text<T: serde::Serialize>(value: &T) -> Ipld {
    Ipld::String(serde_json::to_string(value).unwrap_or_default())
}

/// A text-valued input term (a string or an entity), if bound.
fn text_term(query: &Query, name: &str) -> Option<String> {
    match query.terms.get(name) {
        Some(Term::Constant(Value::String(text))) => Some(text.clone()),
        Some(Term::Constant(Value::Entity(entity))) => Some(entity.to_string()),
        _ => None,
    }
}

/// A number-valued input term, if bound.
fn number_term(query: &Query, name: &str) -> Option<f64> {
    match query.terms.get(name) {
        Some(Term::Constant(Value::Float(number))) => Some(*number),
        Some(Term::Constant(Value::UnsignedInt(number))) => Some(*number as f64),
        Some(Term::Constant(Value::SignedInt(number))) => Some(*number as f64),
        _ => None,
    }
}

/// Run one concept query, written as the page would write it, and read
/// its rows back as the palette's.
async fn rows<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    body: Json,
) -> Result<Vec<Row>, FormulaError> {
    let bad = |reason: String| FormulaError::BadInput {
        formula: NAME.into(),
        reason,
    };
    let wire: Query = serde_json::from_value(body).map_err(|e| bad(e.to_string()))?;
    let query = wire
        .into_concept_query()
        .map_err(|_| bad("not a concept query".into()))?;
    let terms = query.terms.clone();
    let conclusions: Vec<_> = branch
        .query()
        .select(tonk_schema::concept::QueryPlan::from(query))
        .perform(env)
        .try_collect()
        .await
        .map_err(|e| FormulaError::Read(e.to_string()))?;
    conclusions
        .iter()
        .map(|conclusion| {
            let row = serde_json::to_value(project(conclusion, &terms))
                .map_err(|e| bad(e.to_string()))?;
            serde_json::from_value(row).map_err(|e| bad(e.to_string()))
        })
        .collect()
}

fn var(name: &str) -> Json {
    json!({ "?": { "name": name } })
}

fn text(the: &str, cardinality: &str) -> Json {
    json!({ "the": the, "as": "Text", "cardinality": cardinality })
}

fn entity(the: &str, optional: bool) -> Json {
    let mut field = json!({ "the": the, "as": "Entity", "cardinality": "one" });
    if optional {
        field["optional"] = json!(true);
    }
    field
}

/// `intent/action`, `intent/noun`, `intent/role`: an entity and its words.
fn named(the: &str, cardinality: &str) -> Json {
    json!({
        "predicate": { "with": { "name": text(the, cardinality) } },
        "terms": { "this": var("this"), "name": var("name") }
    })
}

/// Every attribute's selector and type, for the fields arguments name.
fn attributes() -> Json {
    json!({
        "predicate": { "with": {
            "id": text("db.attribute/id", "one"),
            "type": text("db.attribute/type", "one"),
            "cardinality": text("db.attribute/cardinality", "one")
        } },
        "terms": {
            "this": var("this"), "id": var("id"), "type": var("type"),
            "cardinality": var("cardinality")
        }
    })
}

/// `intent/argument`.
fn arguments() -> Json {
    json!({
        "predicate": { "with": {
            "command": entity("tonk.dialog.intent.argument/command", false),
            "field": entity("tonk.dialog.intent.argument/field", false),
            "role": entity("tonk.dialog.intent.argument/role", false),
            "noun": entity("tonk.dialog.intent.argument/noun", true)
        } },
        "terms": {
            "this": var("this"), "command": var("command"), "field": var("field"),
            "role": var("role"), "noun": var("noun")
        }
    })
}

/// Arguments declared on the attributes themselves: every concept field
/// whose attribute carries a `tonk.dialog.intent.attribute/role`, as an
/// `intent/argument` row (`command`, `field`, `role` by name).
async fn attribute_arguments<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Result<Vec<Row>, FormulaError> {
    let roles: BTreeMap<String, String> = rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "role": text("tonk.dialog.intent.attribute/role", "one") } },
            "terms": { "this": var("this"), "role": var("role") }
        }),
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        let role = row.fields.get("role")?.as_str()?.to_owned();
        Some((row.this, role))
    })
    .collect();
    if roles.is_empty() {
        return Ok(Vec::new());
    }
    // A concept's fields, as `db.concept.with/<field>` → attribute entity.
    // A keyed field arrives as one `{ <field>: <attribute> }` object per
    // row.
    let fields = rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "field": {
                "the": { "domain": "db.concept.with", "keyed": "dictionary" },
                "as": "Entity", "cardinality": "one"
            } } },
            "terms": { "this": var("this"), "field": var("field") }
        }),
    )
    .await?;
    Ok(fields
        .into_iter()
        .flat_map(|row| {
            let command = row.this;
            let attributes: Vec<String> = match row.fields.get("field") {
                Some(Json::Object(entries)) => entries
                    .values()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect(),
                Some(Json::String(attribute)) => vec![attribute.clone()],
                _ => Vec::new(),
            };
            attributes
                .into_iter()
                .filter_map(|field| {
                    let role = roles.get(&field)?.clone();
                    let mut argument = BTreeMap::new();
                    argument.insert("command".to_owned(), json!(command));
                    argument.insert("field".to_owned(), json!(field));
                    argument.insert("role".to_owned(), json!(role));
                    Some(Row {
                        this: String::new(),
                        fields: argument,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect())
}

/// The selection recorded on `site`, if any.
async fn site_selection<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    site: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "selection": text("xyz.tonk.site/selection", "one") } },
            "terms": { "this": site, "selection": var("selection") }
        }),
    )
    .await?
    .into_iter()
    .find_map(|row| row.fields.get("selection")?.as_str().map(str::to_owned))
    .filter(|text| !text.is_empty()))
}

/// `intent/choice`: what was run before, for the parser's memory.
fn choices() -> Json {
    json!({
        "predicate": { "with": {
            "command": entity("tonk.dialog.intent.choice/command", false),
            "input": text("tonk.dialog.intent.choice/input", "one"),
            "time": { "the": "tonk.dialog.intent.choice/time", "as": "Float", "cardinality": "one" }
        } },
        "terms": {
            "this": var("this"), "command": var("command"), "input": var("input"),
            "time": var("time")
        }
    })
}

/// A noun concept's rows and its `label` facet, resolved the way
/// `<tonk-display>` resolves a model: the descriptor from `db.meta/source`.
async fn noun<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    concept: &str,
) -> Result<ConceptRows, FormulaError> {
    let described = rows(
        branch,
        env,
        json!({
            "predicate": { "with": {
                "concept": { "the": "db.meta/concept", "as": "Entity", "cardinality": "one" },
                "name": text("db.meta/name", "one"),
                "description": text("db.meta/description", "one"),
                "source": text("db.meta/source", "one"),
                "transient": { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
            } },
            "terms": { "this": concept, "name": var("name"), "source": var("source") }
        }),
    )
    .await?;
    let Some(descriptor) = described
        .iter()
        .find_map(|row| row.fields.get("source")?.as_str())
        .and_then(|source| serde_json::from_str::<Json>(source).ok())
    else {
        return Ok(ConceptRows::default());
    };
    let mut terms = json!({ "this": var("this") });
    if let Some(fields) = descriptor["with"].as_object() {
        for field in fields.keys() {
            terms[field] = var(field);
        }
    }
    let instances = rows(
        branch,
        env,
        json!({ "predicate": descriptor, "terms": terms }),
    )
    .await?;
    let facets = rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "show": {
                "the": { "domain": "xyz.tonk.view", "keyed": "dictionary" },
                "as": "Text", "cardinality": "one"
            } } },
            "terms": { "this": concept, "show": var("show"), "show/key": var("show/key") }
        }),
    )
    .await?;
    let label = facets
        .iter()
        .find(|row| row.fields.get("show/key").and_then(Json::as_str) == Some("label"))
        .and_then(|row| row.fields.get("show")?.as_str().map(str::to_owned));
    Ok(ConceptRows {
        label,
        rows: instances,
    })
}
