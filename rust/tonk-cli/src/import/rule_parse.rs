//! YAML parsing and lowering for rule definitions.
//!
//! Parses a YAML file of the form:
//!
//! ```yaml
//! user.rules:
//!   respect-dietary-restrictions:
//!     assert:
//!       diy.meal-planner/Meal:
//!         attendee: ?person
//!         recipe: ?recipe
//!     when:
//!       - diy.cook/Recipe:
//!           this: ?recipe
//!           ingredient: ?ingredient
//!     unless:
//!       - diy.health/Allergy:
//!           this: _
//!           person: ?person
//!           substance: ?substance
//! ```
//!
//! After parsing, the concept-oriented representation is *lowered* into
//! flat EAV-based `RuleDefinition`s for the engine.

use crate::rule::{RuleConclusion, RuleDefinition, RulePremise};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Parsed intermediate representation
// ---------------------------------------------------------------------------

/// A rule parsed from YAML, ready to be lowered into a `RuleDefinition`.
#[derive(Debug)]
pub(super) struct ParsedRule {
    /// Rule name from the YAML key (e.g. "respect-dietary-restrictions").
    pub name: String,
    /// Namespace from the top-level key (e.g. "user.rules").
    pub namespace: String,
    /// The conclusion: which concept is asserted and its bindings.
    pub conclusion: ParsedRuleConclusion,
    /// Positive premises (must hold).
    pub when: Vec<ParsedRulePremise>,
    /// Negative premises (must not hold).
    pub unless: Vec<ParsedRulePremise>,
}

/// The conclusion of a parsed rule.
#[derive(Debug)]
pub(super) struct ParsedRuleConclusion {
    /// Concept name extracted from the reference (e.g. "Meal" from "diy.meal-planner/Meal").
    pub concept_name: String,
    /// Bindings: attribute short names to variables/wildcards/constants.
    /// The `this` key, if present, is stored separately.
    pub bindings: HashMap<String, String>,
    /// Optional explicit `this` binding from the conclusion.
    pub this: Option<String>,
}

/// A single concept-oriented premise from the YAML, before lowering to EAV.
#[derive(Debug)]
pub(super) struct ParsedRulePremise {
    /// Concept name extracted from the reference (e.g. "Recipe" from "diy.cook/Recipe").
    pub concept_name: String,
    /// The `this` value (entity binding). Defaults to `"_"` if absent.
    pub this_value: String,
    /// Attribute bindings: short name -> variable/wildcard/constant.
    /// Does not include `this`.
    pub attributes: Vec<(String, String)>,
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
/// Structure: `namespace -> rule_name -> {assert, when, unless}`.
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
fn parse_rule(namespace: &str, rule_name: &str, value: &serde_yaml::Value) -> Result<ParsedRule> {
    let map: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_value(value.clone())
        .context("Expected a mapping with assert/when/unless")?;

    // Parse `assert` (required)
    let assert_value = map
        .get("assert")
        .ok_or_else(|| anyhow::anyhow!("Rule must have an 'assert' section"))?;

    let conclusion =
        parse_rule_conclusion(assert_value).context("Failed to parse 'assert' section")?;

    // Parse `when` (required, list of concept premises)
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
        conclusion,
        when,
        unless,
    })
}

