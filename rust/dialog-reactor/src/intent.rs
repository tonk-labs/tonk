//! `intent/suggest` — what a typed command could mean, as query rows.
//!
//! A named query (`{ "predicate": "intent/suggest", "terms": {…} }`),
//! answered here by reading the branch's palette vocabulary with ordinary
//! concept queries and parsing with `tonk-lingo` (through
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
use dialog_repository::Stack;

use crate::BranchState;
use futures_util::TryStreamExt as _;
use ipld_core::ipld::Ipld;
use serde_json::{Value as Json, json};
use tonk_intent::{ConceptRows, Proposal, Request, Row, Source};

use crate::env::SelectProvider;
use crate::{Conclusion, FormulaError, Query, project};

/// The query's name, and every row's `this`.
pub const NAME: &str = "intent/suggest";

/// Rows for one `intent/suggest` query on a cached branch, read through
/// its stack so the state layer's intents and selections count.
pub async fn suggest<Env: SelectProvider>(
    state: &BranchState,
    env: &Env,
    query: &Query,
) -> Result<Vec<Conclusion>, FormulaError> {
    // A head that moved outside the stack (a commit through the branch
    // handle, a pull) is captured first, so the read is of the branch as
    // it is now.
    state
        .settle(env)
        .await
        .map_err(|error| FormulaError::Read(error.to_string()))?;
    let stack = state.stack();
    let this = text_term(query, "this");
    let now = number_term(query, "now");
    let max = number_term(query, "max").map_or(5, |max| max.max(1.0) as usize);
    let (mut source, memory) = load(stack, env).await?;

    // An interpreted expression (`intent/interpret` recorded it, with an
    // intent per command it could mean) supplies the input and selection,
    // and its intents carry what rules derived for each command's fields.
    // Without one, the terms supply them.
    let (input, selection, commands, site) = match text_term(query, "expression") {
        Some(expression) => {
            let recorded = expression_input(stack, env, &expression).await?;
            let input = recorded
                .or_else(|| text_term(query, "input"))
                .unwrap_or_default();
            let selection = expression_selection(stack, env, &expression).await?;
            let intents = intents(stack, env, &expression).await?;
            source.fragments = fragments(stack, env, &source, &intents).await?;
            // What labels the derived values: the concepts with a `label`
            // facet. Read only when there is something to label.
            if !source.fragments.is_empty() {
                source.concepts = labelled(stack, env).await?;
            }
            let commands: BTreeSet<String> =
                intents.into_iter().map(|(_, command)| command).collect();
            let site = first_entity(
                stack,
                env,
                &expression,
                "tonk.dialog.intent.expression/site",
            )
            .await?;
            (input, selection, Some(commands), site)
        }
        None => {
            let input = text_term(query, "input").unwrap_or_default();
            // The tab's site, whose `selection` (recorded by `site/select`)
            // the parser tries in every reading, as Ubiquity did.
            let site = text_term(query, "site");
            let selection = match &site {
                Some(site) => site_selection(stack, env, site).await?,
                None => None,
            };
            (input, selection, None, site)
        }
    };
    // What the page shows, most specific first: the entity its route names,
    // then the space. A field defaults to the first of these rules derived
    // for it.
    let mut shown = Vec::new();
    if let Some(site) = &site
        && let Some(entity) = first_entity(stack, env, site, "xyz.tonk.site/entity").await?
    {
        shown.push(entity);
    }
    shown.extend(this.clone());

    let request = Request {
        input: input.clone(),
        max,
        context: tonk_lingo::Context {
            selection: selection.map(|text| tonk_lingo::Selection { text, entity: None }),
            this: this.map(|entity| tonk_lingo::Selection {
                text: String::new(),
                entity: Some(entity),
            }),
        },
        shown,
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
    state: &BranchState,
    env: &Env,
    input: &str,
    site: Option<&str>,
) -> Result<Interpretation, FormulaError> {
    // A head that moved outside the stack (a commit through the branch
    // handle, a pull) is captured first, so the read is of the branch as
    // it is now.
    state
        .settle(env)
        .await
        .map_err(|error| FormulaError::Read(error.to_string()))?;
    let stack = state.stack();
    let (mut source, memory) = load(stack, env).await?;
    // Which commands, not what fills them: rules derive an entity field's
    // candidates onto the intent this records, so none exist yet. Read any
    // text there ("install notebook") as possibly naming one; `suggest`
    // then fills the field from what was derived, or leaves it empty.
    for attribute in &mut source.attributes {
        if attribute.fields.get("type").and_then(Json::as_str) == Some("Entity") {
            attribute.fields.insert("type".to_owned(), json!("Text"));
        }
    }
    let selection = match site {
        Some(site) => site_selection(stack, env, site).await?,
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
            context: tonk_lingo::Context {
                selection: selection
                    .clone()
                    .map(|text| tonk_lingo::Selection { text, entity: None }),
                this: None,
            },
            // Which commands, not their fields: no defaults to choose.
            shown: Vec::new(),
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

/// Everything the stack says about its commands, and the parser's memory.
async fn load<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
) -> Result<(Source, Vec<Row>), FormulaError> {
    let mut source = Source {
        branch: String::new(),
        ..Source::default()
    };
    source.verbs = rows(stack, env, named("tonk.dialog.intent.action/name", "many")).await?;
    source.attributes = rows(stack, env, attributes()).await?;
    source.arguments = arguments(stack, env).await?;
    let memory = rows(stack, env, choices()).await?;
    Ok((source, memory))
}

/// The input recorded for `expression`.
async fn expression_input<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
    expression: &str,
) -> Result<Option<String>, FormulaError> {
    first_text(
        stack,
        env,
        expression,
        "tonk.dialog.intent.expression/input",
    )
    .await
}

/// The selection recorded for `expression`, when there was one.
async fn expression_selection<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
    expression: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(first_text(
        stack,
        env,
        expression,
        "tonk.dialog.intent.expression/selection",
    )
    .await?
    .filter(|text| !text.is_empty()))
}

