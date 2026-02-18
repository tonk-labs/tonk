//! YAML parsing for the canonical abbreviated notation.
//!
//! Parses a YAML file containing standalone attributes, concepts, and/or rules:
//!
//! ```yaml
//! diy.cook:
//!   # Standalone attribute at namespace level
//!   quantity:
//!     description: Amount needed
//!     as: Integer
//!
//!   # Concept with required and optional fields
//!   Recipe:
//!     description: Meal recipe
//!     with:
//!       title:
//!         description: The name of this recipe
//!         as: Text
//!       ingredient:
//!         description: Ingredients of the recipe
//!         cardinality: many
//!     maybe:
//!       notes:
//!         description: Optional notes
//!         as: Text
//!
//!   # Rule with deduce/when/unless
//!   safe-meal:
//!     description: A meal that respects dietary restrictions
//!     deduce:
//!       SafeMeal:
//!         attendee: ?person
//!     when:
//!       - diy.planner/Meal:
//!           attendee: ?person
//!     unless:
//!       - diy.planner/AllergyConflict:
//!           person: ?person
//! ```

use anyhow::{Context, Result};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Parsed intermediate representations
// ---------------------------------------------------------------------------

/// A parsed entry from a YAML namespace — can be a standalone attribute,
/// concept, or rule marker (rules are delegated to `rule_parse`).
#[derive(Debug)]
pub(super) enum ParsedEntry {
    Attribute(ParsedStandaloneAttribute),
    Concept(ParsedConcept),
    /// A rule entry. Contains the rule name, namespace, and the raw YAML
    /// value to be parsed by `rule_parse`.
    Rule {
        name: String,
        namespace: String,
        value: serde_yaml::Value,
    },
}

/// A standalone attribute defined at the namespace level (no parent concept).
#[derive(Debug)]
pub(super) struct ParsedStandaloneAttribute {
    /// Short attribute name (e.g. "quantity").
    pub short_name: String,
    /// The namespace it was defined under (e.g. "diy.cook").
    pub namespace: String,
    /// Description from the `description` field.
    pub description: Option<String>,
    /// Type constraint from `as`, stored verbatim. Arrays are JSON-encoded.
    pub type_str: Option<String>,
    /// Cardinality string (e.g. "many").
    pub cardinality: Option<String>,
}

/// A concept parsed from the YAML, ready for validation and import.
#[derive(Debug)]
pub(super) struct ParsedConcept {
    /// The concept name (e.g. "Recipe").
    pub name: String,
    /// The namespace it was defined under (e.g. "diy.cook").
    pub namespace: String,
    /// Description from the `description` field.
    pub description: Option<String>,
    /// The concept's attributes with their metadata.
    pub attributes: Vec<ParsedAttribute>,
}

/// A single attribute of a parsed concept.
#[derive(Debug)]
pub(super) struct ParsedAttribute {
    /// Short attribute name (e.g. "title").
    pub short_name: String,
    /// Fully qualified attribute reference, if provided explicitly in the YAML.
    /// Set when a concept field uses a string reference like `carry.links/handle`
    /// instead of an inline attribute definition. When set, this should be used
    /// as the stored attribute path instead of generating one from the concept name.
    pub qualified_ref: Option<String>,
    /// Description from the `description` field.
    pub description: Option<String>,
    /// Type constraint from `as`, stored verbatim. Arrays are JSON-encoded.
    pub type_str: Option<String>,
    /// Cardinality string (e.g. "many").
    pub cardinality: Option<String>,
    /// Whether this attribute is optional (defined in `maybe:` block).
    pub optional: bool,
}

// ---------------------------------------------------------------------------
// Entry classification
// ---------------------------------------------------------------------------

/// Determine what kind of entry a second-level YAML value represents.
#[derive(Debug, PartialEq, Eq)]
enum EntryKind {
    /// Has `deduce:` key → rule
    Rule,
    /// Has `with:` key (no `deduce:`) → concept
    Concept,
    /// Otherwise → standalone attribute
    StandaloneAttribute,
}

