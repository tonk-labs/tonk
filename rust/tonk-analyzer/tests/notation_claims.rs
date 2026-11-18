//! The claims a library stores, written two ways: tonk's YAML, lowered
//! as the worker lowers a component (`library_claims_after` in
//! `tonk-worker/src/router/repository.rs`), and the notation, compiled
//! by `dialog-notation` and lowered by [`notation_library_claims`], a
//! back end that writes what `library_claims_after` writes: dialog's
//! rules and data from the notation's steps, and tonk's declaration
//! claims from what the notation declares.
//!
//! Both sides end in `dialog_corpus::Changes`, which is what a branch
//! transaction is handed: `Transaction::assert` asserts into its
//! changes and `Transaction::dispatch` into its transients.
//!
//! The bar runs tonk's `notebook.yaml` and the notebook sketch of the
//! dialog repository (`design/macros/sketches/notebook.clj`) and pins
//! how many of the YAML's claims the notation stores the same.

use std::collections::{BTreeMap, BTreeSet};

use dialog_corpus::{Changes, Instruction, Statement as _, Value};
use dialog_notation::{Declaration, View, parse_written};
use dialog_query::{DeductiveRule, InductiveRule, Parameters, Term};
use tonk_analyzer::analysis::ExpressionAnalysis;
use tonk_analyzer::notation::split;
use tonk_schema::transact::{ApplicationPlan, Planner as _, Statement};

const CORE: &str = include_str!("../../tonk-core/assets/library/core.yaml");
const NOTEBOOK: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");

/// The notebook in the notation, read from the dialog checkout the
/// workspace patches dialog to.
const SKETCH: &str = "/home/claude/notation/rust/dialog-notation/sketches/notebook.clj";

/// What the notebook reads of `core.yaml`, in the notation, from the
/// same checkout: read in front of the notebook, as a component is
/// read after core. Its claims are not measured.
const CORE_SKETCH: &str = "/home/claude/notation/rust/dialog-notation/sketches/core.clj";

type Triple = (String, String, String);

/// A set of claims: durable, and transient (commands, dispatched).
#[derive(Default, Clone, Debug)]
struct Claims {
    durable: BTreeSet<Triple>,
    transient: BTreeSet<Triple>,
}

impl Claims {
    fn all(&self) -> BTreeSet<Triple> {
        self.durable.union(&self.transient).cloned().collect()
    }
}

/// What a `dialog.rule/source` or a view's record holds, decoded, by
/// the hash `shown` gives its bytes.
static DECODED: std::sync::Mutex<BTreeMap<String, String>> = std::sync::Mutex::new(BTreeMap::new());

/// The whole text of each text value `shown` cut short, by what it
/// shows.
static TEXTS: std::sync::Mutex<BTreeMap<String, String>> = std::sync::Mutex::new(BTreeMap::new());

fn text_of(shown: &str) -> Option<String> {
    TEXTS.lock().expect("texts").get(shown).cloned()
}

fn shown(value: &Value) -> String {
    let said = cut(value);
    if let Value::String(text) = value {
        TEXTS
            .lock()
            .expect("texts")
            .insert(said.clone(), text.clone());
    }
    said
}

fn cut(value: &Value) -> String {
    let short = |reference: String| reference.chars().take(16).collect::<String>();
    match value {
        Value::Entity(entity) => entity.to_string(),
        Value::String(text) if text.chars().count() > 96 => format!(
            "text[{}] {}… #{}",
            text.len(),
            text.chars()
                .take(64)
                .collect::<String>()
                .replace('\n', "\\n"),
            short(value.reference().to_string())
        ),
        Value::String(text) => format!("{text:?}"),
        Value::Bytes(bytes) => format!(
            "bytes[{}] #{}",
            bytes.len(),
            short(value.reference().to_string())
        ),
        Value::Record(bytes) => format!(
            "record[{}] #{}",
            bytes.len(),
            short(value.reference().to_string())
        ),
        Value::Boolean(flag) => flag.to_string(),
        other => format!("{other:?}"),
    }
}

