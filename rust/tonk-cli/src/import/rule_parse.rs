//! YAML parsing and lowering for rule definitions.
//!
//! Parses rule definitions using the canonical abbreviated notation:
//!
//! ```yaml
//! diy.planner:
//!   safe-meal:
//!     description: A meal that respects dietary restrictions
//!     deduce:
//!       Meal:
//!         attendee: ?person
//!         recipe: ?recipe
//!     when:
//!       - diy.planner/Meal:
//!           this: ?meal
//!           attendee: ?person
//!           recipe: ?recipe
//!       - diy.cook/ingredient-name:
//!           this: ?entity
//!           is: ?name
//!       - ==:
//!           this: ?name
//!           is: Alice
//!       - math/sum:
//!           of: ?a
//!           with: ?b
//!           is: ?result
//!     unless:
//!       - diy.planner/AllergyConflict:
//!           person: ?person
//!           recipe: ?recipe
//! ```
//!
//! After parsing, the concept-oriented representation is *lowered* into
//! flat EAV-based `RuleDefinition`s for the engine.

use crate::rule::{RuleConclusion, RuleDefinition, RulePremise};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Known formula and constraint references
// ---------------------------------------------------------------------------

/// Known formula references that are treated specially in premises.
const FORMULA_REFS: &[&str] = &[
    "math/sum",
    "math/difference",
    "math/product",
    "math/quotient",
    "math/modulo",
    "text/concatenate",
    "text/length",
    "text/upper-case",
    "text/lower-case",
    "text/like",
    "boolean/and",
    "boolean/or",
    "boolean/not",
];

/// Known constraint references.
const CONSTRAINT_REFS: &[&str] = &["=="];

// ---------------------------------------------------------------------------
// Parsed intermediate representation
// ---------------------------------------------------------------------------

/// A rule parsed from YAML, ready to be lowered into a `RuleDefinition`.
#[derive(Debug)]
pub(super) struct ParsedRule {
    /// Rule name from the YAML key (e.g. "respect-dietary-restrictions").
    pub name: String,
    /// Namespace from the top-level key (e.g. "diy.planner").
    pub namespace: String,
    /// Optional human-readable description from the `description` field.
    pub description: Option<String>,
    /// The conclusion: which concept is deduced and its bindings.
    pub conclusion: ParsedRuleConclusion,
    /// Positive premises (must hold).
    pub when: Vec<ParsedRulePremise>,
    /// Negative premises (must not hold).
    pub unless: Vec<ParsedRulePremise>,
}

/// The conclusion of a parsed rule.
#[derive(Debug)]
pub(super) struct ParsedRuleConclusion {
    /// Concept name extracted from the reference (e.g. "Meal" from "diy.planner/Meal").
    pub concept_name: String,
    /// Bindings: attribute short names to variables/wildcards/constants.
    /// The `this` key, if present, is stored separately.
    pub bindings: HashMap<String, String>,
    /// Optional explicit `this` binding from the conclusion.
    pub this: Option<String>,
}

/// The kind of premise in a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PremiseKind {
    /// Concept premise: key is `namespace/ConceptName` (CamelCase after `/`).
    Concept,
    /// Raw attribute premise: key is `domain/attribute-name` (kebab-case after `/`).
    RawAttribute,
    /// Equality constraint: key is `==`.
    Equality,
    /// Formula: key is `math/sum`, `text/concatenate`, etc.
    Formula,
}