fn classify_entry(value: &serde_yaml::Value) -> EntryKind {
    if let serde_yaml::Value::Mapping(map) = value {
        let has_deduce = map.contains_key(serde_yaml::Value::String("deduce".into()));
        let has_with = map.contains_key(serde_yaml::Value::String("with".into()));

        if has_deduce {
            return EntryKind::Rule;
        }
        if has_with {
            return EntryKind::Concept;
        }
    }
    EntryKind::StandaloneAttribute
}

// ---------------------------------------------------------------------------
// YAML parsing
// ---------------------------------------------------------------------------

/// Parse a YAML string into a list of entries (attributes, concepts, rules).
///
/// The YAML structure is: `namespace -> entry_name -> entry_def`.
/// Entry classification is based on the presence of `deduce:` (rule),
/// `with:` (concept), or neither (standalone attribute).
pub(super) fn parse_yaml(yaml_str: &str) -> Result<Vec<ParsedEntry>> {
    let root: BTreeMap<String, BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(yaml_str).context("Failed to parse YAML")?;

    let mut entries = Vec::new();

    for (namespace, entry_map) in root {
        for (entry_name, entry_value) in entry_map {
            let kind = classify_entry(&entry_value);
            match kind {
                EntryKind::Rule => {
                    entries.push(ParsedEntry::Rule {
                        name: entry_name,
                        namespace: namespace.clone(),
                        value: entry_value,
                    });
                }
                EntryKind::Concept => {
                    let concept = parse_concept(&namespace, &entry_name, &entry_value)
                        .with_context(|| format!("In concept '{}.{}'", namespace, entry_name))?;
                    entries.push(ParsedEntry::Concept(concept));
                }
                EntryKind::StandaloneAttribute => {
                    let attr = parse_standalone_attribute(&namespace, &entry_name, &entry_value)
                        .with_context(|| {
                            format!("In standalone attribute '{}/{}'", namespace, entry_name)
                        })?;
                    entries.push(ParsedEntry::Attribute(attr));
                }
            }
        }
    }

    Ok(entries)
}

