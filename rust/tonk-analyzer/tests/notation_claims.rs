//! The claims a library stores, written two ways: tonk's YAML, lowered
//! as the worker lowers a component (`library_claims_after` in
//! `tonk-worker/src/router/repository.rs`), and the notation, compiled
//! by `dialog-notation` and asserted as its notebook example asserts
//! what a document compiles to.
//!
//! Both sides end in a `dialog_corpus::Changes`: tonk's plans assert
//! into one, and a branch transaction is one (`Transaction::assert`
//! asserts into its changes, `Transaction::dispatch` into its
//! transients). So the two sets are what a commit would be handed.
//!
//! The test measures. It prints both sets as sorted `(the, of, is)`
//! triples per definition and how each YAML claim fares on the
//! notation side, and it asserts only that both sides produce claims.

use std::collections::{BTreeMap, BTreeSet};

use dialog_corpus::{Changes, Instruction, Statement as _, Value};
use dialog_notation::{Analyzer, Step, check};
use dialog_query::query::Output as _;
use dialog_query::{ConceptConclusion, DeductiveRule, InductiveRule, Parameters, Term};
use dialog_repository::Transient;
use tonk_schema::transact::{ApplicationPlan, Planner as _, Statement};

const CORE: &str = include_str!("../../tonk-core/assets/library/core.yaml");

/// One definition, from `notebook.yaml` and from
/// `design/macros/sketches/notebook.clj`, verbatim but for the
/// comments.
struct Definition {
    name: &'static str,
    yaml: &'static str,
    notation: &'static str,
}

const DEFINITIONS: &[Definition] = &[
    Definition {
        name: "concept notebook/block",
        yaml: r#"concept!: &notebook/block
  description: One markdown block of a notebook.
  with:
    notebook:
      description: The notebook this block belongs to.
      the: xyz.tonk.notebook.block/notebook
      as: entity
    source:
      description: The block's markdown source.
      the: xyz.tonk.notebook.block/source
      as: text
"#,
        notation: r#"; One markdown block of a notebook.
(concept (notebook/block ?b notebook: ?n source: ?s)
  ; The notebook this block belongs to.
  (xyz.tonk.notebook.block/notebook of: ?b is: ?n as: :entity cardinality: :one)
  ; The block's markdown source.
  (xyz.tonk.notebook.block/source of: ?b is: ?s as: :text cardinality: :one))
"#,
    },
    Definition {
        name: "command block/edit",
        yaml: r#"command!: &block/edit
  description: Write one notebook block's source.
  with:
    subject:
      description: The block entity, stamped on the element's data-subject.
      the: xyz.tonk.block.edit/subject
      as: entity
    source:
      description: The block's new markdown source.
      the: xyz.tonk.block.edit/source
      as: text
    notebook:
      description: |
        The notebook the block belongs to. Carried on every edit, not just a
        creating one: a block written with only its source does not match
        `tonk:notebook/block`, so it renders no row. Re-asserting it on an
        existing block is a no-op.
      the: xyz.tonk.block.edit/notebook
      as: entity
"#,
        notation: r#"; Write one notebook block's source.
(concept (block/edit subject: ?b source: ?s notebook: ?n)
  ; The block entity, stamped on the element's data-subject.
  (xyz.tonk.block.edit/subject of: ?c is: ?b as: :entity in: :control)
  ; The block's new markdown source.
  (xyz.tonk.block.edit/source of: ?c is: ?s as: :text in: :control)
  ; The notebook the block belongs to.
  (xyz.tonk.block.edit/notebook of: ?c is: ?n as: :entity in: :control))
"#,
    },
    Definition {
        name: "rule persist a block edit",
        yaml: r#"rule!:
  description: Persist a block edit onto its block.
  assert!: notebook/block
  when:
    - assert: block/edit
      where:
        subject: ?this
        source: ?source
        notebook: ?notebook
"#,
        notation: r#"; Persist a block edit onto its block.
(rule (assert (notebook/block ?b notebook: ?n source: ?s))
  (block/edit subject: ?b source: ?s notebook: ?n))
"#,
    },
    Definition {
        name: "assert route /notebook",
        yaml: r#"route!:
  path: "/notebook"
  concept: tonk:notebook/index-route
"#,
        notation: r#"(assert (route ?r path: "/notebook" concept: tonk:notebook/index-route)
  (when (id ?r path: "/notebook" concept: tonk:notebook/index-route)))
"#,
    },
];

