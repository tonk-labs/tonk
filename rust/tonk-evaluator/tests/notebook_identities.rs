//! `notebook.yaml` and `notebook.clj` side by side: the identities each
//! stores for the notebook library, its rules among them.

#![allow(deprecated)]
#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use dialog_capability::Subject;
use dialog_corpus::Value;
use dialog_effects::storage::Location;
use dialog_peer::helpers::{open_peer, test_storage, unique_name};
use dialog_query::query::Output as _;
use dialog_query::{ConceptQuery, Parameters, Term};
use dialog_repository::{Branch, RepositoryExt as _};
use support::{Claims, Env, notation_claims, yaml_claims};
use tonk_analyzer::notation::tour::{Answer, Cell, evaluate_on_branch};
use tonk_evaluator::evaluate::SyntaxEvaluateExt as _;
use tonk_notation::parse;
use tonk_schema::concept::{Concept, QueryPlan};
use tonk_schema::query_source::Source;

const NOTEBOOK_YAML: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");
const NOTEBOOK_CLJ: &str = include_str!("../../tonk-core/assets/library/notebook.clj");

/// The name each `db.name/referent` binds, to the entity it binds.
fn names(claims: &Claims) -> BTreeMap<String, String> {
    claims
        .iter()
        .filter_map(|(entity, claims)| {
            let name = entity.strip_prefix("id:")?;
            claims.iter().find_map(|(the, is)| {
                (the == "db.name/referent").then(|| (name.to_owned(), entity_of(is)))
            })
        })
        .collect()
}

fn entity_of(shown: &str) -> String {
    shown
        .trim_start_matches("Entity(")
        .trim_end_matches(')')
        .to_owned()
}

/// Each stored attribute's relation, to its entity.
fn attributes(claims: &Claims) -> BTreeMap<String, String> {
    claims
        .iter()
        .filter_map(|(entity, claims)| {
            claims.iter().find_map(|(the, is)| {
                (the == "db.attribute/id").then(|| {
                    (
                        serde_json::from_str::<String>(is).expect("text"),
                        entity.clone(),
                    )
                })
            })
        })
        .collect()
}

/// A stored rule: its kind and its head (`dialog.rule/derives` or
/// `dialog.rule/induces`), and the source it is stored with.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Rule {
    kind: &'static str,
    head: String,
    source: String,
}

fn rules(claims: &Claims) -> BTreeMap<String, Rule> {
    claims
        .iter()
        .filter_map(|(entity, claims)| {
            let source = claims
                .iter()
                .find_map(|(the, is)| (the == "dialog.rule/source").then(|| is.clone()))?;
            let (kind, head) = claims.iter().find_map(|(the, is)| match the.as_str() {
                "dialog.rule/derives" => Some(("deduces", entity_of(is))),
                "dialog.rule/induces" => Some(("induces", entity_of(is))),
                _ => None,
            })?;
            Some((entity.clone(), Rule { kind, head, source }))
        })
        .collect()
}

/// A rule's JSON with every variable's name left out, and every field a
/// rule numbers (`~0`, `#1`) listed by what it holds: what is left is the
/// rule's shape, whatever its variables are called.
fn shape(json: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value as J;
    match json {
        J::Object(map) if map.contains_key("?") => serde_json::json!({ "?": {} }),
        J::Object(map) if !map.is_empty() && map.keys().all(|key| numbered(key)) => {
            let mut held: Vec<J> = map.values().map(shape).collect();
            held.sort_by_key(|value| value.to_string());
            J::Array(held)
        }
        J::Object(map) => J::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), shape(value)))
                .collect(),
        ),
        J::Array(items) => J::Array(items.iter().map(shape).collect()),
        other => other.clone(),
    }
}