/// A single premise from the YAML, before lowering to EAV.
#[derive(Debug)]
pub(super) struct ParsedRulePremise {
    /// What kind of premise this is.
    pub kind: PremiseKind,
    /// The full reference key (e.g. "diy.cook/Recipe", "==", "math/sum").
    pub reference: String,
    /// For concept premises: the concept name extracted from the reference.
    /// For others: the full reference.
    pub concept_name: String,
    /// The `this` value (entity binding). Defaults to `"_"` if absent.
    pub this_value: String,
    /// Attribute bindings: short name -> variable/wildcard/constant.
    /// Does not include `this`.
    pub attributes: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Premise classification
// ---------------------------------------------------------------------------

/// Classify a premise reference key into its kind.
fn classify_premise_ref(reference: &str) -> PremiseKind {
    // Check for known constraints
    if CONSTRAINT_REFS.contains(&reference) {
        return PremiseKind::Equality;
    }

    // Check for known formulas
    if FORMULA_REFS.contains(&reference) {
        return PremiseKind::Formula;
    }

    // If it contains `/`, check whether the part after the last `/` starts
    // with an uppercase letter (concept) or lowercase (raw attribute).
    if let Some((_ns, name)) = reference.rsplit_once('/') {
        if name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return PremiseKind::Concept;
        }
        return PremiseKind::RawAttribute;
    }

    // Bare name starting with uppercase → concept, else raw attribute
    if reference.starts_with(|c: char| c.is_ascii_uppercase()) {
        PremiseKind::Concept
    } else {
        PremiseKind::RawAttribute
    }
}

// ---------------------------------------------------------------------------
// Rule YAML parsing
// ---------------------------------------------------------------------------

/// Extract the concept name from a namespaced reference like `"diy.cook/Recipe"`.
///
/// Splits on the last `/` and returns the part after it. If there's no `/`,
/// returns the whole string.
fn extract_concept_name(reference: &str) -> String {
    match reference.rsplit_once('/') {
        Some((_ns, name)) => name.to_string(),
        None => reference.to_string(),
    }
}

/// Parse a YAML string containing rule definitions.
///
/// Structure: `namespace -> rule_name -> {description, deduce, when, unless}`.
pub(super) fn parse_rules_yaml(yaml_str: &str) -> Result<Vec<ParsedRule>> {
    let root: BTreeMap<String, BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(yaml_str).context("Failed to parse YAML")?;

    let mut rules = Vec::new();

    for (namespace, rule_map) in root {
        for (rule_name, rule_value) in rule_map {
            let parsed = parse_rule(&namespace, &rule_name, &rule_value)
                .with_context(|| format!("In rule '{}/{}'", namespace, rule_name))?;
            rules.push(parsed);
        }
    }

    Ok(rules)
}

/// Parse a single rule from its YAML value.
///
/// Called both from `parse_rules_yaml` (standalone rules file) and from
/// the mixed-file parser in `concept_parse`.
pub(super) fn parse_rule(
    namespace: &str,
    rule_name: &str,
    value: &serde_yaml::Value,
) -> Result<ParsedRule> {
    let map: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_value(value.clone())
        .context("Expected a mapping with deduce/when/unless")?;

    // Parse `description` (optional)
    let description = map
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Parse `deduce` (required)
    let deduce_value = map
        .get("deduce")
        .ok_or_else(|| anyhow::anyhow!("Rule must have a 'deduce' section"))?;

    let conclusion =
        parse_rule_conclusion(deduce_value).context("Failed to parse 'deduce' section")?;

    // Parse `when` (required, list of premises)
    let when_value = map
        .get("when")
        .ok_or_else(|| anyhow::anyhow!("Rule must have a 'when' section"))?;

    let when = parse_rule_premises(when_value).context("Failed to parse 'when' section")?;

    if when.is_empty() {
        anyhow::bail!("Rule must have at least one positive premise in 'when'.");
    }

    // Parse `unless` (optional)
    let unless = if let Some(unless_value) = map.get("unless") {
        parse_rule_premises(unless_value).context("Failed to parse 'unless' section")?
    } else {
        Vec::new()
    };

    Ok(ParsedRule {
        name: rule_name.to_string(),
        namespace: namespace.to_string(),
        description,
        conclusion,
        when,
        unless,
    })
}

