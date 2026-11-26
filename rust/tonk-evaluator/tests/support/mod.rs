//! What the identity tests share: each notation of a library lowered
//! as the worker installs it, and the stored claims grouped by entity.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use dialog_corpus::{Changes, Instruction, Statement as _, Value};
use dialog_query::{DeductiveRule, InductiveRule, Parameters, Term};
use tonk_analyzer::analysis::ExpressionAnalysis;
use tonk_schema::transact::{Planner as _, Statement};

const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");
const CORE_CLJ: &str = include_str!("../../../tonk-core/assets/library/core.clj");

/// What the test asks of an environment: everything the evaluator, a
/// commit and the notation's driver do.
pub trait Env:
    tonk_evaluator::evaluate::EvaluateEnv
    + dialog_capability::Provider<dialog_effects::blob::Size>
    + dialog_capability::Provider<dialog_effects::blob::Import>
    + dialog_capability::Provider<dialog_effects::archive::Import>
    + dialog_capability::Provider<dialog_effects::authority::Attest>
    + dialog_capability::Provider<dialog_effects::memory::Publish>
{
}
impl<T> Env for T where
    T: tonk_evaluator::evaluate::EvaluateEnv
        + dialog_capability::Provider<dialog_effects::blob::Size>
        + dialog_capability::Provider<dialog_effects::blob::Import>
        + dialog_capability::Provider<dialog_effects::archive::Import>
        + dialog_capability::Provider<dialog_effects::authority::Attest>
        + dialog_capability::Provider<dialog_effects::memory::Publish>
{
}

/// A claim's relation and value, as text, under the entity it is of.
pub type Claims = BTreeMap<String, BTreeSet<(String, String)>>;

/// A value as text. Bytes that decode as a rule show as the rule's JSON,
/// `rule {…}`, so two stored rules compare by what they hold.
pub fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => format!("{text:?}"),
        Value::Bytes(bytes) => {
            let rule = match (InductiveRule::decode(bytes), DeductiveRule::decode(bytes)) {
                (Ok(rule), _) => serde_json::to_string(&rule).ok(),
                (_, Ok(rule)) => serde_json::to_string(&rule).ok(),
                _ => None,
            };
            match rule {
                Some(rule) => format!("rule {rule}"),
                None => format!("bytes[{}]", bytes.len()),
            }
        }
        other => format!("{other:?}"),
    }
}

pub fn by_entity(instructions: Vec<Instruction>) -> Claims {
    let mut claims = Claims::new();
    for instruction in instructions {
        if let Instruction::Assert(artifact, _) = instruction {
            claims
                .entry(artifact.of.to_string())
                .or_default()
                .insert((artifact.the.to_string(), shown(&artifact.is)));
        }
    }
    claims
}

/// What the worker stores for the component `text` (as `file`), after
/// `core.yaml`: the two analyzed as one document, the component's
/// expressions planned on their own, as `library_claims_after` does.
pub async fn yaml_claims(file: &str, text: &str) -> Claims {
    let parse_at = |file: &str, text: &str| {
        let parsed = tonk_notation::parse_at(tonk_library::location(file), text);
        assert!(
            parsed.diagnostics.is_empty(),
            "{file}: {:?}",
            parsed.diagnostics
        );
        parsed.syntax.expect("syntax")
    };
    let mut joined = parse_at("core.yaml", CORE);
    let first = joined.expressions.len();
    let mut component = parse_at(file, text);
    let unexpanded = tonk_notation::expand(&mut component, &tonk_library::Bundled).await;
    assert!(unexpanded.is_empty(), "{file} includes: {unexpanded:?}");
    joined.expressions.extend(component.expressions);
    let analyzed = tonk_analyzer::analyzer::analyze_local(&joined).expect("analyzes");
    let analysis = &analyzed.analysis;
    let mut bindings = Parameters::new();
    for (name, entity) in &analysis.variables {
        bindings.insert(name.clone(), Term::Constant(Value::Entity(entity.clone())));
    }
    let mut durable = Changes::new();
    for analyzed in analysis.expressions.iter().skip(first) {
        if let ExpressionAnalysis::Assertion(node) = &analyzed.analysis {
            for statement in &node.analysis.claims {
                let Statement::Assert(application) = statement else {
                    panic!("{file} retracts");
                };
                application
                    .clone()
                    .plan(&bindings)
                    .expect("plans")
                    .assert(&mut durable);
            }
        }
    }
    by_entity(durable.into_instructions())
}

/// What the worker stores for the notation component `text`, after
/// `core.clj`, as `notation_claims` installs a `.clj` component. A view
/// is refused by the Dialog-DB compiler and installed as nothing, as the
/// worker installs it; any other refusal fails the test.
pub async fn notation_claims(text: &str) -> Claims {
    let (operator, _) = dialog_peer::helpers::test_session_with_peer().await;
    let library = tonk_analyzer::notation::notation_library_claims(CORE_CLJ, text, &operator)
        .await
        .expect("the component lowers");
    for form in &library.forms {
        assert!(
            form.refused.is_none() || form.kind == "view",
            "{} is refused: {:?}",
            form.key,
            form.refused
        );
    }
    by_entity(library.durable)
}

/// The entity `name` is bound to (`db.name/referent` of `id:<name>`).
pub fn referent(claims: &Claims, name: &str) -> Option<String> {
    claims
        .get(&format!("id:{name}"))?
        .iter()
        .find_map(|(the, is)| {
            (the == "db.name/referent").then(|| {
                is.trim_start_matches("Entity(")
                    .trim_end_matches(')')
                    .to_owned()
            })
        })
}

/// The entity of the attribute stored as `relation` (`db.attribute/id`).
pub fn attribute(claims: &Claims, relation: &str) -> Option<String> {
    let id = ("db.attribute/id".to_owned(), format!("{relation:?}"));
    claims
        .iter()
        .find_map(|(entity, claims)| claims.contains(&id).then(|| entity.clone()))
}

/// A description as the two notations both keep it: each line trimmed,
/// blank lines and the closing newline left out.
pub fn plain(claims: &BTreeSet<(String, String)>) -> BTreeSet<(String, String)> {
    claims
        .iter()
        .map(|(the, is)| {
            if the == "db.meta/description" {
                let text: String = serde_json::from_str(is).expect("a quoted text");
                let lines: Vec<&str> = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect();
                (the.clone(), lines.join("\n"))
            } else {
                (the.clone(), is.clone())
            }
        })
        .collect()
}