fn numbered(key: &str) -> bool {
    let rest = key.trim_start_matches(['~', '#']);
    rest.len() < key.len() && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn json_of(source: &str) -> serde_json::Value {
    serde_json::from_str(source.strip_prefix("rule ").expect("a decoded rule")).expect("JSON")
}

fn strings(json: &serde_json::Value, key: &str, into: &mut BTreeSet<String>) {
    use serde_json::Value as J;
    match json {
        J::Object(map) => {
            for (k, v) in map {
                if k == key
                    && let J::String(s) = v
                {
                    into.insert(s.clone());
                }
                strings(v, key, into);
            }
        }
        J::Array(items) => items.iter().for_each(|item| strings(item, key, into)),
        _ => {}
    }
}

/// What tells two rules of one head apart.
fn causes(yaml: &str, notation: &str) -> Vec<String> {
    let (a, b) = (json_of(yaml), json_of(notation));
    if shape(&a) == shape(&b) {
        return vec!["only the variables' names and numbering".into()];
    }
    let mut causes = Vec::new();
    let head = |json: &serde_json::Value| {
        json.as_object()
            .and_then(|map| map.keys().find(|key| *key != "when").cloned())
            .unwrap_or_default()
    };
    if head(&a) != head(&b) {
        causes.push(format!(
            "the head {} where the YAML's {}",
            head(&b),
            head(&a)
        ));
    }
    let (mut ra, mut rb) = (BTreeSet::new(), BTreeSet::new());
    strings(&a, "the", &mut ra);
    strings(&b, "the", &mut rb);
    let only_yaml: Vec<_> = ra.difference(&rb).collect();
    let only_clj: Vec<_> = rb.difference(&ra).collect();
    if !only_yaml.is_empty() || !only_clj.is_empty() {
        causes.push(format!(
            "reads {only_clj:?} where the YAML reads {only_yaml:?}"
        ));
    }
    let count = |text: &str, what: &str| text.matches(what).count();
    for (what, said) in [
        ("\"as\":", "a type"),
        ("\"==\"", "an `==` binding"),
        ("\"cardinality\":", "a cardinality"),
        ("entity/derive", "an `id` derivation"),
        ("symbol:", "a concept named as a symbol"),
    ] {
        let (x, y) = (count(yaml, what), count(notation, what));
        if x != y {
            causes.push(format!("{said}: {x} in the YAML, {y} in the notation"));
        }
    }
    let premises = |json: &serde_json::Value| json["when"].as_array().map_or(0, Vec::len);
    if premises(&a) != premises(&b) {
        causes.push(format!(
            "{} premises in the YAML, {} in the notation",
            premises(&a),
            premises(&b)
        ));
    }
    if causes.is_empty() {
        causes.push("the premises' shape".into());
    }
    causes
}

/// Why a rule of the YAML and the notation's rule for the same head are
/// stored as two rules, in one word: what the test pins.
fn category(yaml: &str, notation: &str) -> &'static str {
    let (a, b) = (shape(&json_of(yaml)), shape(&json_of(notation)));
    if a == b {
        return "variables";
    }
    let sorted = |json: &serde_json::Value| {
        let mut json = json.clone();
        if let Some(premises) = json["when"].as_array_mut() {
            premises.sort_by_key(|premise| premise.to_string());
        }
        json
    };
    if sorted(&a) == sorted(&b) {
        return "premise order";
    }
    if yaml.matches("\"==\"").count() != notation.matches("\"==\"").count() {
        return "== binding";
    }
    if notation.contains("symbol:") {
        return "command metadata and a concept named as a symbol";
    }
    "meaning"
}

/// The three ways an entity can fare: kept by both sides, moved to
/// another entity, or stored by one side only.
#[derive(Debug, Default, PartialEq, Eq)]
struct Fate {
    kept: BTreeSet<String>,
    moved: BTreeSet<String>,
    yaml_only: BTreeSet<String>,
    notation_only: BTreeSet<String>,
}

