//! `intent/suggest` — what a typed command could mean, answered as rows.
//!
//! A command the page asserts through the transact path
//! (`tonk.dialog.intent.suggest/*`, declared as `intent/suggest` in the
//! library), answered by a provider the embedder registers on the
//! branch's stack: it reads the branch's palette vocabulary with ordinary
//! concept queries, parses with `tonk-lingo` (through [`tonk_intent`]) and
//! writes the readings into the stack's `session:` layer, where the page
//! reads them back with a concept query. [`Ask`] is the command as its
//! rows carry it, [`suggest`] answers it as [`Reading`]s, and [`answer`]
//! writes them as the rows the library declares (`intent/reading`, one
//! per reading, and `intent/answer`, one per ask). The ask's fields:
//!
//! - `input` — what was typed; `""` asks for what can be done without
//!   saying more (the palette's empty state).
//! - `subject` — the entity the page shows: what "this" means, and the
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
//! - `memory` — what was run from the palette, kept on another branch (the
//!   profile's), as JSON `{ "choices": [row…], "picks": [row…] }`; read
//!   along with this branch's own.
//!
//! Each reading, best first: `rank`, `score`, `command` (the command
//! entity), `text` (the reading as plain text), `display` (its segments as
//! JSON), `nouns` (role → kind of thing, as JSON), `input` (the words the
//! reading took for its verb, when it took any), `completion` (the input
//! extended by this reading, when it extends it), `listed` (whether it is
//! one of the values listed for an argument nothing was typed for),
//! `entities` (the entities its arguments are filled with, as JSON) and,
//! once nothing is missing, `claim` (the transact request that runs it, as
//! JSON). Every row names the ask it answers, so the tab that asked reads
//! its own answer and no other's; the rows of one asker (its site, else
//! its expression) are replaced by its next ask, so a session holds one
//! answer per asker.

use std::collections::{BTreeMap, BTreeSet};

use dialog_corpus::{Change, Changes, Entity, Fact, Pick, Relation, Update as _};
use dialog_query::Value;
use dialog_repository::{Branch, session_layer};
use futures_util::TryStreamExt as _;
use serde_json::{Value as Json, json};
use tonk_intent::{ConceptRows, Proposal, Request, Row, Source};

use crate::env::SelectProvider;
use crate::{FormulaError, Query, project};

/// The command's name, for what goes wrong answering it.
pub const NAME: &str = "intent/suggest";

/// The domain of the command's attributes (`tonk.dialog.intent.suggest/*`).
pub const ASK: &str = "tonk.dialog.intent.suggest";

/// The domain of an answer's attributes (`tonk.dialog.intent.answer/*`).
pub const ANSWER: &str = "tonk.dialog.intent.answer";

/// The domain of a reading's attributes (`tonk.dialog.intent.reading/*`).
pub const READING: &str = "tonk.dialog.intent.reading";

/// How many readings an ask gets when it does not say.
const DEFAULT_MAX: usize = 5;

/// One `intent/suggest`, as its rows carry it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    /// The command entity, which every row of its answer names.
    pub this: Entity,
    /// What was typed.
    pub input: String,
    /// The caller's clock in milliseconds, when given.
    pub now: Option<f64>,
    /// How many readings at most.
    pub max: usize,
    /// The entity the page shows, when there is one.
    pub subject: Option<String>,
    /// The tab's site, when the page has one.
    pub site: Option<String>,
    /// The expression `intent/interpret` recorded, when the line was
    /// interpreted.
    pub expression: Option<String>,
    /// What the palette ran before, as the branch that keeps it sent it.
    pub memory: Option<String>,
}

impl Ask {
    /// The ask the rows of command `this` make. A field the rows do not
    /// carry is absent; `max` defaults.
    pub fn from_rows(this: Entity, rows: &[Fact]) -> Self {
        let field = |name: &str| {
            let the = format!("{ASK}/{name}");
            rows.iter()
                .find(|row| row.of == this && row.the.to_string() == the)
                .map(|row| &row.is)
        };
        let text = |name: &str| match field(name) {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Entity(entity)) => Some(entity.to_string()),
            _ => None,
        };
        let number = |name: &str| match field(name) {
            Some(Value::Float(number)) => Some(*number),
            Some(Value::UnsignedInt(number)) => Some(*number as f64),
            Some(Value::SignedInt(number)) => Some(*number as f64),
            _ => None,
        };
        Self {
            input: text("input").unwrap_or_default(),
            now: number("now"),
            max: number("max").map_or(DEFAULT_MAX, |max| max.max(1.0) as usize),
            subject: text("subject"),
            site: text("site"),
            expression: text("expression"),
            memory: text("memory"),
            this,
        }
    }

    /// Who asks: the tab's site, else the palette opening, else the
    /// entity shown. One answer is held per asker.
    pub fn asker(&self) -> &str {
        self.site
            .as_deref()
            .or(self.expression.as_deref())
            .or(self.subject.as_deref())
            .unwrap_or_default()
    }
}