fn decode(the: &str, value: &Value) {
    let decoded = match (the, value) {
        ("dialog.rule/source", Value::Bytes(bytes)) => {
            match (InductiveRule::decode(bytes), DeductiveRule::decode(bytes)) {
                (Ok(rule), _) => serde_json::to_string(&rule).ok(),
                (_, Ok(rule)) => serde_json::to_string(&rule).ok(),
                (Err(error), _) => Some(format!("<undecodable: {error}>")),
            }
        }
        ("xyz.tonk.view/bindings", Value::Record(bytes)) => {
            Some(match tonk_template::bindings::Bindings::decode(bytes) {
                Ok(bindings) => serde_json::to_string(&bindings).unwrap_or_default(),
                Err(error) => format!("<undecodable: {error}>"),
            })
        }
        ("xyz.tonk.view/embeds", Value::Record(bytes)) => {
            Some(match tonk_template::embed::Embeds::decode(bytes) {
                Ok(embeds) => serde_json::to_string(&embeds).unwrap_or_default(),
                Err(error) => format!("<undecodable: {error}>"),
            })
        }
        _ => None,
    };
    if let Some(decoded) = decoded {
        DECODED
            .lock()
            .expect("decoded")
            .insert(shown(value), decoded);
    }
}

fn triples(changes: Changes) -> BTreeSet<Triple> {
    changes
        .into_instructions()
        .into_iter()
        .filter_map(|instruction| match instruction {
            Instruction::Assert(fact, _) => {
                let the = fact.the.to_string();
                decode(&the, &fact.is);
                Some((the, fact.of.to_string(), shown(&fact.is)))
            }
            Instruction::Retract(_) => None,
        })
        .collect()
}

/// The kind of a form, the same words on both sides.
fn yaml_kind(head: &str) -> &'static str {
    match head {
        "attribute" => "attribute",
        "concept" => "concept",
        "command" => "command",
        "rule" => "rule",
        "view" => "view",
        "event" => "event",
        "element" => "assert of an element",
        "route" => "assert of a route",
        _ => "assert of other data",
    }
}

/// One top-level form of a library and what it writes.
#[derive(Debug, Clone)]
struct Form {
    kind: &'static str,
    /// What pairs it with a form of the other side: a concept's name,
    /// or the kind and the place among the forms of that kind.
    key: String,
    claims: Claims,
    /// Why the form writes nothing, when it is refused.
    refused: Option<String>,
}

/// The YAML side, lowered as `library_claims_after` lowers a component:
/// `core.yaml` and the library analyzed as one document, each of the
/// library's expressions planned on its own, a transient concept's
/// instances kept apart.
async fn yaml_forms(library: &str) -> anyhow::Result<Vec<Form>> {
    let parse = |file: &str, text: &str| -> anyhow::Result<tonk_notation::Syntax> {
        let parsed = tonk_notation::parse_at(tonk_library::location(file), text);
        anyhow::ensure!(
            parsed.diagnostics.is_empty(),
            "{file} does not parse: {:?}",
            parsed.diagnostics.first()
        );
        parsed
            .syntax
            .ok_or_else(|| anyhow::anyhow!("{file} has no syntax"))
    };
    let mut joined = parse("core.yaml", CORE)?;
    let first = joined.expressions.len();
    let mut component = parse("notebook.yaml", library)?;
    let unexpanded = tonk_notation::expand(&mut component, &tonk_library::Bundled).await;
    anyhow::ensure!(unexpanded.is_empty(), "includes: {unexpanded:?}");
    joined.expressions.extend(component.expressions);
    let analyzed = tonk_analyzer::analyzer::analyze_local(&joined)
        .map_err(|error| anyhow::anyhow!("analyze: {error}"))?;
    let analysis = &analyzed.analysis;
    let mut bindings = Parameters::new();
    for (name, entity) in &analysis.variables {
        bindings.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
    }
    let transient = analysis.transient_entities();

    let mut ordinal: BTreeMap<&str, usize> = BTreeMap::new();
    let mut forms = Vec::new();
    for (expression, analyzed) in joined
        .expressions
        .iter()
        .zip(&analysis.expressions)
        .skip(first)
    {
        let head = expression.application().predicate.source.as_str();
        let kind = yaml_kind(head);
        let key = match (kind, expression) {
            ("concept" | "command" | "event", tonk_notation::Expression::Claim(claim)) => claim
                .anchor
                .as_ref()
                .map(|anchor| anchor.name.clone())
                .unwrap_or_default(),
            _ => {
                let at = ordinal.entry(kind).or_default();
                *at += 1;
                format!("{kind} {at}")
            }
        };
        let mut durable = Changes::new();
        let mut commands = Changes::new();
        if let ExpressionAnalysis::Assertion(node) = &analyzed.analysis {
            for statement in &node.analysis.claims {
                let Statement::Assert(application) = statement else {
                    anyhow::bail!("a library retracts");
                };
                let plan = application
                    .clone()
                    .plan(&bindings)
                    .map_err(|error| anyhow::anyhow!("plan: {error}"))?;
                let command = matches!(
                    &plan,
                    ApplicationPlan::Concept(concept)
                        if transient.contains(&concept.statement.predicate.this())
                );
                if command {
                    plan.assert(&mut commands);
                } else {
                    plan.assert(&mut durable);
                }
            }
        }
        forms.push(Form {
            kind,
            key,
            claims: Claims {
                durable: triples(durable),
                transient: triples(commands),
            },
            refused: None,
        });
    }
    Ok(forms)
}

