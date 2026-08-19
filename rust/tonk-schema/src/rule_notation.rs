//! Render a stored dialog rule back into `rule!:` notation.
//!
//! The inverse of the analyzer's rule lifting: given an
//! [`InductiveRule`] or [`DeductiveRule`] hydrated from its
//! `dialog.rule/source` body, emit the notation block that
//! re-compiles to the *same content-addressed rule*. The invariant
//! callers should hold the output to is `encode()` byte equality
//! (and therefore `this()` equality) between the stored rule and
//! the re-analyzed render.
//!
//! What makes the inverse exact:
//!
//! - Variables are stored *by name* in the canonical body, so the
//!   content address is name-sensitive. Every named variable —
//!   including analyzer-minted `__N` names from omitted `this:`
//!   slots — is emitted verbatim; `?__7` re-lifts to the identical
//!   representation.
//! - A blank (nameless) concept-field term re-lifts from an
//!   *omitted* field, so blanks are rendered by omission. Blanks
//!   in `this` or formula/constraint operands have no notation
//!   spelling (`_` there mints a *named* unique variable) and fail
//!   the render instead.
//! - Concept references have no inline-descriptor notation form;
//!   the head and every concept premise must resolve to a
//!   published name whose descriptor matches the embedded one in
//!   *serde form* (not content address — the address ignores
//!   descriptions and field names, but the stored body embeds
//!   them, so a drifted description would silently change the
//!   re-compiled bytes).
//!
//! Rules that cannot be expressed in notation — deductive `reduce`
//! folds, typed variables, byte/record/symbol constants, sparse
//! term maps from foreign authoring — return a
//! [`RenderRuleError`] so the caller can skip them loudly rather
//! than emit a block that re-compiles to a different rule.

use std::fmt::Write as _;

use dialog_query::{ConceptDescriptor, ConceptQuery, Proposition};
use serde_json::Value as Json;
use thiserror::Error;

use crate::rule::{DeductiveRule, InductiveRule, Rule};

/// Why a stored rule could not be rendered as notation.
#[derive(Debug, Error)]
pub enum RenderRuleError {
    /// Deductive `reduce` folds have no notation encoding yet.
    #[error("deductive `reduce` folds have no notation form")]
    ReduceNotSupported,
    /// A variable carries a type stamp, which notation cannot spell.
    #[error("variable {name:?} carries a type stamp notation cannot express")]
    TypedVariable {
        /// The stamped variable's name, when it has one.
        name: String,
    },
    /// A constant value kind notation cannot spell.
    #[error("{kind} constants have no notation form")]
    InexpressibleConstant {
        /// Human tag of the offending value kind.
        kind: &'static str,
    },
    /// A concept premise's term map does not cover exactly
    /// `this` + the predicate's fields — foreign authoring.
    #[error("concept premise terms do not match the predicate's fields")]
    SparseConceptTerms,
    /// A blank term sits in a slot whose omission would re-mint a
    /// *named* unique variable instead of a blank.
    #[error("blank {slot} operand has no notation form")]
    BlankOperand {
        /// The slot the blank sits in (`this`, or an operand name).
        slot: String,
    },
    /// No published name carries a descriptor equal (in serde
    /// form) to the one embedded in the rule.
    #[error("no published concept name matches the embedded descriptor ({concept})")]
    UnresolvedConcept {
        /// Compact summary of the unmatched descriptor —
        /// description plus field names — for the skip comment.
        concept: String,
    },
    /// The rule body failed to serialize — should not happen for a
    /// rule that was hydrated from storage.
    #[error("rule body failed to serialize: {0}")]
    Serialize(String),
}

/// Resolve an embedded concept descriptor to the published name the
/// notation should reference. Implementations must match by *serde
/// form* equality (`serde_json::to_value` of both sides), not by
/// content address — see the module docs for why.
pub trait ConceptNames {
    /// The name to write for `descriptor`, or
    /// [`RenderRuleError::UnresolvedConcept`].
    fn reference(&mut self, descriptor: &ConceptDescriptor) -> Result<String, RenderRuleError>;
}

/// Render either rule kind.
pub fn render_rule(rule: &Rule, names: &mut dyn ConceptNames) -> Result<String, RenderRuleError> {
    match rule {
        Rule::Inductive(rule) => render_inductive(rule, names),
        Rule::Deductive(rule) => render_deductive(rule, names),
    }
}