/// Parse the `deduce` section of a rule YAML.
///
/// Expected shape: a single-key mapping where the key is a concept reference
/// and the value is a mapping of attribute bindings.
///
/// ```yaml
/// deduce:
///   Meal:
///     attendee: ?person
///     recipe: ?recipe
/// ```
fn parse_rule_conclusion(value: &serde_yaml::Value) -> Result<ParsedRuleConclusion> {
    let map: BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_value(value.clone()).context("Expected a concept mapping")?;

    if map.len() != 1 {
        anyhow::bail!(
            "The 'deduce' section must contain exactly one concept, found {}",
            map.len()
        );
    }

    let (concept_ref, bindings_value) = map.into_iter().next().unwrap();
    let concept_name = extract_concept_name(&concept_ref);

    let bindings_map: BTreeMap<String, String> = serde_yaml::from_value(bindings_value)
        .context("Expected a mapping of attribute bindings (e.g. attendee: ?person)")?;

    let mut bindings = HashMap::new();
    let mut this = None;

    for (key, val) in bindings_map {
        if key == "this" {
            this = Some(val);
        } else {
            bindings.insert(key, val);
        }
    }

    Ok(ParsedRuleConclusion {
        concept_name,
        bindings,
        this,
    })
}

/// Parse the `when` or `unless` section (a list of premises).
///
/// Each entry is a single-key mapping. The key determines the premise type:
/// - `namespace/ConceptName` (CamelCase) → concept premise
/// - `domain/attribute-name` (kebab-case) → raw attribute premise
/// - `==` → equality constraint
/// - `math/sum`, etc. → formula premise
fn parse_rule_premises(value: &serde_yaml::Value) -> Result<Vec<ParsedRulePremise>> {
    let list: Vec<serde_yaml::Value> =
        serde_yaml::from_value(value.clone()).context("Expected a list of premises")?;

    let mut premises = Vec::new();

    for (i, entry) in list.iter().enumerate() {
        let map: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_value(entry.clone())
            .with_context(|| format!("Premise {} must be a mapping", i + 1))?;

        if map.len() != 1 {
            anyhow::bail!(
                "Premise {} must reference exactly one concept/attribute/constraint/formula, found {}",
                i + 1,
                map.len()
            );
        }

        let (reference, attrs_value) = map.into_iter().next().unwrap();
        let kind = classify_premise_ref(&reference);
        let concept_name = extract_concept_name(&reference);

        let attrs_map: BTreeMap<String, String> = serde_yaml::from_value(attrs_value)
            .with_context(|| {
                format!(
                    "Premise {} ({}) must have bindings (e.g. this: ?var, name: _)",
                    i + 1,
                    reference
                )
            })?;

        let mut this_value = "_".to_string();
        let mut attributes = Vec::new();

        for (key, val) in attrs_map {
            if key == "this" {
                this_value = val;
            } else {
                attributes.push((key, val));
            }
        }

        premises.push(ParsedRulePremise {
            kind,
            reference: reference.clone(),
            concept_name,
            this_value,
            attributes,
        });
    }

    Ok(premises)
}

// ---------------------------------------------------------------------------
// Rule lowering: YAML -> RuleDefinition
// ---------------------------------------------------------------------------

/// Lower a `ParsedRule` into the internal `RuleDefinition` (EAV-based).
///
/// Each concept-oriented premise is expanded into one EAV premise per
/// attribute mentioned. Raw attribute premises pass through as single EAV
/// premises. Equality constraints and formulas are lowered to EAV premises
/// using the reference as the `the` field.
pub(super) fn lower_rule(parsed: &ParsedRule) -> Result<RuleDefinition> {
    // Lower the conclusion
    let conclusion = RuleConclusion {
        concept: parsed.conclusion.concept_name.clone(),
        bindings: parsed.conclusion.bindings.clone(),
        this: parsed.conclusion.this.clone(),
    };

    // Lower positive premises
    let when = lower_premises(&parsed.when).context("Failed to lower 'when' premises")?;

    // Lower negative premises
    let unless = lower_premises(&parsed.unless).context("Failed to lower 'unless' premises")?;

    Ok(RuleDefinition {
        conclusion,
        when,
        unless,
    })
}