/// The adjustment, made on the text: every line of a concept or a
/// command that says no `cardinality:` reads `cardinality: :one`.
fn read_one(form: &str) -> String {
    let concept = parse_written(form)
        .ok()
        .and_then(|(trees, _)| trees.first().map(|tree| tree.applies() == Some("concept")));
    if concept != Some(true) {
        return form.to_string();
    }
    // The lines are the forms two deep that say `of:`.
    let mut out = String::new();
    let mut depth = 0usize;
    let mut line_start: Option<usize> = None;
    let mut chars = form.char_indices().peekable();
    let mut copied = 0;
    while let Some((at, c)) = chars.next() {
        match c {
            ';' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                while let Some((_, c)) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '(' | '[' | '{' => {
                depth += 1;
                if depth == 2 {
                    line_start = Some(at);
                }
            }
            ')' | ']' | '}' => {
                if depth == 2
                    && let Some(begin) = line_start.take()
                {
                    let line = &form[begin..at];
                    if line.contains(" of:") && !line.contains("cardinality:") {
                        out.push_str(&form[copied..at]);
                        out.push_str(" cardinality: :one");
                        copied = at;
                    }
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    out.push_str(&form[copied..]);
    out
}

/// What the module gives for the notebook, with each form's claims as
/// triples.
struct NotationLibrary {
    durable: Vec<Instruction>,
    transient: Vec<Instruction>,
    forms: Vec<Form>,
    declarations: Vec<Declaration>,
}

/// The module's `notation_library_claims`, its writes with a `when`
/// read through a test operator.
async fn notation_library_claims(
    core: &str,
    library: &str,
    views: bool,
) -> anyhow::Result<NotationLibrary> {
    let (operator, _) = dialog_peer::helpers::test_session_with_peer().await;
    let library = tonk_analyzer::notation::notation_library_claims(core, library, views, &operator)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(NotationLibrary {
        durable: library.durable,
        transient: library.transient,
        forms: library
            .forms
            .into_iter()
            .map(|form| Form {
                kind: form.kind,
                key: form.key,
                claims: Claims {
                    durable: triples(form.durable),
                    transient: triples(form.transient),
                },
                refused: form.refused,
            })
            .collect(),
        declarations: library.declarations,
    })
}

/// How a YAML claim fares among the notation's.
fn fate(claim: &Triple, notation: &BTreeSet<Triple>) -> &'static str {
    let (the, of, is) = claim;
    if notation.contains(claim) {
        "identical"
    } else if notation
        .iter()
        .any(|(t, o, i)| t == the && (o == of || i == is))
    {
        "differently"
    } else {
        "not at all"
    }
}

/// Whether the bar being drawn has the view back end on.
static VIEWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn views_lowered() -> bool {
    VIEWS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Why a YAML claim the notation does not store the same is not.
fn difference(
    claim: &Triple,
    kind: &str,
    counterpart: Option<&Form>,
    notation: &BTreeSet<Triple>,
) -> String {
    let (the, of, is) = claim;
    let same_of = notation.iter().any(|(t, o, _)| t == the && o == of);
    let same_is = notation.iter().any(|(t, _, i)| t == the && i == is);
    if the.starts_with("xyz.tonk.view") && counterpart.is_some() && views_lowered() {
        return if same_of {
            "view: the text differs as bytes (the templates compare below)".into()
        } else {
            "view: stored on the pinned concept's entity in the YAML".into()
        };
    }
    if the.starts_with("xyz.tonk.view") {
        return match counterpart.and_then(|form| form.refused.as_ref()) {
            Some(error) if error.contains("is a description") => {
                "view: described, no back end yet".into()
            }
            Some(error) => format!("view refused: {}", brief(error)),
            None => "view: no back end yet".into(),
        };
    }
    if the.starts_with("xyz.tonk.event") {
        return "event: inline in a view, no back end yet".into();
    }
    let Some(counterpart) = counterpart else {
        return format!("{kind}: no form of its own in the notation");
    };
    if let Some(error) = &counterpart.refused {
        return format!("form refused: {}", brief(error));
    }
    let declaration = the.starts_with("db.attribute/")
        || the.starts_with("db.concept.")
        || the == "db.meta/concept";
    if the.starts_with("tonk.dialog.intent.") {
        return "palette metadata (`action:`, `role:`) has no spelling".into();
    }
    if declaration && !of.starts_with("concept:") && !of.starts_with("the:") {
        return "concept pinned to an entity in the YAML (`this:`)".into();
    }
    // A text the YAML writes as a block scalar ends in a newline.
    let trimmed = |is: &str| text_of(is).map(|text| text.trim_end_matches('\n').to_string());
    let newline = trimmed(is).is_some_and(|yaml| {
        text_of(is).is_some_and(|whole| whole != yaml)
            && notation
                .iter()
                .any(|(t, _, i)| t == the && text_of(i).as_deref() == Some(yaml.as_str()))
    });
    if newline {
        let same_entity = notation
            .iter()
            .any(|(t, o, i)| t == the && o == of && trimmed(i) == trimmed(is));
        return if same_entity {
            "text: the YAML's block scalar ends in a newline".into()
        } else {
            "text ends in a newline in the YAML, and the entity differs".into()
        };
    }
    if the.starts_with("db.concept.with/") && same_of {
        return "field labelled differently".into();
    }
    match the.as_str() {
        _ if the.starts_with("dialog.rule/") => "rule stored under another identity".into(),
        "db.name/referent" if same_of => "name points at another entity".into(),
        "db.name/referent" => "name not written".into(),
        "dialog.concept/transient" if same_is => "transient marker on another concept".into(),
        "dialog.concept/transient" => "transient marker not written".into(),
        "db.meta/description" if same_of => "description text differs".into(),
        "db.meta/description" if same_is => "description on another entity".into(),
        "db.meta/description" => "description not written".into(),
        _ if declaration && same_is => "declaration under another identity".into(),
        _ if declaration && same_of => "declaration value differs".into(),
        _ if declaration => "declaration not written".into(),
        _ if same_is => "data: entity derived differently".into(),
        _ if same_of => "data: value differs".into(),
        _ => "data not written".into(),
    }
}

/// An error, said short: its first line, cut at 100 characters.
fn brief(error: &str) -> String {
    let line = error.lines().next().unwrap_or_default();
    match line.char_indices().nth(100) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

/// The headline of a bar: of the claims the YAML stores, how many the
/// notation stores identically, differently and not at all, and how
/// many it stores that the YAML does not.
#[derive(Debug, PartialEq, Eq)]
struct Headline {
    yaml: usize,
    identical: usize,
    differently: usize,
    not_at_all: usize,
    only_notation: usize,
}

fn bar(heading: &str, yaml: &[Form], notation: &[Form]) -> Headline {
    println!("#### {heading}");
    let y: BTreeSet<Triple> = yaml.iter().flat_map(|form| form.claims.all()).collect();
    let n: BTreeSet<Triple> = notation.iter().flat_map(|form| form.claims.all()).collect();
    let mut by_kind: BTreeMap<&str, [usize; 6]> = BTreeMap::new();
    let mut by_difference: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    let mut seen: BTreeSet<Triple> = BTreeSet::new();
    let mut headline = Headline {
        yaml: y.len(),
        identical: 0,
        differently: 0,
        not_at_all: 0,
        only_notation: n.difference(&y).count(),
    };
    for form in yaml {
        let counterpart = notation.iter().find(|other| other.key == form.key);
        let row = by_kind.entry(form.kind).or_default();
        row[0] += 1;
        if counterpart.is_some_and(|other| other.refused.is_none()) {
            row[1] += 1;
        }
        for claim in form.claims.all() {
            if !seen.insert(claim.clone()) {
                continue;
            }
            let row = by_kind.entry(form.kind).or_default();
            row[2] += 1;
            match fate(&claim, &n) {
                "identical" => {
                    row[3] += 1;
                    headline.identical += 1;
                }
                fate => {
                    let differently = fate == "differently";
                    if differently {
                        row[4] += 1;
                        headline.differently += 1;
                    } else {
                        row[5] += 1;
                        headline.not_at_all += 1;
                    }
                    let why = difference(&claim, form.kind, counterpart, &n);
                    by_difference.entry(why.clone()).or_default()[usize::from(!differently)] += 1;
                    println!(
                        "  {fate:11} {:24} {why}: ({}, {}, {})",
                        form.key, claim.0, claim.1, claim.2
                    );
                }
            }
        }
    }
    let mut only_by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for form in notation {
        for claim in form.claims.all() {
            if !y.contains(&claim) {
                *only_by_kind.entry(form.kind).or_default() += 1;
                println!(
                    "  only-notation {:24} ({}, {}, {})",
                    form.key, claim.0, claim.1, claim.2
                );
            }
        }
    }
    println!(
        "## by kind of form: forms, with a notation counterpart that passes, claims, identical, differently, not at all, only the notation"
    );
    for (kind, row) in &by_kind {
        println!(
            "  | {kind} | {} | {} | {} | {} | {} | {} | {} |",
            row[0],
            row[1],
            row[2],
            row[3],
            row[4],
            row[5],
            only_by_kind.get(kind).copied().unwrap_or(0)
        );
    }
    println!("## notation forms: kind, forms, refused");
    let mut notation_kinds: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for form in notation {
        let row = notation_kinds.entry(form.kind).or_default();
        row.0 += 1;
        row.1 += usize::from(form.refused.is_some());
    }
    for (kind, (forms, refused)) in notation_kinds {
        println!("  | {kind} | {forms} | {refused} |");
    }
    println!("## refusals");
    for form in notation {
        if let Some(error) = &form.refused {
            println!("  {}: {}", form.key, brief(error));
        }
    }
    println!("## by kind of difference: differently, not at all");
    for (why, [differently, not]) in &by_difference {
        println!("  | {why} | {differently} | {not} |");
    }
    println!("## headline {headline:?}");
    headline
}

/// For each rule, the body each side stores, decoded.
fn rules(yaml: &[Form], notation: &[Form]) {
    println!("#### rules");
    let body = |form: &Form| -> Vec<String> {
        let decoded = DECODED.lock().expect("decoded");
        form.claims
            .durable
            .iter()
            .filter(|(the, _, _)| the == "dialog.rule/source")
            .map(|(_, of, is)| {
                format!(
                    "{of} {is}\n      {}",
                    decoded.get(is).cloned().unwrap_or_default()
                )
            })
            .collect()
    };
    for form in yaml.iter().filter(|form| form.kind == "rule") {
        println!("== {}", form.key);
        for said in body(form) {
            println!("  yaml     {said}");
        }
        if let Some(other) = notation.iter().find(|other| other.key == form.key) {
            let theirs = body(other);
            for said in &theirs {
                println!("  notation {said}");
            }
            let ours = body(form);
            if let ([yaml], [notation]) = (ours.as_slice(), theirs.as_slice()) {
                println!("  causes {:?}", causes(yaml, notation));
            }
        }
    }
}

/// What tells two bodies of one rule apart, read off their JSON.
fn causes(yaml: &str, notation: &str) -> Vec<&'static str> {
    let entity = |said: &str| {
        said.split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    };
    if entity(yaml) == entity(notation) {
        return vec!["identical"];
    }
    let count = |said: &str, what: &str| said.matches(what).count();
    let reads = |said: &str| -> BTreeSet<String> {
        said.split("\"the\":\"")
            .skip(1)
            .filter_map(|part| part.split('"').next())
            .map(str::to_string)
            .collect()
    };
    let mut causes = Vec::new();
    if count(notation, "entity/derive") > count(yaml, "entity/derive") {
        causes.push("the notation derives with `id` what the YAML reads as the command's `this`");
    }
    if count(notation, "{\"?\":{}}") < count(yaml, "{\"?\":{}}") {
        causes.push("a variable used once where the YAML writes `_`");
    }
    if count(notation, "\"pick\":\"all:\"") != count(yaml, "\"pick\":\"all:\"") {
        causes.push("another pick (`cardinality:`)");
    }
    if notation.contains("symbol:") {
        causes.push("a concept named as a symbol where the YAML gives its entity");
    }
    if count(yaml, "\"assert\":\"==\"") > count(notation, "\"assert\":\"==\"") {
        causes.push("the YAML binds with `==` what the notation binds directly");
    }
    if count(notation, "\"assert\":\"==\"") > count(yaml, "\"assert\":\"==\"") {
        causes.push(
            "the notation binds a collection's key with `==` where the YAML binds it directly",
        );
    }
    if count(yaml, "\"as\":") != count(notation, "\"as\":") {
        causes.push("a type left out on one side");
    }
    if reads(yaml) != reads(notation) {
        causes.push("the premises read other attributes");
    }
    if causes.is_empty() {
        causes.push("something else");
    }
    causes
}

/// The managing session's opinion of the bar for a view: template
/// equivalence. Both texts are parsed with tonk's renderer's parser
/// (`tonk_render::parse_fragment`) and compared as trees with each
/// element's attributes sorted, comments dropped (the renderer drops
/// them) and text trimmed, a text of whitespace alone dropped. The
/// `bindings` records are compared as data.
fn normalized(nodes: Vec<tonk_render::Node>) -> Vec<tonk_render::Node> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            tonk_render::Node::Comment(_) => None,
            tonk_render::Node::Text(text) => {
                let text = text.trim().to_string();
                (!text.is_empty()).then_some(tonk_render::Node::Text(text))
            }
            tonk_render::Node::Element(mut element) => {
                element.attrs.sort();
                element.children = normalized(element.children);
                Some(tonk_render::Node::Element(element))
            }
        })
        .collect()
}

/// Per view: whether its facet claims are equal, whether the templates
/// are equivalent, whether the bindings are equal as data.
fn equivalence(yaml: &[Form], notation: &[Form]) -> (usize, usize, usize) {
    println!("#### views: raw claims, equivalent templates, bindings as data");
    let (mut raw, mut trees, mut bindings) = (0, 0, 0);
    for form in yaml.iter().filter(|form| form.kind == "view") {
        let Some(other) = notation.iter().find(|other| other.key == form.key) else {
            continue;
        };
        let facets = |form: &Form| -> BTreeMap<String, String> {
            form.claims
                .durable
                .iter()
                .filter(|(the, _, _)| {
                    the.starts_with("xyz.tonk.view/") && the != "xyz.tonk.view/bindings"
                })
                .map(|(the, _, is)| (the.clone(), text_of(is).unwrap_or_default()))
                .collect()
        };
        let bound = |form: &Form| -> Option<String> {
            let decoded = DECODED.lock().expect("decoded");
            form.claims
                .durable
                .iter()
                .find(|(the, _, _)| the == "xyz.tonk.view/bindings")
                .and_then(|(_, _, is)| decoded.get(is).cloned())
        };
        let (ours, theirs) = (facets(form), facets(other));
        let same_raw = !ours.is_empty() && ours == theirs;
        let same_tree = !ours.is_empty()
            && ours.len() == theirs.len()
            && ours.iter().all(|(facet, text)| {
                theirs.get(facet).is_some_and(|other| {
                    normalized(tonk_render::parse_fragment(text))
                        == normalized(tonk_render::parse_fragment(other))
                })
            });
        let same_bindings = bound(form) == bound(other);
        raw += usize::from(same_raw);
        trees += usize::from(same_tree);
        bindings += usize::from(same_bindings);
        println!(
            "== {} raw {same_raw} equivalent {same_tree} bindings {same_bindings}",
            form.key
        );
        if !same_tree {
            for (facet, text) in &ours {
                println!(
                    "  yaml     {facet} {}",
                    tonk_render::serialize_nodes(&normalized(tonk_render::parse_fragment(text)))
                );
                if let Some(other) = theirs.get(facet) {
                    println!(
                        "  notation {facet} {}",
                        tonk_render::serialize_nodes(&normalized(tonk_render::parse_fragment(
                            other
                        )))
                    );
                }
            }
        }
        if !same_bindings {
            println!("  yaml     bindings {:?}", bound(form));
            println!("  notation bindings {:?}", bound(other));
        }
    }
    println!("## views: raw {raw}, equivalent {trees}, bindings {bindings}");
    (raw, trees, bindings)
}

/// For each view, what the YAML stores, and what the notation
/// describes.
fn views(yaml: &[Form], declarations: &[Declaration]) {
    println!("#### views");
    for form in yaml.iter().filter(|form| form.kind == "view") {
        println!("== yaml {}", form.key);
        for (the, of, is) in &form.claims.durable {
            println!("  ({the}, {of}, {is})");
            if let Some(text) = text_of(is) {
                println!("{text}");
            }
            if let Some(decoded) = DECODED.lock().expect("decoded").get(is) {
                println!("  {decoded}");
            }
        }
    }
    for form in yaml.iter().filter(|form| form.kind == "event") {
        println!("== yaml {}", form.key);
        for (the, of, is) in &form.claims.durable {
            println!("  ({the}, {of}, {is})");
        }
    }
    for declaration in declarations {
        if let Declaration::View { description, view } = declaration {
            println!("== notation view {} :{}", view.concept, view.name);
            println!("  description {description:?}");
            describe(view);
        }
    }
}

fn describe(view: &View) {
    println!(
        "  entity ?{} conditions {} variables {:?}",
        view.entity,
        view.conditions.len(),
        view.variables()
    );
    for event in view.events() {
        println!(
            "  event :on/{} -> {} {:?}",
            event.kind, event.command, event.fields
        );
    }
    for nested in view.nested() {
        println!("  nested {nested:?}");
    }
    println!("  template {:?}", view.template);
}

#[dialog_common::test]
async fn it_measures_the_notebook_in_two_notations() -> anyhow::Result<()> {
    // A sketch other than the checkout's, for telling apart what moves
    // the numbers: the bar is printed and the pins are not checked.
    let other = std::env::var("NOTEBOOK_BAR_SKETCH").ok();
    let sketch = std::fs::read_to_string(other.as_deref().unwrap_or(SKETCH))?;
    let core = std::fs::read_to_string(CORE_SKETCH)?;
    let yaml = yaml_forms(NOTEBOOK).await?;
    let notation = notation_library_claims(&core, &sketch, true).await?;
    VIEWS.store(true, std::sync::atomic::Ordering::Relaxed);
    let as_it_is = bar("the notation as it is", &yaml, &notation.forms);

    // The adjustment, on the text and not the notation's behaviour:
    // a line of a concept or a command with no `cardinality:` reads
    // `cardinality: :one`, the pick the YAML's fields get by default.
    let adjusted: String = split(&sketch).iter().map(|form| read_one(form)).collect();
    let adjusted = notation_library_claims(&core, &adjusted, true).await?;
    let read_one = bar(
        "ADJUSTED: lines with no `cardinality:` read as `:one`",
        &yaml,
        &adjusted.forms,
    );

    // The same two rows without the view back end, to tell what it
    // moves from what a change of the notation moves.
    let plain = notation_library_claims(&core, &sketch, false).await?;
    VIEWS.store(false, std::sync::atomic::Ordering::Relaxed);
    let without_views = bar("WITHOUT VIEWS: the notation as it is", &yaml, &plain.forms);
    let adjusted_text: String = split(&sketch)
        .iter()
        .map(|form| crate::read_one(form))
        .collect();
    let plain_adjusted = notation_library_claims(&core, &adjusted_text, false).await?;
    let without_views_read_one = bar("WITHOUT VIEWS: ADJUSTED", &yaml, &plain_adjusted.forms);
    println!("## without views {without_views:?} {without_views_read_one:?}");

    rules(&yaml, &adjusted.forms);
    let viewed = equivalence(&yaml, &notation.forms);
    views(&yaml, &notation.declarations);
    println!("#### decoded records");
    for (hash, decoded) in DECODED.lock().expect("decoded").iter() {
        if hash.starts_with("record") {
            println!("  {hash}\n    {decoded}");
        }
    }
    println!(
        "#### the library's claims: {} durable and {} transient as it is",
        notation.durable.len(),
        notation.transient.len()
    );

    if other.is_some() {
        println!("## pins not checked: another sketch");
        return Ok(());
    }
    // Pinned: a change to either side has to move these on purpose.
    assert_eq!(
        as_it_is,
        Headline {
            yaml: 376,
            identical: 107,
            differently: 211,
            not_at_all: 58,
            only_notation: 277,
        }
    );
    assert_eq!(
        read_one,
        Headline {
            yaml: 376,
            identical: 231,
            differently: 111,
            not_at_all: 34,
            only_notation: 153,
        }
    );
    // Of the five views: facet claims equal as bytes, templates
    // equivalent, bindings equal as data.
    assert_eq!(viewed, (0, 5, 5));
    Ok(())
}
