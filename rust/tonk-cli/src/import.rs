//! YAML import for concepts and rules.
//!
//! Parses a YAML file and auto-detects whether it contains concept definitions
//! or rule definitions, then imports them atomically into the active space.
//!
//! # Concept YAML format
//!
//! ```yaml
//! diy.cook:
//!   Recipe:
//!     this:
//!       the: Meal recipe
//!     title:
//!       the: The name of this recipe
//!       as: Text
//!     ingredient:
//!       the: Ingredients of the recipe
//!       cardinality: many
//! ```
//!
//! # Rule YAML format
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

use crate::rule::{RuleConclusion, RuleDefinition, RulePremise};
use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::Value;
use dialog_query::claim::Attribute;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// YAML deserialization types
// ---------------------------------------------------------------------------

/// An attribute definition in the YAML schema. Each key under a concept
/// (other than `this`) is deserialized into this type.
///
/// All fields are optional — a bare key with no value is a valid attribute
/// with no metadata.
#[derive(Debug, Default)]
struct YamlAttributeDef {
    /// Human-readable description of the attribute.
    the: Option<String>,

    /// Type constraint. Can be a single string (`"Text"`, `"Integer"`,
    /// a concept name like `"RecipeStep"`) or an array of strings for
    /// enum-like constraints (`["tsp", "mls"]`).
    as_type: Option<serde_yaml::Value>,

    /// Cardinality: `"many"` for multi-valued attributes. Absent means single.
    cardinality: Option<String>,

    /// Whether the `optional` key was present (even with a null value).
    optional_present: bool,
}

// ---------------------------------------------------------------------------
// Parsed intermediate representation
// ---------------------------------------------------------------------------

/// A concept parsed from the YAML, ready for validation and import.
struct ParsedConcept {
    /// The concept name (e.g. "Recipe").
    name: String,
    /// The namespace it was defined under (e.g. "diy.cook").
    namespace: String,
    /// Description from the `this.the` field.
    description: Option<String>,
    /// The concept's attributes with their metadata.
    attributes: Vec<ParsedAttribute>,
}

/// A single attribute of a parsed concept.
struct ParsedAttribute {
    /// Short attribute name (e.g. "title").
    short_name: String,
    /// Description from the `the` field.
    description: Option<String>,
    /// Type constraint from `as`, stored verbatim. Arrays are JSON-encoded.
    type_str: Option<String>,
    /// Cardinality string (e.g. "many").
    cardinality: Option<String>,
    /// Whether this attribute is optional.
    optional: bool,
}

// ---------------------------------------------------------------------------
// YAML parsing
// ---------------------------------------------------------------------------

/// Parse a YAML string into a list of concepts.
///
/// The YAML structure is: `namespace -> concept_name -> attr_name -> attr_def`.
/// The `this` key is special and provides the concept-level description.
fn parse_yaml(yaml_str: &str) -> Result<Vec<ParsedConcept>> {
    // Parse as a map of namespace -> concepts
    let root: BTreeMap<String, BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(yaml_str).context("Failed to parse YAML")?;

    let mut concepts = Vec::new();

    for (namespace, concept_map) in root {
        for (concept_name, attrs_value) in concept_map {
            let parsed = parse_concept(&namespace, &concept_name, &attrs_value)
                .with_context(|| format!("In concept '{}.{}'", namespace, concept_name))?;
            concepts.push(parsed);
        }
    }

    Ok(concepts)
}

