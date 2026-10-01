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

use std::collections::BTreeMap;

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
    let input = text_term(query, "input").unwrap_or_default();
    let this = text_term(query, "this");
    let now = number_term(query, "now");
    let max = number_term(query, "max").map_or(5, |max| max.max(1.0) as usize);

    let mut source = Source {
        branch: String::new(),
        ..Source::default()
    };
    source.verbs = rows(branch, env, named("tonk.dialog.intent.action/name", "many")).await?;
    source.nouns = rows(branch, env, named("tonk.dialog.intent.noun/name", "many")).await?;
    source.roles = rows(branch, env, named("tonk.dialog.intent.role/name", "one")).await?;
    source.attributes = rows(branch, env, attributes()).await?;
    source.arguments = rows(branch, env, arguments()).await?;
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

    let request = Request {
        input: input.clone(),
        max,
        context: dialog_lingo::Context {
            selection: None,
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
        .take(max)
        .enumerate()
        .map(|(rank, proposal)| row(rank, proposal))
        .collect())
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
            "type": text("db.attribute/type", "one")
        } },
        "terms": { "this": var("this"), "id": var("id"), "type": var("type") }
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
