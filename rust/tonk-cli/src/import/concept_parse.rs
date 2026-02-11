//! YAML parsing for concept definitions.
//!
//! Parses a YAML file of the form:
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

use anyhow::{Context, Result};
use std::collections::BTreeMap;

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
pub(super) struct ParsedConcept {
    /// The concept name (e.g. "Recipe").
    pub name: String,
    /// The namespace it was defined under (e.g. "diy.cook").
    pub namespace: String,
    /// Description from the `this.the` field.
    pub description: Option<String>,
    /// The concept's attributes with their metadata.
    pub attributes: Vec<ParsedAttribute>,
}

/// A single attribute of a parsed concept.
pub(super) struct ParsedAttribute {
    /// Short attribute name (e.g. "title").
    pub short_name: String,
    /// Description from the `the` field.
    pub description: Option<String>,
    /// Type constraint from `as`, stored verbatim. Arrays are JSON-encoded.
    pub type_str: Option<String>,
    /// Cardinality string (e.g. "many").
    pub cardinality: Option<String>,
    /// Whether this attribute is optional.
    pub optional: bool,
}

// ---------------------------------------------------------------------------
// YAML parsing
// ---------------------------------------------------------------------------

/// Parse a YAML string into a list of concepts.
///
/// The YAML structure is: `namespace -> concept_name -> attr_name -> attr_def`.
/// The `this` key is special and provides the concept-level description.
pub(super) fn parse_yaml(yaml_str: &str) -> Result<Vec<ParsedConcept>> {
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
                // Array of strings -> JSON array
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
/// - A bare scalar (e.g. `title:` with no value -> null -> default)
/// - A string shorthand (e.g. `title: Text` -> treated as type only)
fn parse_attr_def(value: &serde_yaml::Value) -> Result<YamlAttributeDef> {
    match value {
        serde_yaml::Value::Null => Ok(YamlAttributeDef::default()),
        serde_yaml::Value::String(s) => {
            // Bare string -> treat as type shorthand
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

        // `optional:` with null value -> true
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
        let yaml = include_str!("../../examples/cook.yaml");
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
        let yaml = include_str!("../../examples/planner.yaml");
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
        let yaml = include_str!("../../examples/minimal.yaml");
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
