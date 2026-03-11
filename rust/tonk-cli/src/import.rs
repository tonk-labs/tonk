//! YAML import for concepts, standalone attributes, and rules.
//!
//! Parses a YAML file using the canonical abbreviated notation and auto-detects
//! whether it contains concept definitions, rule definitions, or a mix of both.
//! Imports are committed atomically into the active space.
//!
//! # Concept YAML format
//!
//! ```yaml
//! diy.cook:
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
//! ```
//!
//! # Standalone attribute format
//!
//! ```yaml
//! diy.cook:
//!   quantity:
//!     description: Amount needed
//!     as: Integer
//!     cardinality: many
//! ```
//!
//! # Rule YAML format
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
//!     unless:
//!       - diy.planner/AllergyConflict:
//!           person: ?person
//!           recipe: ?recipe
//! ```
//!
//! # Mixed files
//!
//! A single file can contain standalone attributes, concepts, and rules
//! all under one namespace key. The parser handles this — it does not
//! require separate files for concepts vs rules.

mod commit;
mod concept_parse;
mod rule_parse;

use crate::schema::SpaceContext;
use anyhow::{Context, Result};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// YAML file type detection
// ---------------------------------------------------------------------------

/// The type of content in a YAML import file.
#[derive(Debug, PartialEq, Eq)]
enum YamlFileType {
    /// Only concepts and/or standalone attributes (no rules).
    Concepts,
    /// Only rules (no concepts or standalone attributes).
    Rules,
    /// A mix of concepts, standalone attributes, and rules.
    Mixed,
}

/// Detect what kind of entries a YAML string contains.
///
/// Inspects the second-level values:
/// - A value with `deduce` + `when` keys → rule
/// - A value with `with` key (no `deduce`) → concept
/// - Otherwise → standalone attribute
///
/// If both rules and concepts/attributes are found, returns `Mixed`.
fn detect_yaml_type(yaml_str: &str) -> Result<YamlFileType> {
    let root: BTreeMap<String, BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(yaml_str).context("Failed to parse YAML")?;

    let mut has_rules = false;
    let mut has_concepts_or_attrs = false;

    for entries in root.values() {
        for value in entries.values() {
            if let serde_yaml::Value::Mapping(map) = value {
                let has_deduce = map.contains_key(serde_yaml::Value::String("deduce".into()));
                let has_with = map.contains_key(serde_yaml::Value::String("with".into()));

                if has_deduce {
                    has_rules = true;
                } else if has_with {
                    has_concepts_or_attrs = true;
                } else {
                    // Standalone attribute or other non-rule, non-concept entry
                    has_concepts_or_attrs = true;
                }
            } else {
                // Scalar or null value → standalone attribute (e.g. `quantity: Integer`)
                has_concepts_or_attrs = true;
            }
        }
    }

    if has_rules && has_concepts_or_attrs {
        Ok(YamlFileType::Mixed)
    } else if has_rules {
        Ok(YamlFileType::Rules)
    } else {
        Ok(YamlFileType::Concepts)
    }
}

// ---------------------------------------------------------------------------
// Import command (entry point)
// ---------------------------------------------------------------------------

/// Import concepts, standalone attributes, and/or rules from a YAML file
/// into the active space.
///
/// Auto-detects the file type based on the YAML structure, then dispatches
/// to the appropriate import handler.
///
/// All entries in the file are validated first, then committed atomically.
/// If `force` is true, existing concepts/rules are overwritten (retracted
/// then re-created); otherwise any collision fails the entire import.
pub async fn import(ctx: &SpaceContext, file: String, force: bool, json: bool) -> Result<()> {
    let yaml_str =
        std::fs::read_to_string(&file).context(format!("Failed to read file: {}", file))?;

    match detect_yaml_type(&yaml_str)? {
        YamlFileType::Concepts => commit::import_concepts(ctx, &yaml_str, &file, force, json).await,
        YamlFileType::Rules => commit::import_rules(ctx, &yaml_str, &file, force, json).await,
        YamlFileType::Mixed => commit::import_mixed(ctx, &yaml_str, &file, force, json).await,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_concept_file() {
        let yaml = r#"
diy.cook:
  Recipe:
    description: Meal recipe
    with:
      title:
        description: The name
        as: Text
"#;
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Concepts);
    }

    #[test]
    fn detect_rule_file() {
        let yaml = r#"
user.rules:
  my-rule:
    description: A rule
    deduce:
      ns/Meal:
        attendee: ?person
    when:
      - ns/Recipe:
          this: ?recipe
          title: _
"#;
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Rules);
    }

    #[test]
    fn detect_mixed_file() {
        let yaml = r#"
diy.cook:
  Recipe:
    description: Meal recipe
    with:
      title:
        description: The name
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
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Mixed);
    }

    #[test]
    fn detect_standalone_attribute_file() {
        let yaml = r#"
diy.cook:
  quantity:
    description: Amount needed
    as: Integer
"#;
        assert_eq!(detect_yaml_type(yaml).unwrap(), YamlFileType::Concepts);
    }
}
