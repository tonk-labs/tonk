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

mod commit;
mod concept_parse;
mod rule_parse;

use anyhow::{Context, Result};
use std::collections::BTreeMap;

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
        YamlFileType::Concepts => commit::import_concepts(&yaml_str, &file, force, json).await,
        YamlFileType::Rules => commit::import_rules(&yaml_str, &file, force, json).await,
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
}