/// Lower a list of parsed premises into flat EAV triples.
///
/// - **Concept premises**: Each attribute produces one `RulePremise` with
///   `the = concept_name_lowercase/attr_name`, `of = this_value`, `is = value`.
/// - **Raw attribute premises**: A single `RulePremise` with `the = reference`,
///   `of = this_value`, `is = is_value`.
/// - **Equality constraints**: A single `RulePremise` with `the = "=="`,
///   `of = this_value`, `is = is_value`.
/// - **Formulas**: A single `RulePremise` per binding with `the = formula_ref`,
///   `of = param_name`, `is = param_value`. Note: formula lowering is
///   provisional — the rule engine does not yet support formulas.
fn lower_premises(premises: &[ParsedRulePremise]) -> Result<Vec<RulePremise>> {
    let mut eav_premises = Vec::new();

    for premise in premises {
        match premise.kind {
            PremiseKind::Concept => {
                let prefix = premise.concept_name.to_lowercase();
                for (attr_name, value) in &premise.attributes {
                    let qualified = format!("{}/{}", prefix, attr_name);
                    eav_premises.push(RulePremise {
                        the: qualified,
                        of: premise.this_value.clone(),
                        is: value.clone(),
                    });
                }
            }
            PremiseKind::RawAttribute => {
                // Raw attribute premise: pass through the fully qualified reference
                // The `is` binding is stored in attributes under key "is"
                let is_value = premise
                    .attributes
                    .iter()
                    .find(|&(k, _)| k == "is")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "_".to_string());

                eav_premises.push(RulePremise {
                    the: premise.reference.clone(),
                    of: premise.this_value.clone(),
                    is: is_value,
                });
            }
            PremiseKind::Equality => {
                // Equality constraint: `==: { this: ?x, is: ?y }`
                let is_value = premise
                    .attributes
                    .iter()
                    .find(|&(k, _)| k == "is")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "_".to_string());

                eav_premises.push(RulePremise {
                    the: "==".to_string(),
                    of: premise.this_value.clone(),
                    is: is_value,
                });
            }
            PremiseKind::Formula => {
                // Formula premise: lower each parameter binding as a separate
                // EAV premise with the formula reference as `the`.
                // This is provisional — the rule engine does not yet support
                // formulas natively. We store them for forward compatibility.
                for (param_name, param_value) in &premise.attributes {
                    eav_premises.push(RulePremise {
                        the: premise.reference.clone(),
                        of: param_name.clone(),
                        is: param_value.clone(),
                    });
                }
            }
        }
    }

    Ok(eav_premises)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_concept_name_with_namespace() {
        assert_eq!(extract_concept_name("diy.cook/Recipe"), "Recipe");
        assert_eq!(extract_concept_name("diy.meal-planner/Meal"), "Meal");
        assert_eq!(extract_concept_name("Recipe"), "Recipe");
    }

    #[test]
    fn classify_concept_premise() {
        assert_eq!(
            classify_premise_ref("diy.cook/Recipe"),
            PremiseKind::Concept
        );
        assert_eq!(
            classify_premise_ref("diy.planner/Meal"),
            PremiseKind::Concept
        );
    }

    #[test]
    fn classify_raw_attribute_premise() {
        assert_eq!(
            classify_premise_ref("diy.cook/ingredient-name"),
            PremiseKind::RawAttribute
        );
        assert_eq!(
            classify_premise_ref("diy.cook/quantity"),
            PremiseKind::RawAttribute
        );
    }

    #[test]
    fn classify_equality_premise() {
        assert_eq!(classify_premise_ref("=="), PremiseKind::Equality);
    }

    #[test]
    fn classify_formula_premise() {
        assert_eq!(classify_premise_ref("math/sum"), PremiseKind::Formula);
        assert_eq!(
            classify_premise_ref("text/concatenate"),
            PremiseKind::Formula
        );
        assert_eq!(classify_premise_ref("boolean/not"), PremiseKind::Formula);
    }

    #[test]
    fn parse_basic_rule() {
        let yaml = r#"
user.rules:
  my-rule:
    description: Test rule
    deduce:
      diy.planner/Meal:
        attendee: ?person
        recipe: ?recipe
    when:
      - diy.cook/Recipe:
          this: ?recipe
          title: _
          ingredient: ?ingredient
      - diy.cook/Ingredient:
          this: ?ingredient
          name: ?substance
    unless:
      - diy.health/Allergy:
          this: _
          person: ?person
          substance: ?substance
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert_eq!(rule.name, "my-rule");
        assert_eq!(rule.namespace, "user.rules");
        assert_eq!(rule.description.as_deref(), Some("Test rule"));
        assert_eq!(rule.conclusion.concept_name, "Meal");
        assert_eq!(rule.conclusion.bindings.len(), 2);
        assert_eq!(rule.conclusion.bindings["attendee"], "?person");
        assert_eq!(rule.conclusion.bindings["recipe"], "?recipe");
        assert!(rule.conclusion.this.is_none());

        assert_eq!(rule.when.len(), 2);
        assert_eq!(rule.when[0].kind, PremiseKind::Concept);
        assert_eq!(rule.when[0].concept_name, "Recipe");
        assert_eq!(rule.when[0].this_value, "?recipe");
        assert_eq!(rule.when[0].attributes.len(), 2);

        assert_eq!(rule.when[1].concept_name, "Ingredient");
        assert_eq!(rule.when[1].this_value, "?ingredient");
        assert_eq!(rule.when[1].attributes.len(), 1);

        assert_eq!(rule.unless.len(), 1);
        assert_eq!(rule.unless[0].concept_name, "Allergy");
        assert_eq!(rule.unless[0].this_value, "_");
        assert_eq!(rule.unless[0].attributes.len(), 2);
    }

    #[test]
    fn parse_rule_with_this_in_conclusion() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Thing:
        this: ?entity
        name: ?n
    when:
      - ns/Other:
          this: ?entity
          label: ?n
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let rule = &rules[0];
        assert_eq!(rule.conclusion.this.as_deref(), Some("?entity"));
        assert_eq!(rule.conclusion.bindings.len(), 1);
        assert_eq!(rule.conclusion.bindings["name"], "?n");
    }

    #[test]
    fn parse_rule_no_unless() {
        let yaml = r#"
ns:
  simple-rule:
    deduce:
      ns/Output:
        value: ?v
    when:
      - ns/Input:
          this: ?x
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        assert!(rules[0].unless.is_empty());
    }

    #[test]
    fn parse_rule_with_description() {
        let yaml = r#"
ns:
  my-rule:
    description: This is a test rule
    deduce:
      ns/Output:
        value: ?v
    when:
      - ns/Input:
          this: ?x
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        assert_eq!(rules[0].description.as_deref(), Some("This is a test rule"));
    }

    #[test]
    fn parse_raw_attribute_premise() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Output:
        name: ?n
    when:
      - diy.cook/ingredient-name:
          this: ?entity
          is: ?n
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let premise = &rules[0].when[0];
        assert_eq!(premise.kind, PremiseKind::RawAttribute);
        assert_eq!(premise.reference, "diy.cook/ingredient-name");
        assert_eq!(premise.this_value, "?entity");
        assert_eq!(premise.attributes.len(), 1);
        assert_eq!(premise.attributes[0], ("is".to_string(), "?n".to_string()));
    }

    #[test]
    fn parse_equality_premise() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Output:
        name: ?name
    when:
      - ns/Person:
          this: ?p
          name: ?name
      - ==:
          this: ?name
          is: Alice
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        assert_eq!(rules[0].when.len(), 2);

        let eq_premise = &rules[0].when[1];
        assert_eq!(eq_premise.kind, PremiseKind::Equality);
        assert_eq!(eq_premise.reference, "==");
        assert_eq!(eq_premise.this_value, "?name");
        assert_eq!(
            eq_premise.attributes[0],
            ("is".to_string(), "Alice".to_string())
        );
    }

    #[test]
    fn parse_formula_premise() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Output:
        total: ?result
    when:
      - ns/Input:
          this: ?e
          value: ?a
      - math/sum:
          of: ?a
          with: ?a
          is: ?result
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        assert_eq!(rules[0].when.len(), 2);

        let formula = &rules[0].when[1];
        assert_eq!(formula.kind, PremiseKind::Formula);
        assert_eq!(formula.reference, "math/sum");
        // Formula bindings: of, is, with (sorted by BTreeMap)
        assert_eq!(formula.attributes.len(), 3);
    }

    // -----------------------------------------------------------------------
    // Rule lowering tests
    // -----------------------------------------------------------------------

    #[test]
    fn lower_basic_rule() {
        let yaml = r#"
user.rules:
  my-rule:
    deduce:
      diy.planner/Meal:
        attendee: ?person
        recipe: ?recipe
    when:
      - diy.planner/Meal:
          this: ?meal
          attendee: ?person
          recipe: ?recipe
      - diy.cook/Recipe:
          this: ?recipe
          title: _
          ingredient: ?ingredient
      - diy.cook/Ingredient:
          this: ?ingredient
          name: ?substance
    unless:
      - diy.health/Allergy:
          this: _
          person: ?person
          substance: ?substance
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.conclusion.concept, "Meal");
        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 2);
        assert_eq!(def.conclusion.bindings["attendee"], "?person");
        assert_eq!(def.conclusion.bindings["recipe"], "?recipe");

        // When: Meal has 2 attrs, Recipe has 2 attrs, Ingredient has 1
        assert_eq!(def.when.len(), 5);

        // Meal premises keep ?meal as entity
        let meal_premises: Vec<&RulePremise> = def
            .when
            .iter()
            .filter(|p| p.the.starts_with("meal/"))
            .collect();
        assert_eq!(meal_premises.len(), 2);
        for p in &meal_premises {
            assert_eq!(p.of, "?meal");
        }

        // Recipe premises keep ?recipe as entity
        let recipe_premises: Vec<&RulePremise> = def
            .when
            .iter()
            .filter(|p| p.the.starts_with("recipe/"))
            .collect();
        assert_eq!(recipe_premises.len(), 2);
        for p in &recipe_premises {
            assert_eq!(p.of, "?recipe");
        }

        // Check specific premises exist
        assert!(
            def.when
                .iter()
                .any(|p| p.the == "recipe/title" && p.is == "_")
        );
        assert!(
            def.when
                .iter()
                .any(|p| p.the == "recipe/ingredient" && p.is == "?ingredient")
        );
        assert!(
            def.when.iter().any(|p| p.the == "ingredient/name"
                && p.of == "?ingredient"
                && p.is == "?substance")
        );

        // Unless premises
        assert_eq!(def.unless.len(), 2);
        assert!(
            def.unless
                .iter()
                .any(|p| p.the == "allergy/person" && p.of == "_" && p.is == "?person")
        );
        assert!(
            def.unless
                .iter()
                .any(|p| p.the == "allergy/substance" && p.of == "_" && p.is == "?substance")
        );
    }

    #[test]
    fn lower_rule_with_this_in_conclusion_no_conflict() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Thing:
        this: ?entity
        name: ?n
    when:
      - ns/Other:
          this: ?entity
          label: ?n
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.conclusion.this.as_deref(), Some("?entity"));
        assert_eq!(def.conclusion.bindings.len(), 1);
        assert_eq!(def.conclusion.bindings["name"], "?n");
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_passes_through_conflicting_vars() {
        let yaml = r#"
ns:
  rule:
    deduce:
      ns/Output:
        source: ?entity
        value: ?v
    when:
      - ns/Input:
          this: ?entity
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 2);
        assert_eq!(def.conclusion.bindings["source"], "?entity");
        assert_eq!(def.conclusion.bindings["value"], "?v");
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_explicit_this_with_same_binding_passes_through() {
        let yaml = r#"
ns:
  rule:
    deduce:
      ns/Output:
        this: ?entity
        source: ?entity
        value: ?v
    when:
      - ns/Input:
          this: ?entity
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.conclusion.this.as_deref(), Some("?entity"));
        assert_eq!(def.conclusion.bindings["source"], "?entity");
        assert_eq!(def.conclusion.bindings["value"], "?v");
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_binding_var_not_entity() {
        let yaml = r#"
ns:
  rule:
    deduce:
      ns/Output:
        name: ?n
    when:
      - ns/Input:
          this: _
          data: ?n
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 1);
        assert_eq!(def.conclusion.bindings["name"], "?n");
    }

    #[test]
    fn lower_no_conflict() {
        let yaml = r#"
ns:
  rule:
    deduce:
      ns/Output:
        value: ?v
    when:
      - ns/Input:
          this: ?src
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 1);
        assert_eq!(def.conclusion.bindings["value"], "?v");
        assert_eq!(def.when[0].of, "?src");
    }

    #[test]
    fn lower_premise_no_this_defaults_to_wildcard() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Out:
        val: ?v
    when:
      - ns/In:
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.when.len(), 1);
        assert_eq!(def.when[0].the, "in/data");
        assert_eq!(def.when[0].of, "_");
        assert_eq!(def.when[0].is, "?v");
    }

    #[test]
    fn lower_raw_attribute_premise() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Output:
        name: ?n
    when:
      - diy.cook/ingredient-name:
          this: ?entity
          is: ?n
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.when.len(), 1);
        assert_eq!(def.when[0].the, "diy.cook/ingredient-name");
        assert_eq!(def.when[0].of, "?entity");
        assert_eq!(def.when[0].is, "?n");
    }

    #[test]
    fn lower_equality_constraint() {
        let yaml = r#"
ns:
  my-rule:
    deduce:
      ns/Output:
        name: ?name
    when:
      - ns/Person:
          this: ?p
          name: ?name
      - ==:
          this: ?name
          is: Alice
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        assert_eq!(def.when.len(), 2);
        // First premise: concept
        assert_eq!(def.when[0].the, "person/name");
        // Second premise: equality constraint
        assert_eq!(def.when[1].the, "==");
        assert_eq!(def.when[1].of, "?name");
        assert_eq!(def.when[1].is, "Alice");
    }

    #[test]
    fn parse_rules_example_file() {
        let yaml = include_str!("../../examples/rules.yaml");
        let entries = super::super::concept_parse::parse_yaml(yaml).unwrap();

        // Extract rule entries
        let mut rules = Vec::new();
        for entry in entries {
            if let super::super::concept_parse::ParsedEntry::Rule {
                name,
                namespace,
                value,
            } = entry
            {
                let parsed = parse_rule(&namespace, &name, &value).unwrap();
                rules.push(parsed);
            }
        }

        assert_eq!(rules.len(), 1);

        // plan-event-meal -> Meal
        let rule1 = &rules[0];
        assert_eq!(rule1.name, "plan-event-meal");
        assert_eq!(rule1.conclusion.concept_name, "Meal");
        assert!(rule1.description.is_some());
        let def1 = lower_rule(rule1).unwrap();
        assert_eq!(def1.conclusion.concept, "Meal");
        assert!(!def1.when.is_empty());
        assert!(def1.unless.is_empty());
    }
}