/// Parse a single concept from its YAML value (the map of attribute names to defs).
fn parse_concept(
    namespace: &str,
    concept_name: &str,
    value: &serde_yaml::Value,
) -> Result<ParsedConcept> {
    let attrs_map: BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_value(value.clone()).context("Expected a mapping of attributes")?;

    let mut description = None;
    let mut attributes = Vec::new();

    for (key, attr_value) in attrs_map {
        if key == "this" {
            // `this` is not an attribute — it describes the entity itself
            let def = parse_attr_def(&attr_value).context("Failed to parse 'this' definition")?;
            description = def.the;
            continue;
        }

        let def = parse_attr_def(&attr_value)
            .with_context(|| format!("Failed to parse attribute '{}'", key))?;

        let type_str = match &def.as_type {
            None => None,
            Some(serde_yaml::Value::String(s)) => Some(s.clone()),
            Some(serde_yaml::Value::Sequence(seq)) => {
                // Array of strings → JSON array
                let items: Vec<String> = seq
                    .iter()
                    .map(|v| match v {
                        serde_yaml::Value::String(s) => Ok(s.clone()),
                        other => Ok(format!("{:?}", other)),
                    })
                    .collect::<Result<Vec<_>>>()?;
                Some(serde_json::to_string(&items)?)
            }
            Some(other) => Some(format!("{:?}", other)),
        };

        let optional = def.optional_present;

        attributes.push(ParsedAttribute {
            short_name: key,
            description: def.the,
            type_str,
            cardinality: def.cardinality,
            optional,
        });
    }

    Ok(ParsedConcept {
        name: concept_name.to_string(),
        namespace: namespace.to_string(),
        description,
        attributes,
    })
}