/// What the notation side needs of `core.yaml` and does not compare:
/// the `route` concept a route asserts, written as `core.yaml` declares
/// it (`tonk:route` over `xyz.tonk.route/path` and
/// `xyz.tonk.route/concept`).
const NOTATION_PRELUDE: &str = r#"; A route: maps a path pattern to the model the SW mounts.
(concept (route ?r path: ?p concept: ?c)
  (xyz.tonk.route/path of: ?r is: ?p as: :text cardinality: :one)
  (xyz.tonk.route/concept of: ?r is: ?c as: :entity cardinality: :one))
"#;

/// A claim as a sortable `(the, of, is)` triple, and whether it is a
/// command's (dispatched, transient) or durable.
type Triple = (String, String, String);

#[derive(Default)]
struct Claims {
    durable: BTreeSet<Triple>,
    transient: BTreeSet<Triple>,
}

impl Claims {
    fn all(&self) -> BTreeSet<Triple> {
        self.durable.union(&self.transient).cloned().collect()
    }
}

fn shown(value: &Value) -> String {
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

/// The rule bodies printed beside the triples, by the hash `shown`
/// gives their bytes: what a `dialog.rule/source` holds, decoded.
static BODIES: std::sync::Mutex<BTreeMap<String, String>> = std::sync::Mutex::new(BTreeMap::new());

fn triples(changes: Changes) -> BTreeSet<Triple> {
    changes
        .into_instructions()
        .into_iter()
        .filter_map(|instruction| match instruction {
            Instruction::Assert(fact, _) => {
                if let Value::Bytes(bytes) = &fact.is
                    && fact.the.to_string() == "dialog.rule/source"
                {
                    let body = match (InductiveRule::decode(bytes), DeductiveRule::decode(bytes)) {
                        (Ok(rule), _) => serde_json::to_string(&rule),
                        (_, Ok(rule)) => serde_json::to_string(&rule),
                        (Err(error), _) => Ok(format!("<undecodable: {error}>")),
                    }
                    .unwrap_or_else(|error| format!("<{error}>"));
                    BODIES.lock().expect("bodies").insert(shown(&fact.is), body);
                }
                Some((fact.the.to_string(), fact.of.to_string(), shown(&fact.is)))
            }
            Instruction::Retract(_) => None,
        })
        .collect()
}

/// The YAML side, lowered as `library_claims_after` lowers a component:
/// `core.yaml` and the component analyzed as one document, the
/// component's statements from `first` on, a transient concept's
/// instances kept apart.
async fn yaml_claims(definitions: &[&str]) -> anyhow::Result<Vec<Claims>> {
    let parse = |file: &str, text: &str| {
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
    let unexpanded = tonk_notation::expand(&mut joined, &tonk_library::Bundled).await;
    anyhow::ensure!(unexpanded.is_empty(), "core.yaml includes: {unexpanded:?}");
    let first = joined.expressions.len();
    for definition in definitions {
        joined
            .expressions
            .extend(parse("notebook.yaml", definition)?.expressions);
    }
    let analyzed = tonk_analyzer::analyzer::analyze_local(&joined)
        .map_err(|error| anyhow::anyhow!("analyze: {error}"))?;
    let analysis = &analyzed.analysis;
    let mut bindings = Parameters::new();
    for (name, entity) in &analysis.variables {
        bindings.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
    }
    let transient = analysis.transient_entities();

    // Each definition is one expression. What it writes is what the
    // statements from it on write less what those after it write.
    let from = |index: usize| -> anyhow::Result<Claims> {
        let mut durable = Changes::new();
        let mut commands = Changes::new();
        for planned in analysis.statements_from(index) {
            let Statement::Assert(application) = planned.statement else {
                anyhow::bail!("a library retracts");
            };
            let plan = application
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
        Ok(Claims {
            durable: triples(durable),
            transient: triples(commands),
        })
    };
    let mut per = Vec::new();
    for index in 0..definitions.len() {
        let these = from(first + index)?;
        let later = from(first + index + 1)?;
        per.push(Claims {
            durable: these.durable.difference(&later.durable).cloned().collect(),
            transient: these
                .transient
                .difference(&later.transient)
                .cloned()
                .collect(),
        });
    }
    Ok(per)
}

/// The notation side: each definition checked in turn by one analyzer,
/// so a later one reads what an earlier one declared, and its steps
/// asserted as `examples/notebook/session.rs` asserts them: a write,
/// a rule and a transient mark into the changes, an issue into the
/// transients.
async fn notation_claims(definitions: &[&str]) -> anyhow::Result<Vec<Result<Claims, String>>> {
    // A write with a `when` asserts what its `when` matches, so it is
    // run against a branch: an empty one, as a library installed into
    // a new space finds it but for core.
    let (operator, profile) = dialog_peer::helpers::test_session_with_peer().await;
    let repository = dialog_peer::helpers::test_repo(&operator, &profile).await;
    let branch = repository.branch("main").open().perform(&operator).await?;
    let mut analyzer = Analyzer::new()?;
    let prelude = check(&mut analyzer, NOTATION_PRELUDE);
    anyhow::ensure!(
        prelude.refusals().next().is_none(),
        "the prelude is refused: {:?}",
        prelude.diagnostics
    );
    let mut per = Vec::new();
    for definition in definitions {
        let checked = check(&mut analyzer, definition);
        let refusals: Vec<String> = checked
            .refusals()
            .map(|diagnostic| format!("{diagnostic:?}"))
            .collect();
        if !refusals.is_empty() {
            per.push(Err(refusals.join("; ")));
            continue;
        }
        let mut changes = Changes::new();
        let mut transients = Changes::new();
        let mut unsubmitted = Vec::new();
        for step in checked.steps {
            match step {
                Step::Assert(statement) => statement.assert(&mut changes),
                Step::Retract(statement) => statement.retract(&mut changes),
                Step::Issue(statement) => statement.assert(&mut transients),
                Step::Deduce(rule) => rule.assert(&mut changes),
                Step::Induce(rule) => rule.assert(&mut changes),
                Step::Transient(concept) => Transient(concept).assert(&mut changes),
                Step::Query(_) => unsubmitted.push("a query"),
                Step::Write(write) => {
                    let mut layer = branch.query();
                    if let Some(rule) = &write.query.rule {
                        layer = layer.with(rule.clone());
                    }
                    let rows: Vec<ConceptConclusion> = layer
                        .select(write.query.select.clone())
                        .perform(&operator)
                        .try_vec()
                        .await?;
                    if rows.is_empty() {
                        unsubmitted.push("a write whose `when` matches nothing");
                    }
                    for row in &rows {
                        let statement = write.statement(row)?;
                        if write.retract {
                            statement.retract(&mut changes);
                        } else {
                            statement.assert(&mut changes);
                        }
                    }
                }
            }
        }
        if !unsubmitted.is_empty() {
            per.push(Err(format!("compiles to {}", unsubmitted.join(", "))));
            continue;
        }
        per.push(Ok(Claims {
            durable: triples(changes),
            transient: triples(transients),
        }));
    }
    Ok(per)
}

/// How a YAML claim fares on the notation side.
fn fate(claim: &Triple, notation: &BTreeSet<Triple>) -> &'static str {
    let (the, of, is) = claim;
    if notation.contains(claim) {
        "identical"
    } else if notation.iter().any(|(t, o, _)| t == the && o == of) {
        "same the and of, other is"
    } else if notation.iter().any(|(t, _, i)| t == the && i == is) {
        "same the and is, other of"
    } else if notation.iter().any(|(t, _, _)| t == the) {
        "same the, other of and is"
    } else {
        "not written"
    }
}

fn print(side: &str, claims: &Claims) {
    for (kind, set) in [
        ("durable", &claims.durable),
        ("transient", &claims.transient),
    ] {
        for (the, of, is) in set {
            println!("  {side} {kind:9} ({the}, {of}, {is})");
        }
    }
}

#[dialog_common::test]
async fn it_measures_the_claims_of_a_library_in_two_notations() -> anyhow::Result<()> {
    let yaml: Vec<&str> = DEFINITIONS.iter().map(|d| d.yaml).collect();
    let notation: Vec<&str> = DEFINITIONS.iter().map(|d| d.notation).collect();
    let yaml = yaml_claims(&yaml).await?;
    let notation = notation_claims(&notation).await?;

    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    for ((definition, yaml), notation) in DEFINITIONS.iter().zip(&yaml).zip(&notation) {
        println!("== {}", definition.name);
        print("yaml    ", yaml);
        let compiled = match notation {
            Ok(claims) => {
                print("notation", claims);
                claims.all()
            }
            Err(refused) => {
                println!("  notation refused: {refused}");
                BTreeSet::new()
            }
        };
        let yaml_all = yaml.all();
        for claim in &yaml_all {
            let fate = fate(claim, &compiled);
            *totals.entry(fate).or_default() += 1;
            println!("  fate {fate:28} ({}, {}, {})", claim.0, claim.1, claim.2);
        }
        for claim in compiled.difference(&yaml_all) {
            *totals.entry("only the notation writes").or_default() += 1;
            println!(
                "  fate {:28} ({}, {}, {})",
                "only the notation writes", claim.0, claim.1, claim.2
            );
        }
    }
    println!("== rule bodies");
    for (hash, body) in BODIES.lock().expect("bodies").iter() {
        println!("  {hash}\n    {body}");
    }
    println!("== totals");
    for (fate, count) in &totals {
        println!("  {fate:28} {count}");
    }
    assert!(yaml.iter().all(|claims| !claims.all().is_empty()));
    Ok(())
}