/// Every concept with a `label` facet, with its rows: what shows a value
/// a rule derived for a command's field.
async fn labelled<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
) -> Result<BTreeMap<String, ConceptRows>, FormulaError> {
    // A view's facets are a dictionary under `xyz.tonk.view`: the `label`
    // facet is the `xyz.tonk.view/label` attribute on the concept.
    let concepts: BTreeSet<String> = rows(
        stack,
        env,
        json!({
            "predicate": { "with": { "label": text("xyz.tonk.view/label", "one") } },
            "terms": { "this": var("this"), "label": var("label") }
        }),
    )
    .await?
    .into_iter()
    .map(|row| row.this)
    .collect();
    let mut labelled = BTreeMap::new();
    for concept in concepts {
        let rows = noun(stack, env, &concept).await?;
        labelled.insert(concept, rows);
    }
    Ok(labelled)
}

/// The one entity value of `the` on `this`, if any.
async fn first_entity<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
    this: &str,
    the: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        stack,
        env,
        json!({
            "predicate": { "with": { "value": entity(the, false) } },
            "terms": { "this": this, "value": var("value") }
        }),
    )
    .await?
    .into_iter()
    .find_map(|row| row.fields.get("value")?.as_str().map(str::to_owned)))
}

/// The one text value of `the` on `this`, if any.
async fn first_text<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
    this: &str,
    the: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        stack,
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
    stack: &Stack,
    env: &Env,
    expression: &str,
) -> Result<Vec<(String, String)>, FormulaError> {
    Ok(rows(
        stack,
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
/// attribute, read on the intent, with the field named after the
/// attribute. A concept's identity is its attributes (domain, name, type,
/// cardinality), not its name or field names, so this is the same concept
/// a library rule concludes for that field, and the rule answers it; the
/// rule binds the field by name, which is why the name must match.
async fn fragments<Env: SelectProvider>(
    stack: &Stack,
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
            // A rule binds its conclusion's fields by name, so the field is
            // read under the attribute's own name: a fragment concept names
            // its one field after the attribute (`subject` for
            // `…retitle/subject`).
            let name = selector.rsplit('/').next().unwrap_or(selector);
            let values = rows(
                stack,
                env,
                json!({
                    "predicate": { "with": { name: {
                        "the": selector, "as": "Entity", "cardinality": cardinality
                    } } },
                    "terms": { "this": intent, name: var(name) }
                }),
            )
            .await?;
            for value in values {
                let Some(value) = value.fields.get(name).and_then(Json::as_str) else {
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
    stack: &Stack,
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
    let conclusions: Vec<_> = stack
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

/// `intent/action`: a command and the words that say it.
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

/// The arguments commands declare: every concept field whose attribute
/// carries a role (`role:`, as `tonk.dialog.intent.attribute/role`), as a
/// row of `command`, `field` (the attribute entity) and `role` by name.
async fn arguments<Env: SelectProvider>(
    stack: &Stack,
    env: &Env,
) -> Result<Vec<Row>, FormulaError> {
    let roles: BTreeMap<String, String> = rows(
        stack,
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
        stack,
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
    stack: &Stack,
    env: &Env,
    site: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        stack,
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
    stack: &Stack,
    env: &Env,
    concept: &str,
) -> Result<ConceptRows, FormulaError> {
    let described = rows(
        stack,
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
        stack,
        env,
        json!({ "predicate": descriptor, "terms": terms }),
    )
    .await?;
    let facets = rows(
        stack,
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
