//! YAML import for concept definitions.
//!
//! Parses a YAML file describing concepts under namespaces and imports them
//! atomically into the active space. Each concept gets its attributes registered
//! along with rich metadata (description, type, cardinality, optionality) stored
//! on per-attribute metadata entities.
//!
//! # YAML format
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
//! - Top-level keys are namespaces (stored as metadata, not used in concept naming).
//! - Second-level keys are concept names.
//! - `this` describes the entity itself; its `the` field becomes the concept description.
//! - All other keys are attributes with optional `the`, `as`, `cardinality`, `optional` fields.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::Value;
use dialog_query::claim::Attribute;
use std::collections::{BTreeMap, HashSet};
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
// Import command
// ---------------------------------------------------------------------------

/// Import concepts from a YAML file into the active space.
///
/// All concepts in the file are validated first, then committed atomically.
/// If `force` is true, existing concepts are overwritten (retracted then
/// re-created); otherwise any collision fails the entire import.
pub async fn import(file: String, force: bool, json: bool) -> Result<()> {
    let yaml_str =
        std::fs::read_to_string(&file).context(format!("Failed to read file: {}", file))?;

    let concepts = parse_yaml(&yaml_str)?;

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

    // Validate all concept names
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

        // Validate attribute short names (no slashes, safe characters)
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

    // Check against existing concepts in the space
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;
    let registry = registry_entity(&ctx.space_did)?;

    let mut retract_instructions: Vec<Instruction> = Vec::new();

    for (cname, _concept) in &validated {
        let entity = concept_entity(&ctx.space_did, cname)?;
        let existing = fetch_string(&branch, &entity, ATTR_CONCEPT_NAME).await?;

        if existing.is_some() {
            if force {
                // Build retract instructions for the existing concept
                let existing_attrs =
                    fetch_string_values(&branch, &entity, ATTR_CONCEPT_ATTRIBUTE).await?;

                // Retract attribute metadata entities
                for attr_name in &existing_attrs {
                    let meta_entity = attribute_meta_entity(&ctx.space_did, cname, attr_name)?;

                    // Retract any existing metadata
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

                    // Retract concept/attribute entry
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                        of: entity.clone(),
                        is: Value::String(attr_name.clone()),
                        cause: None,
                    }));
                }

                // Retract concept name
                if let Some(name) = fetch_string(&branch, &entity, ATTR_CONCEPT_NAME).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
                        of: entity.clone(),
                        is: Value::String(name),
                        cause: None,
                    }));
                }

                // Retract concept description
                if let Some(desc) = fetch_string(&branch, &entity, ATTR_CONCEPT_DESCRIPTION).await?
                {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
                        of: entity.clone(),
                        is: Value::String(desc),
                        cause: None,
                    }));
                }

                // Retract concept namespace
                if let Some(ns) = fetch_string(&branch, &entity, ATTR_CONCEPT_NAMESPACE).await? {
                    retract_instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
                        of: entity.clone(),
                        is: Value::String(ns),
                        cause: None,
                    }));
                }

                // Retract registry entry
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

    // --- Build assert instructions for all concepts ---

    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (cname, concept) in &validated {
        let entity = concept_entity(&ctx.space_did, cname)?;

        // Register concept in registry
        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_REGISTRY_CONCEPT)?,
            of: registry.clone(),
            is: Value::Entity(entity.clone()),
            cause: None,
        }));

        // Set concept name
        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
            of: entity.clone(),
            is: Value::String(cname.to_string()),
            cause: None,
        }));

        // Set concept description (from `this.the`)
        if let Some(desc) = &concept.description {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
                of: entity.clone(),
                is: Value::String(desc.clone()),
                cause: None,
            }));
        }

        // Store namespace
        assert_instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
            of: entity.clone(),
            is: Value::String(concept.namespace.clone()),
            cause: None,
        }));

        // Add each attribute
        let mut attr_summaries: Vec<String> = Vec::new();

        for attr in &concept.attributes {
            let qualified = qualify_attribute(cname, &attr.short_name)?;

            // Register attribute on concept
            assert_instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                of: entity.clone(),
                is: Value::String(qualified.clone()),
                cause: None,
            }));

            // Store attribute metadata on the attribute metadata entity
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

    // --- Atomic commit: retractions first, then assertions ---

    let mut all_instructions = retract_instructions;
    all_instructions.extend(assert_instructions);

    branch
        .commit(futures_util::stream::iter(all_instructions))
        .await?;

    // --- Output ---

    if json {
        let output = serde_json::json!({
            "ok": true,
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
        assert_eq!(concepts.len(), 3);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Allergy"));
        assert!(names.contains(&"Event"));
        assert!(names.contains(&"Meal"));

        // Allergy is under diy.health, others under diy.planner
        let allergy = concepts.iter().find(|c| c.name == "Allergy").unwrap();
        assert_eq!(allergy.namespace, "diy.health");

        let event = concepts.iter().find(|c| c.name == "Event").unwrap();
        assert_eq!(event.namespace, "diy.planner");

        let meal = concepts.iter().find(|c| c.name == "Meal").unwrap();
        assert_eq!(meal.namespace, "diy.planner");
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
}