/// Render an inductive rule as a `rule!:` block with an `assert!:`
/// or `retract!:` head.
pub fn render_inductive(
    rule: &InductiveRule,
    names: &mut dyn ConceptNames,
) -> Result<String, RenderRuleError> {
    let descriptor = rule.descriptor();
    let (head_key, head) = match (&descriptor.assert, &descriptor.retract) {
        (Some(head), None) => ("assert!", head),
        (None, Some(head)) => ("retract!", head),
        // `descriptor()` on a compiled rule always sets exactly one.
        _ => {
            return Err(RenderRuleError::Serialize(
                "inductive descriptor without exactly one head".to_owned(),
            ));
        }
    };
    render_block(
        descriptor.description.as_deref(),
        head_key,
        head,
        &descriptor.when,
        &descriptor.unless,
        names,
    )
}

/// Render a deductive rule as a `rule!:` block with the no-bang
/// `assert:` head the analyzer compiles to a deduction.
pub fn render_deductive(
    rule: &DeductiveRule,
    names: &mut dyn ConceptNames,
) -> Result<String, RenderRuleError> {
    let descriptor = rule.descriptor();
    if !descriptor.reduce.is_empty() {
        return Err(RenderRuleError::ReduceNotSupported);
    }
    render_block(
        descriptor.description.as_deref(),
        "assert",
        &descriptor.deduce,
        &descriptor.when,
        &descriptor.unless,
        names,
    )
}

fn render_block(
    description: Option<&str>,
    head_key: &str,
    head: &ConceptDescriptor,
    when: &[Proposition],
    unless: &[Proposition],
    names: &mut dyn ConceptNames,
) -> Result<String, RenderRuleError> {
    let mut out = String::from("rule!:\n");
    if let Some(description) = description {
        let _ = writeln!(out, "  description: {}", quote(description));
    }
    let _ = writeln!(
        out,
        "  {head_key}: {}",
        concept_reference(&names.reference(head)?)
    );
    for (key, premises) in [("when", when), ("unless", unless)] {
        if premises.is_empty() {
            continue;
        }
        let _ = writeln!(out, "  {key}:");
        for premise in premises {
            render_premise(&mut out, premise, names)?;
        }
    }
    Ok(out)
}

fn render_premise(
    out: &mut String,
    premise: &Proposition,
    names: &mut dyn ConceptNames,
) -> Result<(), RenderRuleError> {
    match premise {
        Proposition::Concept(query) => render_concept_premise(out, query, names),
        // Formula, constraint, and resolver premises share the
        // `{assert: <name>, where: {operands}}` serde shape; the
        // variant only decides how blanks are treated.
        Proposition::Formula(_) | Proposition::Constraint(_) => {
            render_operand_premise(out, premise, SlotPolicy::Operand)
        }
        // A resolver's unwritten operands default to blank on the
        // deserialize path, so omission is their spelling.
        Proposition::Resolver(_) => {
            render_operand_premise(out, premise, SlotPolicy::ResolverOperand)
        }
        // Attribute-query premises have no formal-notation
        // encoding, and dialog refuses to store rules containing
        // them — reachable only through foreign bodies.
        Proposition::Attribute(_) | Proposition::OptionalAttribute(_) => Err(
            RenderRuleError::Serialize("attribute-query premises have no notation form".to_owned()),
        ),
    }
}

fn render_concept_premise(
    out: &mut String,
    query: &ConceptQuery,
    names: &mut dyn ConceptNames,
) -> Result<(), RenderRuleError> {
    let name = names.reference(&query.predicate)?;
    let terms = serde_json::to_value(&query.terms)
        .map_err(|error| RenderRuleError::Serialize(error.to_string()))?;
    let Json::Object(terms) = terms else {
        return Err(RenderRuleError::Serialize(
            "concept premise terms did not serialize as a map".to_owned(),
        ));
    };
    // The analyzer's lifting fills a term for `this` plus *every*
    // predicate field, so a stored map with any other key set is
    // foreign authoring the re-lift would silently reshape.
    let fields: Vec<&str> = query.predicate.with().keys().collect();
    if terms.len() != fields.len() + 1 || !terms.contains_key("this") {
        return Err(RenderRuleError::SparseConceptTerms);
    }
    for field in &fields {
        if !terms.contains_key(*field) {
            return Err(RenderRuleError::SparseConceptTerms);
        }
    }

    let _ = writeln!(out, "    - assert: {}", concept_reference(&name));
    let _ = writeln!(out, "      where:");
    // `this` first, then the predicate's field order. Order is
    // free — the analyzer rebuilds a map — but determinism keeps
    // the output diffable.
    render_operand(out, "this", &terms["this"], SlotPolicy::ConceptThis)?;
    for field in fields {
        render_operand(out, field, &terms[field], SlotPolicy::ConceptField)?;
    }
    Ok(())
}