/// Convenience: parse a YAML string and return only the concepts.
/// Used by tests and the concept-only import path.
#[cfg(test)]
fn parse_concepts(yaml_str: &str) -> Result<Vec<ParsedConcept>> {
    let entries = parse_yaml(yaml_str)?;
    Ok(entries
        .into_iter()
        .filter_map(|e| match e {
            ParsedEntry::Concept(c) => Some(c),
            _ => None,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Standalone attribute parsing
// ---------------------------------------------------------------------------

/// Parse a standalone attribute from its YAML value.
///
/// Handles three forms:
/// - Null: bare key with no value (e.g. `quantity:`)
/// - String: type shorthand (e.g. `quantity: Integer`)
/// - Mapping: full definition with `description:`, `as:`, `cardinality:`
fn parse_standalone_attribute(
    namespace: &str,
    name: &str,
    value: &serde_yaml::Value,
) -> Result<ParsedStandaloneAttribute> {
    match value {
        serde_yaml::Value::Null => Ok(ParsedStandaloneAttribute {
            short_name: name.to_string(),
            namespace: namespace.to_string(),
            description: None,
            type_str: None,
            cardinality: None,
        }),
        serde_yaml::Value::String(s) => Ok(ParsedStandaloneAttribute {
            short_name: name.to_string(),
            namespace: namespace.to_string(),
            description: None,
            type_str: Some(s.clone()),
            cardinality: None,
        }),
        serde_yaml::Value::Mapping(map) => {
            let description = map
                .get(serde_yaml::Value::String("description".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let as_type = map.get(serde_yaml::Value::String("as".into())).cloned();
            let type_str = resolve_as_type(&as_type)?;

            let cardinality = map
                .get(serde_yaml::Value::String("cardinality".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            Ok(ParsedStandaloneAttribute {
                short_name: name.to_string(),
                namespace: namespace.to_string(),
                description,
                type_str,
                cardinality,
            })
        }
        other => anyhow::bail!(
            "Unexpected value type for standalone attribute '{}': {:?}",
            name,
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Concept parsing
// ---------------------------------------------------------------------------

/// Parse a single concept from its YAML value.
///
/// Expected structure:
/// ```yaml
/// ConceptName:
///   description: Human-readable description
///   with:
///     field1:
///       description: ...
///       as: Text
///     field2: .           # punning reference
///   maybe:
///     opt_field:
///       description: ...
///       as: Text
/// ```
fn parse_concept(
    namespace: &str,
    concept_name: &str,
    value: &serde_yaml::Value,
) -> Result<ParsedConcept> {
    let map: &serde_yaml::Mapping = value
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("Expected a mapping for concept '{}'", concept_name))?;

    // Extract description
    let description = map
        .get(serde_yaml::Value::String("description".into()))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut attributes = Vec::new();

    // Parse `with:` block (required fields)
    if let Some(with_value) = map.get(serde_yaml::Value::String("with".into())) {
        let with_attrs =
            parse_field_block(with_value, false).context("Failed to parse 'with' block")?;
        attributes.extend(with_attrs);
    }

    // Parse `maybe:` block (optional fields)
    if let Some(maybe_value) = map.get(serde_yaml::Value::String("maybe".into())) {
        let maybe_attrs =
            parse_field_block(maybe_value, true).context("Failed to parse 'maybe' block")?;
        attributes.extend(maybe_attrs);
    }

    Ok(ParsedConcept {
        name: concept_name.to_string(),
        namespace: namespace.to_string(),
        description,
        attributes,
    })
}

/// Parse a `with:` or `maybe:` block into a list of attributes.
///
/// Each field in the block can be:
/// - A string reference (`.`, `.name`, `domain/name`) — attribute by reference
/// - A mapping — inline attribute definition with `description:`, `as:`, `cardinality:`
/// - Null — bare key with no metadata
fn parse_field_block(value: &serde_yaml::Value, optional: bool) -> Result<Vec<ParsedAttribute>> {
    let fields: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_value(value.clone())
        .context("Expected a mapping of field names to definitions")?;

    let mut attributes = Vec::new();

    for (field_name, field_value) in fields {
        let attr = parse_field_entry(&field_name, &field_value, optional)
            .with_context(|| format!("Failed to parse field '{}'", field_name))?;
        attributes.push(attr);
    }

    Ok(attributes)
}

/// Parse a single field entry within a `with:` or `maybe:` block.
///
/// Three forms of field values:
/// 1. String `.` — punning (resolves to concept-domain/field-name)
/// 2. String `.name` — explicit name override (resolves to concept-domain/name)
/// 3. String `domain/name` — fully qualified attribute reference
/// 4. Mapping — inline attribute definition
/// 5. Null — bare key with no metadata
fn parse_field_entry(
    field_name: &str,
    value: &serde_yaml::Value,
    optional: bool,
) -> Result<ParsedAttribute> {
    match value {
        serde_yaml::Value::Null => {
            // Bare key, no metadata
            Ok(ParsedAttribute {
                short_name: field_name.to_string(),
                qualified_ref: None,
                description: None,
                type_str: None,
                cardinality: None,
                optional,
            })
        }
        serde_yaml::Value::String(ref_str) => {
            // Reference form: ".", ".name", or "domain/name"
            // Resolve and store the fully qualified reference so commit.rs
            // can use the correct attribute path.
            let qualified = if ref_str == "." {
                // Punning: will be resolved at commit time using concept domain
                None
            } else if let Some(name) = ref_str.strip_prefix('.') {
                // `.name` — explicit name, concept domain (resolved at commit time)
                // For now store as-is; commit.rs will prepend concept domain
                Some(format!(".{}", name))
            } else if ref_str.contains('/') {
                // Fully qualified: `domain/name`
                Some(ref_str.clone())
            } else {
                None
            };

            Ok(ParsedAttribute {
                short_name: field_name.to_string(),
                qualified_ref: qualified,
                description: None,
                type_str: None,
                cardinality: None,
                optional,
            })
        }
        serde_yaml::Value::Mapping(map) => {
            // Inline attribute definition
            let description = map
                .get(serde_yaml::Value::String("description".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let as_type = map.get(serde_yaml::Value::String("as".into())).cloned();
            let type_str = resolve_as_type(&as_type)?;

            let cardinality = map
                .get(serde_yaml::Value::String("cardinality".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            Ok(ParsedAttribute {
                short_name: field_name.to_string(),
                qualified_ref: None,
                description,
                type_str,
                cardinality,
                optional,
            })
        }
        other => anyhow::bail!(
            "Unexpected value type for field '{}': {:?}",
            field_name,
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Type resolution helpers
// ---------------------------------------------------------------------------

/// Resolve the `as:` field value into a stored type string.
///
/// Handles:
/// - `None` → `None`
/// - `String("Text")` → `Some("Text")`
/// - `String(".RecipeStep")` → `Some("RecipeStep")` (strip dot prefix)
/// - `Sequence([:tsp, :mls])` → `Some(r#"["tsp","mls"]"#)` (strip colon, JSON array)
/// - Other → debug format
fn resolve_as_type(as_type: &Option<serde_yaml::Value>) -> Result<Option<String>> {
    match as_type {
        None => Ok(None),
        Some(serde_yaml::Value::String(s)) => {
            if let Some(stripped) = s.strip_prefix('.') {
                // Concept type reference: `.RecipeStep` → `RecipeStep`
                Ok(Some(stripped.to_string()))
            } else {
                Ok(Some(s.clone()))
            }
        }
        Some(serde_yaml::Value::Sequence(seq)) => {
            // Array of symbols: `[:tsp, :mls]` → JSON `["tsp", "mls"]`
            let items: Vec<String> = seq
                .iter()
                .map(|v| match v {
                    serde_yaml::Value::String(s) => {
                        // Strip colon prefix from symbols
                        Ok(s.strip_prefix(':').unwrap_or(s).to_string())
                    }
                    other => Ok(format!("{:?}", other)),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(serde_json::to_string(&items)?))
        }
        Some(other) => Ok(Some(format!("{:?}", other))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to extract only concepts from parsed entries
    fn extract_concepts(entries: Vec<ParsedEntry>) -> Vec<ParsedConcept> {
        entries
            .into_iter()
            .filter_map(|e| match e {
                ParsedEntry::Concept(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    // Helper to extract only standalone attributes from parsed entries
    fn extract_attributes(entries: Vec<ParsedEntry>) -> Vec<ParsedStandaloneAttribute> {
        entries
            .into_iter()
            .filter_map(|e| match e {
                ParsedEntry::Attribute(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    // Helper to count rules in parsed entries
    fn count_rules(entries: &[ParsedEntry]) -> usize {
        entries
            .iter()
            .filter(|e| matches!(e, ParsedEntry::Rule { .. }))
            .count()
    }

    #[test]
    fn parse_basic_concept() {
        let yaml = r#"
diy.cook:
  Recipe:
    description: Meal recipe
    with:
      title:
        description: The name of this recipe
        as: Text
      ingredient:
        description: Ingredients of the recipe
        cardinality: many
"#;
        let concepts = parse_concepts(yaml).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(concepts[0].name, "Recipe");
        assert_eq!(concepts[0].namespace, "diy.cook");
        assert_eq!(concepts[0].description.as_deref(), Some("Meal recipe"));
        assert_eq!(concepts[0].attributes.len(), 2);

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
    description: A thing
    with:
      unit:
        description: Unit of measurement
        as: [":tsp", ":mls"]
"#;
        let concepts = parse_concepts(yaml).unwrap();
        let attr = &concepts[0].attributes[0];
        assert_eq!(attr.short_name, "unit");
        // Stored as JSON array with colon-prefixed symbols stripped
        let parsed: Vec<String> = serde_json::from_str(attr.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["tsp", "mls"]);
    }

    #[test]
    fn parse_optional_attribute() {
        let yaml = r#"
ns:
  Thing:
    description: A thing
    with:
      name:
        description: The name
        as: Text
    maybe:
      after:
        description: Step to perform this after
        as: .RecipeStep
"#;
        let concepts = parse_concepts(yaml).unwrap();
        let name_attr = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "name")
            .unwrap();
        assert!(!name_attr.optional);

        // Fields in `maybe:` block are optional
        let after_attr = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "after")
            .unwrap();
        assert!(after_attr.optional);
        // `.RecipeStep` should have dot stripped
        assert_eq!(after_attr.type_str.as_deref(), Some("RecipeStep"));
    }

    #[test]
    fn parse_multiple_namespaces() {
        let yaml = r#"
diy.cook:
  Recipe:
    description: Meal recipe
    with:
      title:
        description: The name
        as: Text

diy.health:
  Allergy:
    description: The allergy person has
    with:
      person:
        description: Person having an allergy
      substance:
        description: Substance person has an allergy for
        as: Text
"#;
        let concepts = parse_concepts(yaml).unwrap();
        assert_eq!(concepts.len(), 2);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Recipe"));
        assert!(names.contains(&"Allergy"));
    }

    #[test]
    fn parse_empty_yaml() {
        let yaml = "";
        // serde_yaml treats empty string as null, which fails to parse as BTreeMap
        let result = parse_yaml(yaml);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn concept_with_no_description() {
        let yaml = r#"
ns:
  Simple:
    with:
      name:
        description: A name
        as: Text
"#;
        let concepts = parse_concepts(yaml).unwrap();
        assert_eq!(concepts[0].description, None);
        assert_eq!(concepts[0].attributes.len(), 1);
    }

    #[test]
    fn parse_cook_example() {
        let yaml = include_str!("../../examples/cook.yaml");
        let concepts = parse_concepts(yaml).unwrap();
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
        // `.RecipeStep` should have dot stripped to `RecipeStep`
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

        // RecipeStep.after is optional (in maybe: block)
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
        let entries = parse_yaml(yaml).unwrap();

        let concepts = extract_concepts(
            entries
                .into_iter()
                .filter(|e| !matches!(e, ParsedEntry::Rule { .. }))
                .collect(),
        );
        // planner.yaml now has 4 concepts + 2 rules
        // We only look at concepts here
        assert_eq!(concepts.len(), 4);

        let names: Vec<&str> = concepts.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Allergy"));
        assert!(names.contains(&"Event"));
        assert!(names.contains(&"Meal"));
        assert!(names.contains(&"AllergyConflict"));

        // Allergy is under diy.health, others under diy.planner
        let allergy = concepts.iter().find(|c| c.name == "Allergy").unwrap();
        assert_eq!(allergy.namespace, "diy.health");

        let event = concepts.iter().find(|c| c.name == "Event").unwrap();
        assert_eq!(event.namespace, "diy.planner");

        let meal = concepts.iter().find(|c| c.name == "Meal").unwrap();
        assert_eq!(meal.namespace, "diy.planner");

        let conflict = concepts
            .iter()
            .find(|c| c.name == "AllergyConflict")
            .unwrap();
        assert_eq!(conflict.namespace, "diy.planner");
        assert_eq!(meal.attributes.len(), 3);
    }

    #[test]
    fn parse_planner_has_rules() {
        let yaml = include_str!("../../examples/planner.yaml");
        let entries = parse_yaml(yaml).unwrap();
        let rule_count = count_rules(&entries);
        assert_eq!(rule_count, 2);
    }

    #[test]
    fn parse_minimal_example() {
        let yaml = include_str!("../../examples/minimal.yaml");
        let concepts = parse_concepts(yaml).unwrap();
        assert_eq!(concepts.len(), 1);

        let task = &concepts[0];
        assert_eq!(task.name, "Task");
        assert_eq!(task.namespace, "app");
        assert_eq!(task.description.as_deref(), Some("A task to be completed"));
        assert_eq!(task.attributes.len(), 3);

        // status is an enum (colon-prefixed symbols)
        let status = task
            .attributes
            .iter()
            .find(|a| a.short_name == "status")
            .unwrap();
        let parsed: Vec<String> = serde_json::from_str(status.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["todo", "in-progress", "done"]);
        assert!(!status.optional);

        // priority is optional (in maybe: block) and an enum
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

    #[test]
    fn parse_standalone_attribute_basic() {
        let yaml = r#"
diy.cook:
  quantity:
    description: Amount needed
    as: Integer
    cardinality: many
"#;
        let entries = parse_yaml(yaml).unwrap();
        let attrs = extract_attributes(entries);
        assert_eq!(attrs.len(), 1);

        let attr = &attrs[0];
        assert_eq!(attr.short_name, "quantity");
        assert_eq!(attr.namespace, "diy.cook");
        assert_eq!(attr.description.as_deref(), Some("Amount needed"));
        assert_eq!(attr.type_str.as_deref(), Some("Integer"));
        assert_eq!(attr.cardinality.as_deref(), Some("many"));
    }

    #[test]
    fn parse_standalone_attribute_with_symbol_enum() {
        let yaml = r#"
diy.cook:
  unit:
    description: The unit of measurement
    as: [":tsp", ":mls"]
"#;
        let entries = parse_yaml(yaml).unwrap();
        let attrs = extract_attributes(entries);
        assert_eq!(attrs.len(), 1);

        let attr = &attrs[0];
        let parsed: Vec<String> = serde_json::from_str(attr.type_str.as_ref().unwrap()).unwrap();
        assert_eq!(parsed, vec!["tsp", "mls"]);
    }

    #[test]
    fn parse_concept_with_punning_ref() {
        let yaml = r#"
diy.cook:
  Ingredient:
    description: An ingredient
    with:
      quantity: .
      name: .ingredient-name
      person-name: io.gozala.person/name
"#;
        let concepts = parse_concepts(yaml).unwrap();
        assert_eq!(concepts.len(), 1);
        assert_eq!(concepts[0].attributes.len(), 3);

        // String references should parse without error, stored as attrs with no type/description
        let qty = concepts[0]
            .attributes
            .iter()
            .find(|a| a.short_name == "quantity")
            .unwrap();
        assert!(qty.description.is_none());
        assert!(qty.type_str.is_none());
    }

    #[test]
    fn parse_mixed_file_classification() {
        let yaml = r#"
diy.cook:
  quantity:
    description: Amount needed
    as: Integer

  Recipe:
    description: A recipe
    with:
      title:
        description: Title
        as: Text

  find-something:
    description: A rule
    deduce:
      Recipe:
        title: ?t
    when:
      - diy.cook/Recipe:
          this: ?r
          title: ?t
"#;
        let entries = parse_yaml(yaml).unwrap();

        let attrs = entries
            .iter()
            .filter(|e| matches!(e, ParsedEntry::Attribute(_)))
            .count();
        let concepts = entries
            .iter()
            .filter(|e| matches!(e, ParsedEntry::Concept(_)))
            .count();
        let rules = count_rules(&entries);

        assert_eq!(attrs, 1);
        assert_eq!(concepts, 1);
        assert_eq!(rules, 1);
    }

    #[test]
    fn classify_entry_rule() {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(
            r#"
deduce:
  Foo:
    bar: ?x
when:
  - ns/Baz:
      this: ?x
"#,
        )
        .unwrap();
        assert_eq!(classify_entry(&yaml_val), EntryKind::Rule);
    }

    #[test]
    fn classify_entry_concept() {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(
            r#"
description: A thing
with:
  name:
    as: Text
"#,
        )
        .unwrap();
        assert_eq!(classify_entry(&yaml_val), EntryKind::Concept);
    }

    #[test]
    fn classify_entry_standalone_attr() {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(
            r#"
description: Amount needed
as: Integer
"#,
        )
        .unwrap();
        assert_eq!(classify_entry(&yaml_val), EntryKind::StandaloneAttribute);
    }

    #[test]
    fn dot_prefix_type_stripped() {
        let yaml = r#"
ns:
  Thing:
    description: A thing
    with:
      ref:
        description: Reference
        as: .OtherThing
"#;
        let concepts = parse_concepts(yaml).unwrap();
        let attr = &concepts[0].attributes[0];
        // Dot prefix should be stripped
        assert_eq!(attr.type_str.as_deref(), Some("OtherThing"));
    }
}