/// One reading of what was typed, as `suggest` answers it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Its place among the readings, best first, from 0.
    pub rank: usize,
    /// How well it reads.
    pub score: f64,
    /// The command it would run.
    pub command: String,
    /// The reading as plain text.
    pub text: String,
    /// Its segments, as JSON.
    pub display: String,
    /// Role → kind of thing, as JSON.
    pub nouns: String,
    /// The words the reading took for its verb, when it took any.
    pub input: Option<String>,
    /// The input extended by this reading, when it extends it.
    pub completion: Option<String>,
    /// The transact request that runs it, as JSON, once nothing is
    /// missing.
    pub claim: Option<String>,
    /// Whether it is one of the values listed for an argument nothing was
    /// typed for.
    pub listed: bool,
    /// The entities its arguments are filled with, as JSON.
    pub entities: String,
}

/// The readings of `ask` on `branch`, best first.
pub async fn suggest<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    ask: &Ask,
) -> Result<Vec<Reading>, FormulaError> {
    let this = ask.subject.clone();
    let now = ask.now;
    let max = ask.max;
    let (mut source, mut memory, mut picks) = load(branch, env).await?;
    if let Some(remembered) = &ask.memory {
        let remembered: Remembered =
            serde_json::from_str(remembered).map_err(|e| FormulaError::BadInput {
                formula: NAME.into(),
                reason: format!("memory: {e}"),
            })?;
        memory.extend(remembered.choices);
        picks.extend(remembered.picks);
    }

    // An interpreted expression (`intent/interpret` recorded it, with an
    // intent per command it could mean) supplies the input and selection,
    // and its intents carry what rules derived for each command's fields.
    // Without one, the ask supplies them.
    let (input, selection, commands, site) = match &ask.expression {
        Some(expression) => {
            let recorded = expression_input(branch, env, expression).await?;
            let input = recorded.unwrap_or_else(|| ask.input.clone());
            let selection = expression_selection(branch, env, expression).await?;
            let intents = intents(branch, env, expression).await?;
            source.fragments = fragments(branch, env, &source, &intents).await?;
            // What labels the derived values: the concepts with a `label`
            // facet. Read only when there is something to label.
            if !source.fragments.is_empty() {
                source.concepts = labelled(branch, env).await?;
            }
            let commands: BTreeSet<String> =
                intents.into_iter().map(|(_, command)| command).collect();
            let site = first_entity(
                branch,
                env,
                expression,
                "tonk.dialog.intent.expression/site",
            )
            .await?;
            (input, selection, Some(commands), site)
        }
        None => {
            let input = ask.input.clone();
            // The tab's site, whose `selection` (recorded by `site/select`)
            // the parser tries in every reading, as Ubiquity did.
            let site = ask.site.clone();
            let selection = match &site {
                Some(site) => site_selection(branch, env, site).await?,
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
        && let Some(entity) = first_entity(branch, env, site, "xyz.tonk.site/entity").await?
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
        picks,
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
        .map(|(rank, proposal)| reading(rank, proposal))
        .collect())
}

/// The answer to `ask`: the rows a provider writes into the stack's
/// `session:` layer, replacing the asker's previous answer among `held`,
/// the layer's rows as they stand. One `intent/answer` of the asker, and
/// one `intent/reading` per reading, every row naming the ask.
pub fn answer(ask: &Ask, readings: &[Reading], held: &Changes) -> Changes {
    let layer = session_layer();
    let key = asker_key(ask.asker());
    let answer_entity = format!("answer:{key}");
    let reading_prefix = format!("reading:{key}-");
    let mut changes = Changes::new();
    // The previous answer for this asker, row by row: a reading of the new
    // answer may lack a field the old one had.
    for (entity, the, change) in held.iter() {
        let name = entity.to_string();
        if name != answer_entity && !name.starts_with(&reading_prefix) {
            continue;
        }
        if let Change::Assert(value, _) = change {
            changes.dissociate_in(layer.clone(), the.clone(), entity.clone(), value.clone());
        }
    }
    let Ok(answer_entity) = answer_entity.parse::<Entity>() else {
        return changes;
    };
    let mut put = |of: &Entity, the: String, is: Value| {
        if let Ok(the) = the.parse::<Relation>() {
            changes.associate_in(layer.clone(), the, of.clone(), is, Pick::Last);
        }
    };
    put(
        &answer_entity,
        format!("{ANSWER}/ask"),
        Value::Entity(ask.this.clone()),
    );
    put(
        &answer_entity,
        format!("{ANSWER}/readings"),
        Value::UnsignedInt(readings.len() as u128),
    );
    for reading in readings {
        let Ok(of) = format!("{reading_prefix}{}", reading.rank).parse::<Entity>() else {
            continue;
        };
        let text = |name: &str, value: &str| {
            (format!("{READING}/{name}"), Value::String(value.to_owned()))
        };
        let command = match reading.command.parse::<Entity>() {
            Ok(command) => Value::Entity(command),
            Err(_) => Value::String(reading.command.clone()),
        };
        let mut rows = vec![
            (format!("{READING}/ask"), Value::Entity(ask.this.clone())),
            (
                format!("{READING}/rank"),
                Value::UnsignedInt(reading.rank as u128),
            ),
            (format!("{READING}/score"), Value::Float(reading.score)),
            (format!("{READING}/command"), command),
            text("text", &reading.text),
            text("display", &reading.display),
            text("nouns", &reading.nouns),
            (format!("{READING}/listed"), Value::Boolean(reading.listed)),
            text("entities", &reading.entities),
        ];
        if let Some(input) = &reading.input {
            rows.push(text("input", input));
        }
        if let Some(completion) = &reading.completion {
            rows.push(text("completion", completion));
        }
        if let Some(claim) = &reading.claim {
            rows.push(text("claim", claim));
        }
        for (the, is) in rows {
            put(&of, the, is);
        }
    }
    changes
}

/// The asker as a key safe in an entity: a digest of its name, so a site,
/// an expression and a DID all name their answer the same way.
fn asker_key(asker: &str) -> String {
    blake3::hash(asker.as_bytes()).to_hex()[..32].to_owned()
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
    let (mut source, memory, picks) = load(branch, env).await?;
    // Which commands, not what fills them: rules derive an entity field's
    // candidates onto the intent this records, so none exist yet. Read any
    // text there ("install notebook") as possibly naming one; `suggest`
    // then fills the field from what was derived, or leaves it empty.
    for attribute in &mut source.attributes {
        if attribute.fields.get("type").and_then(Json::as_str) == Some("entity:") {
            attribute.fields.insert("type".to_owned(), json!("text:"));
        }
    }
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
            picks,
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

/// What the palette ran before, as the `memory` term carries it from the
/// branch that keeps it.
#[derive(Debug, Default, serde::Deserialize)]
struct Remembered {
    #[serde(default)]
    choices: Vec<Row>,
    #[serde(default)]
    picks: Vec<Row>,
}

/// Everything `branch` says about its commands, and the parser's memory:
/// the commands run and the entities they were given.
async fn load<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Result<(Source, Vec<Row>, Vec<Row>), FormulaError> {
    let mut source = Source {
        branch: String::new(),
        ..Source::default()
    };
    source.verbs = rows(branch, env, named("tonk.dialog.intent.action/name", "many")).await?;
    source.attributes = rows(branch, env, attributes()).await?;
    source.arguments = arguments(branch, env).await?;
    let memory = rows(branch, env, choices()).await?;
    let picks = rows(branch, env, picks()).await?;
    Ok((source, memory, picks))
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

/// Every concept with a `label` facet, with its rows: what shows a value
/// a rule derived for a command's field.
async fn labelled<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
) -> Result<BTreeMap<String, ConceptRows>, FormulaError> {
    // A view's facets are a dictionary under `xyz.tonk.view`: the `label`
    // facet is the `xyz.tonk.view/label` attribute on the concept.
    let concepts: BTreeMap<String, String> = rows(
        branch,
        env,
        json!({
            "predicate": { "with": { "label": text("xyz.tonk.view/label", "one") } },
            "terms": { "this": var("this"), "label": var("label") }
        }),
    )
    .await?
    .into_iter()
    .filter_map(|row| {
        let label = row.fields.get("label")?.as_str()?.to_owned();
        Some((row.this, label))
    })
    .collect();
    let mut labelled = BTreeMap::new();
    for (concept, label) in concepts {
        let rows = noun(branch, env, &concept).await?;
        labelled.insert(
            concept,
            ConceptRows {
                label: Some(label),
                rows,
            },
        );
    }
    Ok(labelled)
}

/// The one entity value of `the` on `this`, if any.
async fn first_entity<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    this: &str,
    the: &str,
) -> Result<Option<String>, FormulaError> {
    Ok(rows(
        branch,
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
/// attribute, read on the intent, with the field named after the
/// attribute. A concept's identity is its attributes (domain, name, type,
/// cardinality), not its name or field names, so this is the same concept
/// a library rule concludes for that field, and the rule answers it; the
/// rule binds the field by name, which is why the name must match.
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
            let (Some(selector), Some("entity:")) =
                (attribute.fields.get("id").and_then(Json::as_str), kind)
            else {
                continue;
            };
            // A rule binds its conclusion's fields by name, so the field is
            // read under the attribute's own name: a fragment concept names
            // its one field after the attribute (`subject` for
            // `…retitle/subject`). The field's candidates are a set, so
            // the read selects `all`: under the attribute's own policy a
            // read would elect one of them and the palette would offer
            // one component to install, whichever the election picked.
            let name = selector.rsplit('/').next().unwrap_or(selector);
            let values = rows(
                branch,
                env,
                json!({
                    "predicate": { "with": { name: {
                        "the": selector, "as": "Entity", "select": "all"
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

/// One proposal as a reading.
fn reading(rank: usize, proposal: Proposal) -> Reading {
    let entities: Vec<&str> = proposal
        .parse
        .arguments
        .iter()
        .filter_map(|filled| match &filled.value {
            Some(tonk_lingo::Value::Entity(entity)) => Some(entity.as_str()),
            _ => None,
        })
        .collect();
    Reading {
        rank,
        score: proposal.parse.score,
        command: proposal.command.clone(),
        text: proposal.parse.display_text(),
        display: json_text(&proposal.parse.display),
        nouns: json_text(&proposal.nouns),
        input: proposal.parse.input.clone(),
        completion: proposal.completion.clone(),
        claim: proposal.claim.as_ref().map(ToString::to_string),
        listed: proposal.listed,
        entities: json_text(&entities),
    }
}

/// A structured value as JSON text: a row's values are scalars.
fn json_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
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
            "type": entity("db.attribute/as", true)
        } },
        "terms": { "this": var("this"), "id": var("id"), "type": var("type") }
    })
}

/// The arguments commands declare: every concept field whose attribute
/// carries a role (`role:`, as `tonk.dialog.intent.attribute/role`), as a
/// row of `command`, `field` (the attribute entity) and `role` by name.
async fn arguments<Env: SelectProvider>(
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

/// `intent/pick`: the entities commands run before were given, for ranking
/// the values listed for an argument.
fn picks() -> Json {
    json!({
        "predicate": { "with": {
            "command": entity("tonk.dialog.intent.pick/command", false),
            "value": entity("tonk.dialog.intent.pick/value", false),
            "time": { "the": "tonk.dialog.intent.pick/time", "as": "Float", "cardinality": "one" }
        } },
        "terms": {
            "this": var("this"), "command": var("command"), "value": var("value"),
            "time": var("time")
        }
    })
}

/// A noun concept's rows, resolved the way `<tonk-display>` resolves a
/// model: the descriptor from `db.meta/source`.
async fn noun<Env: SelectProvider>(
    branch: &Branch,
    env: &Env,
    concept: &str,
) -> Result<Vec<Row>, FormulaError> {
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
        return Ok(Vec::new());
    };
    let mut terms = json!({ "this": var("this") });
    if let Some(fields) = descriptor["with"].as_object() {
        for field in fields.keys() {
            terms[field] = var(field);
        }
    }
    rows(
        branch,
        env,
        json!({ "predicate": descriptor, "terms": terms }),
    )
    .await
}