fn render_operand_premise(
    out: &mut String,
    premise: &Proposition,
    blanks: SlotPolicy,
) -> Result<(), RenderRuleError> {
    let json = serde_json::to_value(premise)
        .map_err(|error| RenderRuleError::Serialize(error.to_string()))?;
    let (Some(Json::String(name)), Some(Json::Object(operands))) =
        (json.get("assert"), json.get("where"))
    else {
        return Err(RenderRuleError::Serialize(
            "operand premise did not serialize as {assert, where}".to_owned(),
        ));
    };
    let _ = writeln!(out, "    - assert: {}", concept_reference(name));
    let _ = writeln!(out, "      where:");
    // Operand maps serialize from a HashMap; sort for determinism.
    let mut sorted: Vec<(&String, &Json)> = operands.iter().collect();
    sorted.sort_by_key(|(key, _)| key.as_str());
    for (operand, term) in sorted {
        render_operand(out, operand, term, blanks)?;
    }
    Ok(())
}

/// How the slot at hand treats blanks and type stamps.
#[derive(Clone, Copy, PartialEq)]
enum SlotPolicy {
    /// Concept field: blanks spell as omission (re-lifting an
    /// omitted field mints a blank again); a type stamp cannot be
    /// re-derived from notation and fails the render.
    ConceptField,
    /// Concept `this`: omission mints a *named* unique variable,
    /// so a blank has no spelling; type stamps fail as above.
    ConceptThis,
    /// Formula / constraint operand: blanks have no spelling
    /// (omission mints a named variable, `_` likewise), but type
    /// stamps are structural — the operand cell re-derives the
    /// same type when the notation is lifted — so `?name` alone
    /// is the faithful spelling.
    Operand,
    /// Resolver operand: unwritten operands default to blank on
    /// the deserialize path, so omission is their spelling; type
    /// stamps are structural as above.
    ResolverOperand,
}

impl SlotPolicy {
    fn omits_blanks(self) -> bool {
        matches!(self, Self::ConceptField | Self::ResolverOperand)
    }

    fn derives_types(self) -> bool {
        matches!(self, Self::Operand | Self::ResolverOperand)
    }
}