/// Parse the `assert` section of a rule YAML.
///
/// Expected shape: a single-key mapping where the key is a concept reference
/// and the value is a mapping of attribute bindings.
///
/// ```yaml
/// assert:
///   diy.meal-planner/Meal:
///     attendee: ?person
///     recipe: ?recipe
/// ```
fn parse_rule_conclusion(value: &serde_yaml::Value) -> Result<ParsedRuleConclusion> {
    let map: BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_value(value.clone()).context("Expected a concept mapping")?;

    if map.len() != 1 {
        anyhow::bail!(
            "The 'assert' section must contain exactly one concept, found {}",
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

/// Parse the `when` or `unless` section (a list of concept premises).
///
/// Each entry is a single-key mapping: concept reference -> attribute bindings.
///
/// ```yaml
/// when:
///   - diy.cook/Recipe:
///       this: ?recipe
///       title: _
///       ingredient: ?ingredient
/// ```
fn parse_rule_premises(value: &serde_yaml::Value) -> Result<Vec<ParsedRulePremise>> {
    let list: Vec<serde_yaml::Value> =
        serde_yaml::from_value(value.clone()).context("Expected a list of premises")?;

    let mut premises = Vec::new();

    for (i, entry) in list.iter().enumerate() {
        let map: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_value(entry.clone())
            .with_context(|| format!("Premise {} must be a concept mapping", i + 1))?;

        if map.len() != 1 {
            anyhow::bail!(
                "Premise {} must reference exactly one concept, found {}",
                i + 1,
                map.len()
            );
        }

        let (concept_ref, attrs_value) = map.into_iter().next().unwrap();
        let concept_name = extract_concept_name(&concept_ref);

        let attrs_map: BTreeMap<String, String> = serde_yaml::from_value(attrs_value)
            .with_context(|| {
                format!(
                    "Premise {} ({}) must have attribute bindings (e.g. this: ?var, name: _)",
                    i + 1,
                    concept_ref
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
/// attribute mentioned. The `this` key maps to the entity (`of`) field.
/// Attributes are qualified using the concept name as namespace prefix
/// (e.g. attribute `title` on concept `Recipe` becomes `recipe/title`).
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

/// Lower a list of parsed concept-oriented premises into flat EAV triples.
///
/// Each attribute mentioned in a premise produces one `RulePremise` with:
/// - `the`: fully qualified attribute (`concept_name_lowercase/attr_name`)
/// - `of`: the entity binding from `this` (or `"_"` wildcard)
/// - `is`: the attribute's value binding
fn lower_premises(premises: &[ParsedRulePremise]) -> Result<Vec<RulePremise>> {
    let mut eav_premises = Vec::new();

    for premise in premises {
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
    fn parse_basic_rule() {
        let yaml = r#"
user.rules:
  my-rule:
    assert:
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
        assert_eq!(rule.conclusion.concept_name, "Meal");
        assert_eq!(rule.conclusion.bindings.len(), 2);
        assert_eq!(rule.conclusion.bindings["attendee"], "?person");
        assert_eq!(rule.conclusion.bindings["recipe"], "?recipe");
        assert!(rule.conclusion.this.is_none());

        assert_eq!(rule.when.len(), 2);
        assert_eq!(rule.when[0].concept_name, "Recipe");
        assert_eq!(rule.when[0].this_value, "?recipe");
        // title and ingredient (BTreeMap sorted: ingredient, title)
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
    assert:
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
    assert:
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

    // -----------------------------------------------------------------------
    // Rule lowering tests
    // -----------------------------------------------------------------------

    #[test]
    fn lower_basic_rule() {
        let yaml = r#"
user.rules:
  my-rule:
    assert:
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

        // Lowering passes variables through as-is (no splitting)
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
        // When `this: ?entity` is in the conclusion and NO binding
        // uses ?entity, there's no conflict — no splitting needed.
        let yaml = r#"
ns:
  my-rule:
    assert:
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

        // No conflict: ?entity is only in `this`, not in bindings
        assert_eq!(def.conclusion.this.as_deref(), Some("?entity"));
        assert_eq!(def.conclusion.bindings.len(), 1);
        assert_eq!(def.conclusion.bindings["name"], "?n");
        // Premise entity stays as-is
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_passes_through_conflicting_vars() {
        // Lowering does NOT split variables. If a conclusion binding
        // variable is also used as `this` in a premise, both keep the
        // original variable. The conflict is handled later at compile time
        // (build_rename_map will skip the entity var for auto-detection).
        let yaml = r#"
ns:
  rule:
    assert:
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

        // Variables pass through as-is — no splitting
        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 2);
        assert_eq!(def.conclusion.bindings["source"], "?entity");
        assert_eq!(def.conclusion.bindings["value"], "?v");

        // Premise entity keeps original variable
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_explicit_this_with_same_binding_passes_through() {
        // Lowering passes variables through as-is. The explicit `this`
        // and binding using the same variable will be caught later at
        // compile time by build_rename_map.
        let yaml = r#"
ns:
  rule:
    assert:
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

        // Lowering just passes through; no splitting
        assert_eq!(def.conclusion.this.as_deref(), Some("?entity"));
        assert_eq!(def.conclusion.bindings["source"], "?entity");
        assert_eq!(def.conclusion.bindings["value"], "?v");
        assert_eq!(def.when[0].of, "?entity");
    }

    #[test]
    fn lower_binding_var_not_entity() {
        // Binding variable that is NOT a premise entity variable.
        // Lowering passes it through as-is.
        let yaml = r#"
ns:
  rule:
    assert:
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
        // When entity var and binding vars are different, simple pass-through.
        let yaml = r#"
ns:
  rule:
    assert:
      ns/Output:
        value: ?v
    when:
      - ns/Input:
          this: ?src
          data: ?v
"#;
        let rules = parse_rules_yaml(yaml).unwrap();
        let def = lower_rule(&rules[0]).unwrap();

        // Lowering just passes through; conclusion.this is not set by lowering
        assert!(def.conclusion.this.is_none());
        assert_eq!(def.conclusion.bindings.len(), 1);
        assert_eq!(def.conclusion.bindings["value"], "?v");
        assert_eq!(def.when[0].of, "?src");
    }

    #[test]
    fn lower_premise_no_this_defaults_to_wildcard() {
        // A premise with no `this` key should use "_" as entity
        let yaml = r#"
ns:
  my-rule:
    assert:
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
    fn parse_rules_example_file() {
        let yaml = include_str!("../../examples/rules.yaml");
        let rules = parse_rules_yaml(yaml).unwrap();
        assert_eq!(rules.len(), 2);

        // First rule: find-allergy-conflicts -> AllergyConflict
        let rule1 = rules
            .iter()
            .find(|r| r.name == "find-allergy-conflicts")
            .unwrap();
        assert_eq!(rule1.conclusion.concept_name, "AllergyConflict");
        let def1 = lower_rule(rule1).unwrap();
        assert_eq!(def1.conclusion.concept, "AllergyConflict");
        assert!(!def1.when.is_empty());
        assert!(def1.unless.is_empty());

        // Second rule: respect-dietary-restrictions -> SafeMeal
        let rule2 = rules
            .iter()
            .find(|r| r.name == "respect-dietary-restrictions")
            .unwrap();
        assert_eq!(rule2.conclusion.concept_name, "SafeMeal");
        let def2 = lower_rule(rule2).unwrap();
        assert_eq!(def2.conclusion.concept, "SafeMeal");
        assert!(!def2.when.is_empty());
        assert!(!def2.unless.is_empty());
    }
}