/// Parse an attribute definition value. Handles three forms:
/// - A full mapping `{the: ..., as: ..., ...}`
/// - A bare scalar (e.g. `title:` with no value → null → default)
/// - A string shorthand (e.g. `title: Text` → treated as type only)
fn parse_attr_def(value: &serde_yaml::Value) -> Result<YamlAttributeDef> {
    match value {
        serde_yaml::Value::Null => Ok(YamlAttributeDef::default()),
        serde_yaml::Value::String(s) => {
            // Bare string → treat as type shorthand
            Ok(YamlAttributeDef {
                as_type: Some(serde_yaml::Value::String(s.clone())),
                ..Default::default()
            })
        }
        serde_yaml::Value::Mapping(map) => {
            // Manually extract fields so we can detect the *presence* of `optional:`
            // (even when its value is null), which serde's Option<T> would swallow.
            let the = map
                .get(serde_yaml::Value::String("the".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let as_type = map.get(serde_yaml::Value::String("as".into())).cloned();

            let cardinality = map
                .get(serde_yaml::Value::String("cardinality".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let optional_present = map.contains_key(serde_yaml::Value::String("optional".into()));

            Ok(YamlAttributeDef {
                the,
                as_type,
                cardinality,
                optional_present,
            })
        }
        other => anyhow::bail!(
            "Unexpected value type for attribute definition: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Rule YAML parsing types
// ---------------------------------------------------------------------------

/// A rule parsed from YAML, ready to be lowered into a `RuleDefinition`.
#[derive(Debug)]
struct ParsedRule {
    /// Rule name from the YAML key (e.g. "respect-dietary-restrictions").
    name: String,
    /// Namespace from the top-level key (e.g. "user.rules").
    namespace: String,
    /// The conclusion: which concept is asserted and its bindings.
    conclusion: ParsedRuleConclusion,
    /// Positive premises (must hold).
    when: Vec<ParsedRulePremise>,
    /// Negative premises (must not hold).
    unless: Vec<ParsedRulePremise>,
}

/// The conclusion of a parsed rule.
#[derive(Debug)]
struct ParsedRuleConclusion {
    /// Concept name extracted from the reference (e.g. "Meal" from "diy.meal-planner/Meal").
    concept_name: String,
    /// Bindings: attribute short names to variables/wildcards/constants.
    /// The `this` key, if present, is stored separately.
    bindings: HashMap<String, String>,
    /// Optional explicit `this` binding from the conclusion.
    this: Option<String>,
}

/// A single concept-oriented premise from the YAML, before lowering to EAV.
#[derive(Debug)]
struct ParsedRulePremise {
    /// Concept name extracted from the reference (e.g. "Recipe" from "diy.cook/Recipe").
    concept_name: String,
    /// The `this` value (entity binding). Defaults to `"_"` if absent.
    this_value: String,
    /// Attribute bindings: short name → variable/wildcard/constant.
    /// Does not include `this`.
    attributes: Vec<(String, String)>,
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
fn parse_rules_yaml(yaml_str: &str) -> Result<Vec<ParsedRule>> {
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
/// Each entry is a single-key mapping: concept reference → attribute bindings.
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
// Rule lowering: YAML → RuleDefinition
// ---------------------------------------------------------------------------

/// Lower a `ParsedRule` into the internal `RuleDefinition` (EAV-based).
///
/// Each concept-oriented premise is expanded into one EAV premise per
/// attribute mentioned. The `this` key maps to the entity (`of`) field.
/// Attributes are qualified using the concept name as namespace prefix
/// (e.g. attribute `title` on concept `Recipe` becomes `recipe/title`).
fn lower_rule(parsed: &ParsedRule) -> Result<RuleDefinition> {
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
// YAML file type detection
// ---------------------------------------------------------------------------

/// The type of content in a YAML import file.
#[derive(Debug, PartialEq, Eq)]
enum YamlFileType {
    Concepts,
    Rules,
}

/// Detect whether a YAML string contains concept or rule definitions.
///
/// Inspects the second-level values: if any contains `assert` or `when`
/// keys, it's a rule file. Otherwise it's a concept file.
fn detect_yaml_type(yaml_str: &str) -> Result<YamlFileType> {
    let root: BTreeMap<String, BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(yaml_str).context("Failed to parse YAML")?;

    for entries in root.values() {
        for value in entries.values() {
            if let serde_yaml::Value::Mapping(map) = value {
                let has_assert = map.contains_key(serde_yaml::Value::String("assert".into()));
                let has_when = map.contains_key(serde_yaml::Value::String("when".into()));
                if has_assert || has_when {
                    return Ok(YamlFileType::Rules);
                }
            }
        }
    }

    Ok(YamlFileType::Concepts)
}

// ---------------------------------------------------------------------------
// Import command (entry point)
// ---------------------------------------------------------------------------

/// Import concepts or rules from a YAML file into the active space.
///
/// Auto-detects the file type based on the YAML structure, then dispatches
/// to the appropriate import handler.
///
/// All concepts in the file are validated first, then committed atomically.
/// If `force` is true, existing concepts are overwritten (retracted then
/// re-created); otherwise any collision fails the entire import.
pub async fn import(file: String, force: bool, json: bool) -> Result<()> {
    let yaml_str =
        std::fs::read_to_string(&file).context(format!("Failed to read file: {}", file))?;

    match detect_yaml_type(&yaml_str)? {
        YamlFileType::Concepts => import_concepts(&yaml_str, &file, force, json).await,
        YamlFileType::Rules => import_rules(&yaml_str, &file, force, json).await,
    }
}

// ---------------------------------------------------------------------------
// Concept import
// ---------------------------------------------------------------------------

/// Import concepts from a parsed YAML string into the active space.
///
/// All concepts are validated first, then committed atomically.
async fn import_concepts(yaml_str: &str, file: &str, force: bool, json: bool) -> Result<()> {
    let concepts = parse_yaml(yaml_str)?;

    if concepts.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"ok": true, "imported": []}))?
            );
        } else {
            println!("No concepts found in YAML file.");
        }
        return Ok(());
    }

    // --- Validation phase ---

    let mut validated: Vec<(ConceptName, &ParsedConcept)> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for concept in &concepts {
        let cname = ConceptName::new(&concept.name)?;

        let lower = cname.to_lowercase();
        if !seen_names.insert(lower.clone()) {
            anyhow::bail!(
                "Duplicate concept name '{}' in YAML file. \
                 Concept names must be unique (case-insensitive).",
                concept.name
            );
        }

        if concept.attributes.is_empty() {
            anyhow::bail!(
                "Concept '{}' has no attributes. A concept must have at least one attribute.",
                concept.name
            );
        }

        for attr in &concept.attributes {
            if attr.short_name.contains('/') {
                anyhow::bail!(
                    "Attribute name '{}' in concept '{}' must not contain '/'. \
                     Use short names only (e.g. 'title', not 'recipe/title').",
                    attr.short_name,
                    concept.name,
                );
            }
            validate_safe_name(&attr.short_name, "Attribute")?;
        }

        validated.push((cname, concept));
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;
    let registry = registry_entity(&ctx.space_did)?;

    let mut retract_instructions: Vec<Instruction> = Vec::new();

    for (cname, _concept) in &validated {
        let entity = concept_entity(&ctx.space_did, cname)?;
        let existing = fetch_string(&branch, &entity, ATTR_CONCEPT_NAME).await?;

        if existing.is_some() {
            if force {
                let existing_attrs =
                    fetch_string_values(&branch, &entity, ATTR_CONCEPT_ATTRIBUTE).await?;

                for attr_name in &existing_attrs {
                    let meta_entity = attribute_meta_entity(&ctx.space_did, cname, attr_name)?;

                    for meta_attr in &[
                        ATTR_ATTRIBUTE_DESCRIPTION,
                        ATTR_ATTRIBUTE_TYPE,
                        ATTR_ATTRIBUTE_CARDINALITY,
                        ATTR_ATTRIBUTE_OPTIONAL,
                    ] {
                        if let Some(val) = fetch_string(&branch, &meta_entity, meta_attr).await? {
                            retract_instructions.push(Instruction::Retract(Artifact {
                                the: Attribute::from_str(meta_attr)?,
                                of: meta_entity.clone(),
                                is: Value::String(val),
                                cause: None,
                            }));
                        }
                    }

                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                        of: entity.clone(),
                        is: Value::String(attr_name.clone()),
                        cause: None,
                    }));
                }

                if let Some(name) = fetch_string(&branch, &entity, ATTR_CONCEPT_NAME).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
                        of: entity.clone(),
                        is: Value::String(name),
                        cause: None,
                    }));
                }

                if let Some(desc) = fetch_string(&branch, &entity, ATTR_CONCEPT_DESCRIPTION).await?
                {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
                        of: entity.clone(),
                        is: Value::String(desc),
                        cause: None,
                    }));
                }

                if let Some(ns) = fetch_string(&branch, &entity, ATTR_CONCEPT_NAMESPACE).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
                        of: entity.clone(),
                        is: Value::String(ns),
                        cause: None,
                    }));
                }

                retract_instructions.push(Instruction::Retract(Artifact {
                    the: Attribute::from_str(ATTR_REGISTRY_CONCEPT)?,
                    of: registry.clone(),
                    is: Value::Entity(entity.clone()),
                    cause: None,
                }));
            } else {
                anyhow::bail!(
                    "Concept '{}' already exists. Use --force to overwrite, \
                     or delete it first with 'tonk concept delete {}'.",
                    cname,
                    cname
                );
            }
        }
    }

    // --- Build assert instructions ---

    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (cname, concept) in &validated {
        let entity = concept_entity(&ctx.space_did, cname)?;

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_REGISTRY_CONCEPT)?,
            of: registry.clone(),
            is: Value::Entity(entity.clone()),
            cause: None,
        }));

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
            of: entity.clone(),
            is: Value::String(cname.to_string()),
            cause: None,
        }));

        if let Some(desc) = &concept.description {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
                of: entity.clone(),
                is: Value::String(desc.clone()),
                cause: None,
            }));
        }

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
            of: entity.clone(),
            is: Value::String(concept.namespace.clone()),
            cause: None,
        }));

        let mut attr_summaries: Vec<String> = Vec::new();

        for attr in &concept.attributes {
            let qualified = qualify_attribute(cname, &attr.short_name)?;

            assert_instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                of: entity.clone(),
                is: Value::String(qualified.clone()),
                cause: None,
            }));

            let meta_entity = attribute_meta_entity(&ctx.space_did, cname, &qualified)?;

            if let Some(desc) = &attr.description {
                assert_instructions.push(Instruction::Assert(Artifact {
                    the: Attribute::from_str(ATTR_ATTRIBUTE_DESCRIPTION)?,
                    of: meta_entity.clone(),
                    is: Value::String(desc.clone()),
                    cause: None,
                }));
            }

            if let Some(type_str) = &attr.type_str {
                assert_instructions.push(Instruction::Assert(Artifact {
                    the: Attribute::from_str(ATTR_ATTRIBUTE_TYPE)?,
                    of: meta_entity.clone(),
                    is: Value::String(type_str.clone()),
                    cause: None,
                }));
            }

            if let Some(cardinality) = &attr.cardinality {
                assert_instructions.push(Instruction::Assert(Artifact {
                    the: Attribute::from_str(ATTR_ATTRIBUTE_CARDINALITY)?,
                    of: meta_entity.clone(),
                    is: Value::String(cardinality.clone()),
                    cause: None,
                }));
            }

            if attr.optional {
                assert_instructions.push(Instruction::Assert(Artifact {
                    the: Attribute::from_str(ATTR_ATTRIBUTE_OPTIONAL)?,
                    of: meta_entity.clone(),
                    is: Value::String("true".to_string()),
                    cause: None,
                }));
            }

            attr_summaries.push(attr.short_name.clone());
        }

        import_summary.push(serde_json::json!({
            "name": cname.as_str(),
            "namespace": concept.namespace,
            "attributes": attr_summaries,
            "description": concept.description,
        }));
    }

    // --- Atomic commit ---

    let mut all_instructions = retract_instructions;
    all_instructions.extend(assert_instructions);

    branch
        .commit(futures_util::stream::iter(all_instructions))
        .await?;

    // --- Output ---

    if json {
        let output = serde_json::json!({
            "ok": true,
            "type": "concepts",
            "imported": import_summary,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Imported {} concept(s) from '{}':\n", concepts.len(), file);
        for (cname, concept) in &validated {
            let desc_str = concept
                .description
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {} [{}]{}", cname, concept.namespace, desc_str);
            for attr in &concept.attributes {
                let mut meta_parts = Vec::new();
                if let Some(t) = &attr.type_str {
                    meta_parts.push(format!("as: {}", t));
                }
                if let Some(c) = &attr.cardinality {
                    meta_parts.push(format!("cardinality: {}", c));
                }
                if attr.optional {
                    meta_parts.push("optional".to_string());
                }
                let meta_str = if meta_parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", meta_parts.join(", "))
                };
                let desc = attr
                    .description
                    .as_ref()
                    .map(|d| format!(" - {}", d))
                    .unwrap_or_default();
                println!("    {}{}{}", attr.short_name, meta_str, desc);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rule import
// ---------------------------------------------------------------------------

/// Import rules from a parsed YAML string into the active space.
///
/// All rules are parsed, lowered to `RuleDefinition`, validated against
/// the existing concept schemas, trial-compiled, then committed atomically.
async fn import_rules(yaml_str: &str, file: &str, force: bool, json: bool) -> Result<()> {
    let rules = parse_rules_yaml(yaml_str)?;

    if rules.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"ok": true, "type": "rules", "imported": []})
                )?
            );
        } else {
            println!("No rules found in YAML file.");
        }
        return Ok(());
    }

    // --- Parse and lower all rules ---

    let mut lowered: Vec<(&ParsedRule, RuleDefinition)> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for rule in &rules {
        validate_safe_name(&rule.name, "Rule")?;

        let lower = rule.name.to_lowercase();
        if !seen_names.insert(lower) {
            anyhow::bail!(
                "Duplicate rule name '{}' in YAML file. \
                 Rule names must be unique (case-insensitive).",
                rule.name
            );
        }

        let definition =
            lower_rule(rule).with_context(|| format!("Failed to lower rule '{}'", rule.name))?;

        lowered.push((rule, definition));
    }

    // --- Validate against the space ---

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;
    let registry = registry_entity(&ctx.space_did)?;

    let mut retract_instructions: Vec<Instruction> = Vec::new();

    for (parsed, definition) in &lowered {
        // Check if rule already exists
        let rule_ent = rule_entity(&ctx.space_did, &parsed.name)?;
        let existing = fetch_string(&branch, &rule_ent, ATTR_RULE_NAME).await?;

        if existing.is_some() {
            if force {
                // Retract existing rule
                if let Some(name) = fetch_string(&branch, &rule_ent, ATTR_RULE_NAME).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_RULE_NAME)?,
                        of: rule_ent.clone(),
                        is: Value::String(name),
                        cause: None,
                    }));
                }
                if let Some(conclusion) =
                    fetch_string(&branch, &rule_ent, ATTR_RULE_CONCLUSION).await?
                {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
                        of: rule_ent.clone(),
                        is: Value::String(conclusion),
                        cause: None,
                    }));
                }
                if let Some(def) = fetch_string(&branch, &rule_ent, ATTR_RULE_DEFINITION).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_RULE_DEFINITION)?,
                        of: rule_ent.clone(),
                        is: Value::String(def),
                        cause: None,
                    }));
                }
                if let Some(desc) = fetch_string(&branch, &rule_ent, ATTR_RULE_DESCRIPTION).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_RULE_DESCRIPTION)?,
                        of: rule_ent.clone(),
                        is: Value::String(desc),
                        cause: None,
                    }));
                }
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: Attribute::from_str(ATTR_REGISTRY_RULE)?,
                    of: registry.clone(),
                    is: Value::Entity(rule_ent.clone()),
                    cause: None,
                }));
            } else {
                anyhow::bail!(
                    "Rule '{}' already exists. Use --force to overwrite, \
                     or delete it first with 'tonk rule delete {}'.",
                    parsed.name,
                    parsed.name
                );
            }
        }

        // Validate the conclusion concept exists
        let conclusion_concept = ConceptName::new(&definition.conclusion.concept)?;
        let concept_ent = concept_entity(&ctx.space_did, &conclusion_concept)?;
        let concept_name = fetch_string(&branch, &concept_ent, ATTR_CONCEPT_NAME)
            .await?
            .context(format!(
                "Conclusion concept '{}' for rule '{}' not found. Define it first.",
                definition.conclusion.concept, parsed.name
            ))?;
        let concept_name = ConceptName::from_stored(concept_name);

        let concept_attrs =
            fetch_string_values(&branch, &concept_ent, ATTR_CONCEPT_ATTRIBUTE).await?;

        // Validate bindings match concept schema
        crate::rule::validate_definition(definition, &concept_attrs, &concept_name)
            .with_context(|| format!("Rule '{}' validation failed", parsed.name))?;

        // Trial-compile
        crate::rule::compile_rule(definition, &concept_name, &concept_attrs).with_context(
            || {
                format!(
                    "Rule '{}' failed to compile. Check variable names match between \
                     conclusion bindings and premises.",
                    parsed.name
                )
            },
        )?;
    }

    // --- Build assert instructions ---

    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (parsed, definition) in &lowered {
        let rule_ent = rule_entity(&ctx.space_did, &parsed.name)?;
        let concept_name = &definition.conclusion.concept;
        let definition_str = serde_json::to_string(definition)?;

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_REGISTRY_RULE)?,
            of: registry.clone(),
            is: Value::Entity(rule_ent.clone()),
            cause: None,
        }));

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_NAME)?,
            of: rule_ent.clone(),
            is: Value::String(parsed.name.clone()),
            cause: None,
        }));

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_CONCLUSION)?,
            of: rule_ent.clone(),
            is: Value::String(concept_name.clone()),
            cause: None,
        }));

        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_RULE_DEFINITION)?,
            of: rule_ent.clone(),
            is: Value::String(definition_str),
            cause: None,
        }));

        import_summary.push(serde_json::json!({
            "name": parsed.name,
            "namespace": parsed.namespace,
            "conclusion": concept_name,
            "when_count": definition.when.len(),
            "unless_count": definition.unless.len(),
        }));
    }

    // --- Atomic commit ---

    let mut all_instructions = retract_instructions;
    all_instructions.extend(assert_instructions);

    branch
        .commit(futures_util::stream::iter(all_instructions))
        .await?;

    // --- Output ---

    if json {
        let output = serde_json::json!({
            "ok": true,
            "type": "rules",
            "imported": import_summary,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Imported {} rule(s) from '{}':\n", rules.len(), file);
        for (parsed, definition) in &lowered {
            println!(
                "  {} [{}] -> {}",
                parsed.name, parsed.namespace, definition.conclusion.concept
            );
            println!(
                "    {} when, {} unless",
                definition.when.len(),
                definition.unless.len()
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_concept() {
        let yaml = r#"
diy.cook:
  Recipe:
    this:
      the: Meal recipe
    title:
      the: The name of this recipe
      as: Text
    ingredient:
      the: Ingredients of the recipe
      cardinality: many
"#;
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(concepts[0].name, "Recipe");
        assert_eq!(concepts[0].namespace, "diy.cook");
        assert_eq!(concepts[0].description.as_deref(), Some("Meal recipe"));
        assert_eq!(concepts[0].attributes.len(), 2);

        // BTreeMap sorts alphabetically: ingredient before title
        let ingredient = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "ingredient")
            .unwrap();
        assert_eq!(
            ingredient.description.as_deref(),
            Some("Ingredients of the recipe")
        );
        assert_eq!(ingredient.cardinality.as_deref(), Some("many"));

        let title = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "title")
            .unwrap();
        assert_eq!(
            title.description.as_deref(),
            Some("The name of this recipe")
        );
        assert_eq!(title.type_str.as_deref(), Some("Text"));
        assert!(!title.optional);
        assert!(title.cardinality.is_none());
    }

    #[test]
    fn parse_enum_type() {
        let yaml = r#"
ns:
  Thing:
    unit:
      the: Unit of measurement
      as: [tsp, mls]
"#;
        let concepts = parse_yaml(yaml).unwrap();
        let attr = &concepts[0].attributes[0];
        assert_eq!(attr.short_name, "unit");
        // Stored as JSON array
        let parsed: Vec<String> = serde_json::from_str(attr.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["tsp", "mls"]);
    }

    #[test]
    fn parse_optional_attribute() {
        let yaml = r#"
ns:
  Thing:
    name:
      the: The name
      as: Text
    after:
      the: Step to perform this after
      as: RecipeStep
      optional:
"#;
        let concepts = parse_yaml(yaml).unwrap();
        let name_attr = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "name")
            .unwrap();
        assert!(!name_attr.optional);

        // `optional:` with null value → true
        let after_attr = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "after")
            .unwrap();
        assert!(after_attr.optional);
    }

    #[test]
    fn parse_multiple_namespaces() {
        let yaml = r#"
diy.cook:
  Recipe:
    this:
      the: Meal recipe
    title:
      the: The name
      as: Text

diy.health:
  Allergy:
    this:
      the: The allergy person has
    person:
      the: Person having an allergy
    substance:
      the: Substance person has an allergy for
      as: Text
"#;
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts.len(), 2);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Recipe"));
        assert!(names.contains(&"Allergy"));
    }

    #[test]
    fn parse_empty_yaml() {
        let yaml = "";
        // serde_yaml treats empty string as null, which fails to parse as BTreeMap
        // Our parser should handle this gracefully
        let result = parse_yaml(yaml);
        // Empty YAML is fine - just no concepts
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn concept_with_no_this() {
        let yaml = r#"
ns:
  Simple:
    name:
      the: A name
      as: Text
"#;
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts[0].description, None);
        assert_eq!(concepts[0].attributes.len(), 1);
    }

    #[test]
    fn parse_cook_example() {
        let yaml = include_str!("../examples/cook.yaml");
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts.len(), 3);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Recipe"));
        assert!(names.contains(&"Ingredient"));
        assert!(names.contains(&"RecipeStep"));

        // All under the same namespace
        for c in &concepts {
            assert_eq!(c.namespace, "diy.cook");
        }

        // Recipe has 3 attrs: title, ingredient, steps
        let recipe = concepts.iter().find(|c| c.name == "Recipe").unwrap();
        assert_eq!(recipe.description.as_deref(), Some("Meal recipe"));
        assert_eq!(recipe.attributes.len(), 3);

        let steps = recipe
            .attributes
            .iter()
            .find(|a| a.short_name == "steps")
            .unwrap();
        assert_eq!(steps.cardinality.as_deref(), Some("many"));
        assert_eq!(steps.type_str.as_deref(), Some("RecipeStep"));

        // Ingredient has enum type on unit
        let ingredient = concepts.iter().find(|c| c.name == "Ingredient").unwrap();
        let unit = ingredient
            .attributes
            .iter()
            .find(|a| a.short_name == "unit")
            .unwrap();
        let parsed: Vec<String> = serde_json::from_str(unit.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["tsp", "mls"]);

        // RecipeStep.after is optional
        let step = concepts.iter().find(|c| c.name == "RecipeStep").unwrap();
        let after = step
            .attributes
            .iter()
            .find(|a| a.short_name == "after")
            .unwrap();
        assert!(after.optional);
        assert_eq!(after.type_str.as_deref(), Some("RecipeStep"));
    }

    #[test]
    fn parse_planner_example() {
        let yaml = include_str!("../examples/planner.yaml");
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts.len(), 5);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Allergy"));
        assert!(names.contains(&"Event"));
        assert!(names.contains(&"Meal"));
        assert!(names.contains(&"SafeMeal"));
        assert!(names.contains(&"AllergyConflict"));

        // Allergy is under diy.health, others under diy.planner
        let allergy = concepts.iter().find(|c| c.name == "Allergy").unwrap();
        assert_eq!(allergy.namespace, "diy.health");

        let event = concepts.iter().find(|c| c.name == "Event").unwrap();
        assert_eq!(event.namespace, "diy.planner");

        let meal = concepts.iter().find(|c| c.name == "Meal").unwrap();
        assert_eq!(meal.namespace, "diy.planner");

        let safe_meal = concepts.iter().find(|c| c.name == "SafeMeal").unwrap();
        assert_eq!(safe_meal.namespace, "diy.planner");

        let conflict = concepts
            .iter()
            .find(|c| c.name == "AllergyConflict")
            .unwrap();
        assert_eq!(conflict.namespace, "diy.planner");
        assert_eq!(meal.attributes.len(), 3);

        let occasion = meal
            .attributes
            .iter()
            .find(|a| a.short_name == "occasion")
            .unwrap();
        assert_eq!(occasion.type_str.as_deref(), Some("Event"));
    }

    #[test]
    fn parse_minimal_example() {
        let yaml = include_str!("../examples/minimal.yaml");
        let concepts = parse_yaml(yaml).unwrap();
        assert_eq!(concepts.len(), 1);

        let task = &concepts[0];
        assert_eq!(task.name, "Task");
        assert_eq!(task.namespace, "app");
        assert_eq!(task.description.as_deref(), Some("A task to be completed"));
        assert_eq!(task.attributes.len(), 3);

        // status is an enum
        let status = task
            .attributes
            .iter()
            .find(|a| a.short_name == "status")
            .unwrap();
        let parsed: Vec<String> = serde_json::from_str(status.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["todo", "in-progress", "done"]);
        assert!(!status.optional);

        // priority is optional and an enum
        let priority = task
            .attributes
            .iter()
            .find(|a| a.short_name == "priority")
            .unwrap();
        assert!(priority.optional);
        let parsed: Vec<String> =
            serde_json::from_str(priority.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["low", "medium", "high"]);
    }

    // -----------------------------------------------------------------------
    // Auto-detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_concept_file() {
        let yaml = r#"
diy.cook:
  Recipe:
    this:
      the: Meal recipe
    title:
      the: The name
      as: Text
"#;
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Concepts);
    }

    #[test]
    fn detect_rule_file() {
        let yaml = r#"
user.rules:
  my-rule:
    assert:
      ns/Meal:
        attendee: ?person
    when:
      - ns/Recipe:
          this: ?recipe
          title: _
"#;
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Rules);
    }

    // -----------------------------------------------------------------------
    // Rule parsing tests
    // -----------------------------------------------------------------------

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
        let yaml = include_str!("../examples/rules.yaml");
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