fn render_operand(
    out: &mut String,
    slot: &str,
    term: &Json,
    policy: SlotPolicy,
) -> Result<(), RenderRuleError> {
    // Serialized `Term`: `{"?": {"name"?, "type"?}}` for a
    // variable, any other JSON value for a constant.
    if let Some(var) = term.get("?") {
        if var.get("type").is_some() && !policy.derives_types() {
            return Err(RenderRuleError::TypedVariable {
                name: var
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        return match var.get("name").and_then(Json::as_str) {
            Some(name) => {
                let _ = writeln!(out, "        {slot}: ?{name}");
                Ok(())
            }
            None if policy.omits_blanks() => Ok(()),
            None => Err(RenderRuleError::BlankOperand {
                slot: slot.to_owned(),
            }),
        };
    }
    let rendered = render_constant(term)?;
    let _ = writeln!(out, "        {slot}: {rendered}");
    Ok(())
}

/// Spell a constant term so the analyzer lifts it back to the same
/// [`dialog_artifacts::Value`]:
///
/// - strings always quoted (a bare `42`, `true`, or `id:x` would
///   misclassify), except entity URIs which must stay *unquoted*
///   (quoting one would demote it to a string and trip the
///   `as: entity` type check);
/// - floats through `{:?}` so `2.0` keeps its dot and re-parses as
///   a float rather than an integer.
///
/// Bytes, records, and symbols have no notation spelling. Entity
/// vs string is decided the way the term serializer decided it:
/// dialog serializes `Value::Entity` as its URI string, and the
/// deserialize path (like the analyzer's lifting) reclassifies by
/// URI shape — so a `Value::String` that *looks* like an entity URI
/// already round-trips as an entity in the canonical body, and
/// spelling it unquoted is byte-faithful.
fn render_constant(term: &Json) -> Result<String, RenderRuleError> {
    match term {
        Json::String(text) => {
            if is_entity_uri(text) {
                Ok(text.clone())
            } else {
                Ok(quote(text))
            }
        }
        Json::Bool(flag) => Ok(flag.to_string()),
        Json::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                Ok(unsigned.to_string())
            } else if let Some(signed) = number.as_i64() {
                Ok(signed.to_string())
            } else if let Some(float) = number.as_f64() {
                if !float.is_finite() {
                    return Err(RenderRuleError::InexpressibleConstant {
                        kind: "non-finite float",
                    });
                }
                Ok(format!("{float:?}"))
            } else {
                Err(RenderRuleError::InexpressibleConstant {
                    kind: "out-of-range number",
                })
            }
        }
        // Bytes serialize as arrays, records/symbols as objects —
        // neither has a notation spelling.
        Json::Array(_) => Err(RenderRuleError::InexpressibleConstant { kind: "bytes" }),
        _ => Err(RenderRuleError::InexpressibleConstant {
            kind: "structured value",
        }),
    }
}

/// `true` when `text` parses as an entity URI, which the analyzer
/// (and dialog's own untagged deserialize) classifies ahead of
/// plain strings.
fn is_entity_uri(text: &str) -> bool {
    text.parse::<dialog_artifacts::Entity>().is_ok()
}

/// Spell a concept / formula / constraint name. Bare only when it
/// is shaped like the symbols the notation classifier accepts
/// (lowercase kebab, optionally `/`-qualified); quoted otherwise —
/// the premise `assert:` and rule heads both accept quoted strings.
fn concept_reference(name: &str) -> String {
    let symbol_shaped = !name.is_empty()
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        && name.matches('/').count() <= 1;
    if symbol_shaped {
        name.to_owned()
    } else {
        quote(name)
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use dialog_query::{DeductiveRuleDescriptor, InductiveRuleDescriptor};
    use serde_json::json;

    use super::*;

    /// Name table keyed by descriptor serde form — the same
    /// matching rule real callers must use.
    struct Names(Vec<(Json, String)>);

    impl Names {
        fn new(entries: &[(&Json, &str)]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(descriptor, name)| ((*descriptor).clone(), (*name).to_string()))
                    .collect(),
            )
        }
    }

    impl ConceptNames for Names {
        fn reference(&mut self, descriptor: &ConceptDescriptor) -> Result<String, RenderRuleError> {
            let form = serde_json::to_value(descriptor)
                .map_err(|error| RenderRuleError::Serialize(error.to_string()))?;
            self.0
                .iter()
                .find(|(candidate, _)| *candidate == form)
                .map(|(_, name)| name.clone())
                .ok_or_else(|| RenderRuleError::UnresolvedConcept {
                    concept: form.to_string(),
                })
        }
    }

    fn counter_concept() -> Json {
        json!({
            "with": {
                "count": {
                    "the": "counter/count",
                    "as": "UnsignedInteger",
                    "cardinality": "one",
                    "description": "the count"
                }
            }
        })
    }

    /// Normalize a raw descriptor JSON through the typed
    /// `ConceptDescriptor` so name-table entries compare in the
    /// same serde form the renderer produces.
    fn normalized(descriptor: &Json) -> Json {
        let typed: ConceptDescriptor = serde_json::from_value(descriptor.clone()).unwrap();
        serde_json::to_value(&typed).unwrap()
    }

    #[dialog_common::test]
    fn it_renders_an_asserting_rule_with_concept_and_formula_premises() {
        let counter = counter_concept();
        let descriptor: InductiveRuleDescriptor = serde_json::from_value(json!({
            "assert!": counter,
            "when": [
                {
                    "assert": counter,
                    "where": {
                        "this": { "?": { "name": "this" } },
                        "count": { "?": { "name": "prev" } }
                    }
                },
                {
                    "assert": "math/sum",
                    "where": {
                        "of": { "?": { "name": "prev" } },
                        "with": 1,
                        "is": { "?": { "name": "count" } }
                    }
                }
            ]
        }))
        .unwrap();
        let rule = descriptor.compile().unwrap();
        let counter_form = normalized(&counter);
        let mut names = Names::new(&[(&counter_form, "counter")]);

        let rendered = render_inductive(&rule, &mut names).unwrap();

        assert_eq!(
            rendered,
            r#"rule!:
  assert!: counter
  when:
    - assert: counter
      where:
        this: ?this
        count: ?prev
    - assert: math/sum
      where:
        is: ?count
        of: ?prev
        with: 1
"#
        );
    }

    #[dialog_common::test]
    fn it_renders_retract_heads_unless_blocks_and_constants() {
        let counter = counter_concept();
        let descriptor: InductiveRuleDescriptor = serde_json::from_value(json!({
            "retract!": counter,
            "when": [{
                "assert": counter,
                "where": {
                    "this": { "?": { "name": "this" } },
                    "count": { "?": { "name": "count" } }
                }
            }],
            "unless": [{
                "assert": counter,
                "where": {
                    "this": { "?": { "name": "this" } },
                    "count": 2.0
                }
            }]
        }))
        .unwrap();
        let rule = descriptor.compile().unwrap();
        let counter_form = normalized(&counter);
        let mut names = Names::new(&[(&counter_form, "counter")]);

        let rendered = render_inductive(&rule, &mut names).unwrap();

        assert!(rendered.contains("  retract!: counter\n"));
        assert!(rendered.contains("  unless:\n"), "{rendered}");
        // `{:?}` keeps the dot so the value re-lifts as a float.
        assert!(rendered.contains("count: 2.0\n"), "{rendered}");
    }

    #[dialog_common::test]
    fn it_renders_a_deductive_rule_with_the_no_bang_head() {
        let counter = counter_concept();
        let descriptor: DeductiveRuleDescriptor = serde_json::from_value(json!({
            "deduce": counter,
            "when": [{
                "assert": counter,
                "where": {
                    "this": { "?": { "name": "this" } },
                    "count": { "?": { "name": "count" } }
                }
            }]
        }))
        .unwrap();
        let rule = descriptor.compile().unwrap();
        let counter_form = normalized(&counter);
        let mut names = Names::new(&[(&counter_form, "counter")]);

        let rendered = render_deductive(&rule, &mut names).unwrap();

        assert!(
            rendered.starts_with("rule!:\n  assert: counter\n"),
            "{rendered}"
        );
    }

    #[dialog_common::test]
    fn it_keeps_minted_variable_names_and_omits_blank_fields() {
        let counter = counter_concept();
        let descriptor: InductiveRuleDescriptor = serde_json::from_value(json!({
            "assert!": counter,
            "when": [
                {
                    "assert": counter,
                    "where": {
                        "this": { "?": { "name": "this" } },
                        "count": { "?": { "name": "count" } }
                    }
                },
                {
                    "assert": counter,
                    "where": {
                        // An omitted `this:` at authoring time
                        // minted this name; it must survive
                        // verbatim.
                        "this": { "?": { "name": "__7" } },
                        // A blank field spells as omission.
                        "count": { "?": {} }
                    }
                }
            ]
        }))
        .unwrap();
        let rule = descriptor.compile().unwrap();
        let counter_form = normalized(&counter);
        let mut names = Names::new(&[(&counter_form, "counter")]);

        let rendered = render_inductive(&rule, &mut names).unwrap();

        assert!(rendered.contains("this: ?__7\n"), "{rendered}");
        // The blank-count premise renders its `where:` with only
        // the `this` line; the named-count premise keeps its line.
        assert!(
            rendered.contains("      where:\n        this: ?__7\n"),
            "{rendered}"
        );
    }

    #[dialog_common::test]
    fn it_rejects_reduce_folds_and_unresolved_concepts() {
        let counter = counter_concept();
        let reduced: DeductiveRuleDescriptor = serde_json::from_value(json!({
            "deduce": counter,
            "when": [{
                "assert": counter,
                "where": {
                    "this": { "?": { "name": "this" } },
                    "count": { "?": { "name": "n" } }
                }
            }],
            "reduce": { "count": { "apply": "sum", "of": { "?": { "name": "n" } } } }
        }))
        .unwrap();
        let rule = reduced.compile().unwrap();
        let counter_form = normalized(&counter);
        let mut names = Names::new(&[(&counter_form, "counter")]);
        assert!(matches!(
            render_deductive(&rule, &mut names),
            Err(RenderRuleError::ReduceNotSupported)
        ));

        let plain: InductiveRuleDescriptor = serde_json::from_value(json!({
            "assert!": counter,
            "when": [{
                "assert": counter,
                "where": {
                    "this": { "?": { "name": "this" } },
                    "count": { "?": { "name": "count" } }
                }
            }]
        }))
        .unwrap();
        let rule = plain.compile().unwrap();
        let mut empty = Names::new(&[]);
        assert!(matches!(
            render_inductive(&rule, &mut empty),
            Err(RenderRuleError::UnresolvedConcept { .. })
        ));
    }

    #[dialog_common::test]
    fn it_spells_strings_quoted_and_entities_bare() {
        assert_eq!(
            render_constant(&json!("plain text")).unwrap(),
            "\"plain text\""
        );
        assert_eq!(render_constant(&json!("id:vault")).unwrap(), "id:vault");
        assert_eq!(render_constant(&json!(true)).unwrap(), "true");
        assert_eq!(render_constant(&json!(3)).unwrap(), "3");
    }
}
