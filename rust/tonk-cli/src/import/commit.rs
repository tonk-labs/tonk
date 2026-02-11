//! Import orchestration: validation, retraction, assertion, and atomic commit.
//!
//! Contains the async functions that take parsed concepts or rules, validate
//! them against the active space, build retract/assert instruction lists, and
//! commit them atomically.

use super::concept_parse::{ParsedAttribute, ParsedConcept};
use super::rule_parse::{ParsedRule, lower_rule};
use crate::rule::RuleDefinition;
use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::Value;
use dialog_query::claim::Attribute;
use std::collections::HashSet;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Concept import
// ---------------------------------------------------------------------------

/// Import concepts from a parsed YAML string into the active space.
///
/// All concepts are validated first, then committed atomically.
pub(super) async fn import_concepts(
    yaml_str: &str,
    file: &str,
    force: bool,
    json: bool,
) -> Result<()> {
    let concepts = super::concept_parse::parse_yaml(yaml_str)?;

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
                print_attribute_summary(attr);
            }
        }
    }

    Ok(())
}

/// Print a single attribute summary line (used in concept import output).
fn print_attribute_summary(attr: &ParsedAttribute) {
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

// ---------------------------------------------------------------------------
// Rule import
// ---------------------------------------------------------------------------

/// Import rules from a parsed YAML string into the active space.
///
/// All rules are parsed, lowered to `RuleDefinition`, validated against
/// the existing concept schemas, trial-compiled, then committed atomically.
pub(super) async fn import_rules(
    yaml_str: &str,
    file: &str,
    force: bool,
    json: bool,
) -> Result<()> {
    let rules = super::rule_parse::parse_rules_yaml(yaml_str)?;

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