fn fate(ours: &BTreeMap<String, String>, theirs: &BTreeMap<String, String>) -> Fate {
    let mut fate = Fate::default();
    for key in ours.keys().chain(theirs.keys()) {
        let key = key.clone();
        match (ours.get(&key), theirs.get(&key)) {
            (Some(a), Some(b)) if a == b => fate.kept.insert(key),
            (Some(_), Some(_)) => fate.moved.insert(key),
            (Some(_), None) => fate.yaml_only.insert(key),
            (None, Some(_)) => fate.notation_only.insert(key),
            (None, None) => false,
        };
    }
    fate
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// The notebook's names, attributes and rules, each side as the worker
/// installs it, and what a migration from the YAML would have to rewrite.
///
/// Names (`db.name/referent`): every concept the notebook declares
/// without pinning it to an entity keeps its entity. The two route
/// concepts the YAML pins (`this: tonk:notebook/route`, gaps row 6) move
/// to the entity their attributes give. The element moves from its body's
/// hash to the entity the notation names it by. The YAML's names for
/// attributes, its four events and its session concept
/// `notebook/retitled` have no notation counterpart.
///
/// Attributes: every relation the notation declares keeps its entity.
/// `xyz.tonk.invocation/output`, which only the YAML's session rule
/// writes, is the YAML's alone. Five keep their entity and differ in
/// what is said of them: three descriptions reworded in the sketch, and
/// the intent role (`role:`) the YAML gives the retitle command's
/// fields, which the notation has no spelling for.
///
/// Rules: a rule is stored under the entity its body hashes to, so the
/// YAML's rule and the notation's for the same head are one rule only
/// when the bodies are the same; otherwise a space that holds the YAML's
/// and installs the notation's holds both, and both fire, until the old
/// one is taken out. Three of seventeen are one rule. The other fourteen:
///
/// - two differ only in how their variables are named or numbered
///   (a variable the YAML writes `_` is named in the notation);
/// - one only in the order of its premises;
/// - seven where the notation binds a collection's key with `==` and the
///   YAML binds it in the pattern;
/// - one where the notation's command carries no intent metadata and
///   names a concept as a symbol;
/// - one that means something else: the notation's rule for
///   `xyz.tonk.notebook.block/at` requires `insert/next` to be
///   `case:none` where the YAML's leaves it free;
/// - one YAML rule, retracting a removed block's notebook and source
///   together, is two in the notation, one per field (a retract by
///   entity alone retracts each field's claims);
/// - one YAML rule writes the retitle's output to the session layer
///   (`in: "session:"`, gaps row 16) and has no counterpart.
#[dialog_common::test]
async fn the_notebook_keeps_its_unpinned_concepts_and_moves_most_rules() {
    let yaml = yaml_claims("notebook.yaml", NOTEBOOK_YAML).await;
    let notation = notation_claims(NOTEBOOK_CLJ).await;

    let names = fate(&names(&yaml), &names(&notation));
    println!("names {names:#?}");
    assert_eq!(
        names,
        Fate {
            kept: set(&[
                "block/chain",
                "block/edit",
                "block/insert",
                "block/move",
                "block/place",
                "block/position",
                "block/remove",
                "block/stray-entry",
                "notebook",
                "notebook/block",
                "notebook/blocks",
                "notebook/named",
                "notebook/retitle",
            ]),
            moved: set(&["notebook-save", "notebook/index-route", "notebook/route"]),
            yaml_only: set(&[
                "block/source",
                "notebook/retitle-subject",
                "notebook/retitled",
                "notebook/title",
                "on/block-edit",
                "on/block-place",
                "on/block-remove",
                "on/notebook-retitle",
            ]),
            notation_only: BTreeSet::new(),
        }
    );

    let (ours, theirs) = (attributes(&yaml), attributes(&notation));
    let fated = fate(&ours, &theirs);
    assert_eq!(fated.moved, BTreeSet::new());
    assert_eq!(fated.yaml_only, set(&["xyz.tonk.invocation/output"]));
    assert_eq!(fated.notation_only, BTreeSet::new());
    let restated: BTreeSet<String> = fated
        .kept
        .iter()
        .filter(|relation| yaml[&ours[*relation]] != notation[&theirs[*relation]])
        .cloned()
        .collect();
    assert_eq!(
        restated,
        set(&[
            "xyz.tonk.block.edit/notebook",
            "xyz.tonk.block.remove/key",
            "xyz.tonk.notebook.insert/prev",
            "xyz.tonk.notebook.retitle/subject",
            "xyz.tonk.notebook.retitle/title",
        ])
    );

    let (ours, theirs) = (rules(&yaml), rules(&notation));
    assert_eq!((ours.len(), theirs.len()), (17, 17));
    let one: BTreeSet<&String> = ours
        .keys()
        .filter(|rule| theirs.contains_key(*rule))
        .collect();
    assert_eq!(one.len(), 3, "rules stored as one: {one:?}");
    for rule in &one {
        assert_eq!(ours[*rule], theirs[*rule], "one entity, one stored source");
    }

    // Pair each other YAML rule with the notation's for its head that
    // differs least, and say why the two are two.
    let mut categories: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unpaired_yaml = 0;
    let mut unpaired_notation = 0;
    let heads: BTreeSet<(&str, &str)> = ours
        .values()
        .chain(theirs.values())
        .map(|rule| (rule.kind, rule.head.as_str()))
        .collect();
    for (kind, head) in heads {
        let of = |rules: &BTreeMap<String, Rule>| -> Vec<String> {
            rules
                .iter()
                .filter(|(entity, rule)| {
                    rule.kind == kind && rule.head == head && !one.contains(entity)
                })
                .map(|(entity, _)| entity.clone())
                .collect()
        };
        let mut left = of(&theirs);
        for rule in of(&ours) {
            let best = left
                .iter()
                .enumerate()
                .min_by_key(|(_, other)| causes(&ours[&rule].source, &theirs[*other].source).len())
                .map(|(at, _)| at);
            match best {
                Some(at) => {
                    let other = left.remove(at);
                    let why = category(&ours[&rule].source, &theirs[&other].source);
                    println!("{kind} {head}: {rule} / {other}: {why}");
                    for cause in causes(&ours[&rule].source, &theirs[&other].source) {
                        println!("    {cause}");
                    }
                    *categories.entry(why).or_default() += 1;
                }
                None => {
                    println!("{kind} {head}: {rule} has no counterpart");
                    unpaired_yaml += 1;
                }
            }
        }
        for other in left {
            println!("{kind} {head}: {other} is the notation's alone");
            unpaired_notation += 1;
        }
    }
    assert_eq!(
        categories,
        BTreeMap::from([
            ("variables", 2),
            ("premise order", 1),
            ("== binding", 7),
            ("command metadata and a concept named as a symbol", 1),
            ("meaning", 1),
        ])
    );
    // The YAML's session rule, and its two-field retract, which the
    // notation writes as two rules of other heads.
    assert_eq!((unpaired_yaml, unpaired_notation), (2, 2));
}

const CORE_CLJ: &str = include_str!("../../tonk-core/assets/library/core.clj");
const CORE_YAML: &str = include_str!("../../tonk-core/assets/library/core.yaml");

const NOTEBOOK: &str =
    "notebook!:\n  this: id:nb\n  title: \"Plan\"\n  block:\n    N1: id:nb/1\n    N5: id:nb/2\n";

/// Evaluate tonk's YAML `text` on `branch` and commit it.
async fn commit_yaml<E: Env>(branch: &Branch, operator: &E, text: &str) {
    let parsed = parse(text);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut syntax = parsed.syntax.expect("syntax");
    let unexpanded = tonk_notation::expand(&mut syntax, &tonk_library::Bundled).await;
    assert!(unexpanded.is_empty(), "{unexpanded:?}");
    syntax
        .evaluate(branch.transaction())
        .perform(operator)
        .await
        .expect("evaluates")
        .commit()
        .publish()
        .perform(operator)
        .await
        .expect("commits");
}

/// Run a notation cell on `branch`, after `core.clj` and `notebook.clj`.
async fn cell<E: Env>(branch: &Branch, operator: &E, source: &str) -> Answer {
    let answer = evaluate_on_branch(
        branch,
        &Cell {
            source: source.to_owned(),
            context: vec![CORE_CLJ.to_owned(), NOTEBOOK_CLJ.to_owned()],
        },
        operator,
    )
    .await;
    assert!(answer.error.is_none(), "{:?}", answer.error);
    for form in &answer.forms {
        assert!(form.refused.is_none(), "{:?}", form.refused);
    }
    answer
}

/// A notebook written under the YAML, its title and its keyed blocks,
/// answers through the notation's `notebook`, and one the notation writes
/// reads back through the concept the YAML stored.
#[dialog_common::test]
async fn notebooks_read_back_across_the_two_notations() {
    let peer = open_peer(test_storage().await, Location::profile(unique_name("nb")))
        .await
        .expect("a peer");
    let operator = peer
        .session(b"nb")
        .space(peer.state())
        .allow(Subject::any())
        .await
        .expect("a session");
    let branch: Branch = peer
        .space(unique_name("repo"))
        .open()
        .perform(&operator)
        .await
        .expect("a repository")
        .branch("main")
        .open()
        .perform(&operator)
        .await
        .expect("a branch");
    // A space holds core before any component, as the worker seeds it.
    commit_yaml(&branch, &operator, CORE_YAML).await;
    commit_yaml(&branch, &operator, NOTEBOOK_YAML).await;
    commit_yaml(&branch, &operator, NOTEBOOK).await;

    let answer = cell(
        &branch,
        &operator,
        "(select (notebook id:nb title: ?title block: {?k ?b}))",
    )
    .await;
    let printed = answer.forms[0].printed.join("\n");
    println!("{printed}");
    assert_eq!(answer.forms[0].printed.len(), 2, "{printed}");
    for value in ["\"Plan\"", "id:nb/1", "id:nb/2"] {
        assert!(printed.contains(value), "{value} in {printed}");
    }

    cell(
        &branch,
        &operator,
        "(assert (notebook id:nb2 title: \"Notes\" block: {\"N3\" id:nb2/1}))",
    )
    .await;
    let concept = names(&yaml_claims("notebook.yaml", NOTEBOOK_YAML).await)["notebook"].clone();
    let stored = Concept::by_entity(concept.parse().expect("an entity"))
        .resolve(&Source::from(&branch), &operator)
        .await
        .expect("the concept resolves")
        .expect("the concept is stored");
    let mut terms = Parameters::new();
    terms.insert(
        "this".to_owned(),
        Term::Constant(Value::Entity("id:nb2".parse().expect("an entity"))),
    );
    terms.insert("title".to_owned(), Term::var("title"));
    let rows = branch
        .select(QueryPlan::from(ConceptQuery {
            predicate: stored.descriptor,
            terms,
        }))
        .perform(&operator)
        .try_vec()
        .await
        .expect("the concept reads");
    assert!(
        rows.iter()
            .any(|row| row.source().value_of("title") == Some(&Value::String("Notes".into()))),
        "{} rows",
        rows.len()
    );
}
