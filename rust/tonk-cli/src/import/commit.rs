//! Import orchestration: validation, retraction, assertion, and atomic commit.
//!
//! Contains the async functions that take parsed concepts or rules, validate
//! them against the active space, build retract/assert instruction lists, and
//! commit them atomically.
//!
//! Uses raw Branch + Instruction (not Session/Transaction) because imports
//! write multi-valued attributes: multiple `concept/attribute` entries per
//! concept and multiple attribute metadata entries. Transaction's
//! `HashMap<Entity, HashMap<Attribute, Change>>` deduplicates by
//! `(entity, attribute)`, so only the last value per pair would survive.

use super::concept_parse::{
    ParsedAttribute, ParsedConcept, ParsedEntry, ParsedStandaloneAttribute,
};
use super::rule_parse::{ParsedRule, lower_rule};
use crate::rule::RuleDefinition;
use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::{Entity, Value};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Concept import
// ---------------------------------------------------------------------------

/// Import concepts from a parsed YAML string into the active space.
///
/// All concepts are validated first, then committed atomically.
pub(super) async fn import_concepts(
    ctx: &SpaceContext,
    yaml_str: &str,
    file: &str,
    force: bool,
    json: bool,
) -> Result<()> {
    let entries = super::concept_parse::parse_yaml(yaml_str)?;

    // Separate concepts and standalone attributes
    let mut concepts = Vec::new();
    let mut standalone_attrs = Vec::new();

    for entry in entries {
        match entry {
            ParsedEntry::Concept(c) => concepts.push(c),
            ParsedEntry::Attribute(a) => standalone_attrs.push(a),
            ParsedEntry::Rule {
                name, namespace, ..
            } => {
                anyhow::bail!(
                    "Unexpected rule '{}' in namespace '{}'. \
                     Use a mixed-format file or a separate rules file.",
                    name,
                    namespace
                );
            }
        }
    }

    if concepts.is_empty() && standalone_attrs.is_empty() {
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

        if concept.description.is_none() {
            anyhow::bail!(
                "Concept '{}' is missing a description. Add a 'this' section with \
                 'the: <description>' to the concept definition.",
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

    // Validate standalone attributes
    for attr in &standalone_attrs {
        validate_safe_name(&attr.short_name, "Attribute")?;
    }

    let mut branch = open_branch(ctx).await?;

    let mut retract_instructions: Vec<Instruction> = Vec::new();

    // Track old entity IDs for provenance linking on force-reimport
    let mut old_entities: HashMap<String, Entity> = HashMap::new();

    for (cname, _concept) in &validated {
        if let Some(old_entity) =
            retract_concept_if_exists(&branch, cname, force, &mut retract_instructions).await?
        {
            // Remember the old entity for provenance linking
            old_entities.insert(cname.to_string(), old_entity);
        }
    }

    for attr in &standalone_attrs {
        retract_standalone_attr_if_exists(
            &branch,
            &ctx.space_did,
            attr,
            force,
            &mut retract_instructions,
        )
        .await?;
    }

    // --- Build assert instructions ---

    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (cname, concept) in &validated {
        build_concept_assertions(
            cname,
            concept,
            &mut assert_instructions,
            &mut import_summary,
        )?;

        // If force-reimporting and entity identity changed, assert provenance link
        let qualified_attrs: Vec<String> = concept
            .attributes
            .iter()
            .map(|a| qualify_attribute(&concept.namespace, &a.short_name))
            .collect::<Result<Vec<_>>>()?;
        let empty_cardinalities = std::collections::HashMap::new();
        let entity = concept_entity_from_attrs(&qualified_attrs, &empty_cardinalities)?;
        if let Some(old_entity) = old_entities.get(&cname.to_string())
            && *old_entity != entity
        {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: concept_prior_selector(),
                of: entity.clone(),
                is: Value::String(old_entity.to_string()),
                cause: None,
            }));
        }
    }

    // Build standalone attribute assertions
    for attr in &standalone_attrs {
        build_standalone_attr_assertions(&ctx.space_did, attr, &mut assert_instructions)?;
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
        let total = concepts.len() + standalone_attrs.len();
        println!("Imported {} item(s) from '{}':\n", total, file);
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
        for attr in &standalone_attrs {
            println!(
                "  {}/{} (standalone attribute)",
                attr.namespace, attr.short_name
            );
        }
    }

    Ok(())
}

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
    ctx: &SpaceContext,
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

        if rule.description.is_none() {
            anyhow::bail!(
                "Rule '{}' is missing a description. Add a 'description' field \
                 to the rule definition.",
                rule.name
            );
        }

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

    let mut branch = open_branch(ctx).await?;

    let mut retract_instructions: Vec<Instruction> = Vec::new();

    for (parsed, definition) in &lowered {
        retract_rule_if_exists(&branch, parsed, force, &mut retract_instructions).await?;

        validate_rule_against_space(&branch, parsed, definition).await?;
    }

    // --- Build assert instructions ---

    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (parsed, definition) in &lowered {
        build_rule_assertions(
            &ctx.space_did,
            parsed,
            definition,
            &mut assert_instructions,
            &mut import_summary,
        )?;
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
            let desc_str = parsed
                .description
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!(
                "  {} [{}] -> {}{}",
                parsed.name, parsed.namespace, definition.conclusion.concept, desc_str
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
// Mixed import (concepts + rules + standalone attributes in one file)
// ---------------------------------------------------------------------------

/// Import a mixed YAML file containing concepts, standalone attributes,
/// and rules into the active space.
///
/// Concepts, standalone attributes, and rules are validated first, then
/// committed together atomically. Rules are validated against the effective
/// schema (including concepts defined in the same file).
pub(super) async fn import_mixed(
    ctx: &SpaceContext,
    yaml_str: &str,
    file: &str,
    force: bool,
    json: bool,
) -> Result<()> {
    let entries = super::concept_parse::parse_yaml(yaml_str)?;

    // Separate into concepts, standalone attributes, and rules
    let mut concepts = Vec::new();
    let mut standalone_attrs = Vec::new();
    let mut rule_entries: Vec<(String, String, serde_yaml::Value)> = Vec::new();

    for entry in entries {
        match entry {
            ParsedEntry::Concept(c) => concepts.push(c),
            ParsedEntry::Attribute(a) => standalone_attrs.push(a),
            ParsedEntry::Rule {
                name,
                namespace,
                value,
            } => {
                rule_entries.push((name, namespace, value));
            }
        }
    }

    // Parse the raw rule YAML values into ParsedRules
    let mut parsed_rules = Vec::new();
    for (name, namespace, value) in &rule_entries {
        let parsed = super::rule_parse::parse_rule(namespace, name, value)
            .with_context(|| format!("In rule '{}/{}'", namespace, name))?;
        parsed_rules.push(parsed);
    }

    if concepts.is_empty() && standalone_attrs.is_empty() && parsed_rules.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"ok": true, "imported": []}))?
            );
        } else {
            println!("No entries found in YAML file.");
        }
        return Ok(());
    }

    // --- Validate concepts ---

    let mut validated_concepts: Vec<(ConceptName, &ParsedConcept)> = Vec::new();
    let mut seen_concept_names: HashSet<String> = HashSet::new();

    for concept in &concepts {
        let cname = ConceptName::new(&concept.name)?;

        let lower = cname.to_lowercase();
        if !seen_concept_names.insert(lower.clone()) {
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

        validated_concepts.push((cname, concept));
    }

    // Validate standalone attributes
    for attr in &standalone_attrs {
        validate_safe_name(&attr.short_name, "Attribute")?;
    }

    // --- Lower rules (structural validation only, no space validation yet) ---

    let mut lowered_rules: Vec<(&ParsedRule, RuleDefinition)> = Vec::new();
    let mut seen_rule_names: HashSet<String> = HashSet::new();

    for rule in &parsed_rules {
        validate_safe_name(&rule.name, "Rule")?;

        let lower = rule.name.to_lowercase();
        if !seen_rule_names.insert(lower) {
            anyhow::bail!(
                "Duplicate rule name '{}' in YAML file. \
                 Rule names must be unique (case-insensitive).",
                rule.name
            );
        }

        let definition =
            lower_rule(rule).with_context(|| format!("Failed to lower rule '{}'", rule.name))?;

        lowered_rules.push((rule, definition));
    }

    // --- Build schema overrides for concepts defined in this file ---

    #[allow(clippy::type_complexity)]
    let mut concept_overrides: std::collections::HashMap<
        String,
        (
            ConceptName,
            Vec<String>,
            std::collections::HashMap<String, dialog_query::Cardinality>,
            String, // namespace
        ),
    > = std::collections::HashMap::new();
    for (cname, concept) in &validated_concepts {
        let attrs = build_concept_attr_list(cname, concept)?;
        let cardinalities = build_concept_cardinalities(cname, concept)?;
        concept_overrides.insert(
            cname.to_lowercase(),
            (
                cname.clone(),
                attrs,
                cardinalities,
                concept.namespace.clone(),
            ),
        );
    }

    // --- Validate rules against the effective schema ---

    let mut branch = open_branch(ctx).await?;

    for (parsed, definition) in &lowered_rules {
        let conclusion_concept = ConceptName::new(&definition.conclusion.concept)?;
        let key = conclusion_concept.to_lowercase();
        let (concept_name, concept_attrs, cardinalities, concept_ns) =
            if let Some((name, attrs, cards, ns)) = concept_overrides.get(&key) {
                (name.clone(), attrs.clone(), cards.clone(), ns.clone())
            } else {
                let concept_ent = lookup_concept_by_name(&branch, &conclusion_concept)
                    .await?
                    .context(format!(
                        "Conclusion concept '{}' for rule '{}' not found. Define it first.",
                        definition.conclusion.concept, parsed.name
                    ))?;
                let concept_name = ConceptName::from_stored(
                    fetch_string(&branch, &concept_ent, concept_name_selector())
                        .await?
                        .unwrap_or_else(|| conclusion_concept.to_string()),
                );
                let concept_attrs =
                    fetch_string_values(&branch, &concept_ent, concept_attribute_selector())
                        .await?;
                let concept_ns = fetch_string(&branch, &concept_ent, concept_namespace_selector())
                    .await?
                    .unwrap_or_else(|| ctx.space_name.clone());
                let cardinalities = fetch_attribute_cardinalities(&branch, &concept_attrs).await?;
                (concept_name, concept_attrs, cardinalities, concept_ns)
            };

        validate_rule_against_schema(
            parsed,
            definition,
            &concept_name,
            &concept_attrs,
            &cardinalities,
            &concept_ns,
        )?;
    }

    // --- Build all instructions and commit atomically ---

    let mut retract_instructions: Vec<Instruction> = Vec::new();
    let mut assert_instructions: Vec<Instruction> = Vec::new();
    let mut import_summary: Vec<serde_json::Value> = Vec::new();

    for (cname, _concept) in &validated_concepts {
        retract_concept_if_exists(&branch, cname, force, &mut retract_instructions).await?;
    }

    for attr in &standalone_attrs {
        retract_standalone_attr_if_exists(
            &branch,
            &ctx.space_did,
            attr,
            force,
            &mut retract_instructions,
        )
        .await?;
    }

    for (parsed, _definition) in &lowered_rules {
        retract_rule_if_exists(&branch, parsed, force, &mut retract_instructions).await?;
    }

    for (cname, concept) in &validated_concepts {
        build_concept_assertions(
            cname,
            concept,
            &mut assert_instructions,
            &mut import_summary,
        )?;
    }

    for attr in &standalone_attrs {
        build_standalone_attr_assertions(&ctx.space_did, attr, &mut assert_instructions)?;
    }

    for (parsed, definition) in &lowered_rules {
        build_rule_assertions(
            &ctx.space_did,
            parsed,
            definition,
            &mut assert_instructions,
            &mut import_summary,
        )?;
    }

    let mut all_instructions = retract_instructions;
    all_instructions.extend(assert_instructions);

    branch
        .commit(futures_util::stream::iter(all_instructions))
        .await?;

    // --- Output ---

    if json {
        let output = serde_json::json!({
            "ok": true,
            "type": "mixed",
            "imported": import_summary,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        let total = concepts.len() + standalone_attrs.len() + parsed_rules.len();
        println!(
            "Imported {} item(s) from '{}' ({} concept(s), {} attribute(s), {} rule(s)):\n",
            total,
            file,
            concepts.len(),
            standalone_attrs.len(),
            parsed_rules.len()
        );
        for (cname, concept) in &validated_concepts {
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
        for attr in &standalone_attrs {
            println!(
                "  {}/{} (standalone attribute)",
                attr.namespace, attr.short_name
            );
        }
        for (parsed, definition) in &lowered_rules {
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
// Shared helpers: concept retraction and assertion
// ---------------------------------------------------------------------------

/// Check if a concept already exists and build retract instructions if `force` is true.
async fn retract_concept_if_exists<S: dialog_artifacts::ArtifactStore + ArtifactStoreMut>(
    branch: &S,
    cname: &ConceptName,
    force: bool,
    retract_instructions: &mut Vec<Instruction>,
) -> Result<Option<Entity>> {
    let existing_entity = lookup_concept_by_name(branch, cname).await?;

    if let Some(entity) = existing_entity {
        if force {
            let existing_attrs =
                fetch_string_values(branch, &entity, concept_attribute_selector()).await?;

            for attr_name in &existing_attrs {
                let meta_entity = attribute_meta_entity(attr_name)?;

                for meta_attr in &[
                    ATTR_ATTRIBUTE_DESCRIPTION,
                    ATTR_ATTRIBUTE_TYPE,
                    ATTR_ATTRIBUTE_CARDINALITY,
                    ATTR_ATTRIBUTE_OPTIONAL,
                ] {
                    let claim_attr = parse_claim_attribute(meta_attr)?;
                    if let Some(val) =
                        fetch_string(branch, &meta_entity, claim_attr.clone()).await?
                    {
                        retract_instructions.push(Instruction::Retract(Artifact {
                            the: claim_attr,
                            of: meta_entity.clone(),
                            is: Value::String(val),
                            cause: None,
                        }));
                    }
                }

                retract_instructions.push(Instruction::Retract(Artifact {
                    the: concept_attribute_selector(),
                    of: entity.clone(),
                    is: Value::String(attr_name.clone()),
                    cause: None,
                }));
            }

            if let Some(name) = fetch_string(branch, &entity, concept_name_selector()).await? {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: concept_name_selector(),
                    of: entity.clone(),
                    is: Value::String(name),
                    cause: None,
                }));
            }

            if let Some(desc) =
                fetch_string(branch, &entity, concept_description_selector()).await?
            {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: concept_description_selector(),
                    of: entity.clone(),
                    is: Value::String(desc),
                    cause: None,
                }));
            }

            if let Some(ns) = fetch_string(branch, &entity, concept_namespace_selector()).await? {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: concept_namespace_selector(),
                    of: entity.clone(),
                    is: Value::String(ns),
                    cause: None,
                }));
            }

            return Ok(Some(entity));
        } else {
            anyhow::bail!(
                "Concept '{}' already exists. Use --force to overwrite, \
                 or delete it first with 'tonk concept delete {}'.",
                cname,
                cname
            );
        }
    }

    Ok(None)
}

/// Build assert instructions for a single concept.
fn build_concept_assertions(
    cname: &ConceptName,
    concept: &ParsedConcept,
    assert_instructions: &mut Vec<Instruction>,
    import_summary: &mut Vec<serde_json::Value>,
) -> Result<()> {
    // Derive concept entity from its attribute set (structural identity)
    let qualified_attrs: Vec<String> = concept
        .attributes
        .iter()
        .map(|a| qualify_attribute(&concept.namespace, &a.short_name))
        .collect::<Result<Vec<_>>>()?;
    let empty_cardinalities = std::collections::HashMap::new();
    let entity = concept_entity_from_attrs(&qualified_attrs, &empty_cardinalities)?;

    assert_instructions.push(Instruction::Assert(Artifact {
        the: concept_name_selector(),
        of: entity.clone(),
        is: Value::String(cname.to_string()),
        cause: None,
    }));

    if let Some(desc) = &concept.description {
        assert_instructions.push(Instruction::Assert(Artifact {
            the: concept_description_selector(),
            of: entity.clone(),
            is: Value::String(desc.clone()),
            cause: None,
        }));
    }

    assert_instructions.push(Instruction::Assert(Artifact {
        the: concept_namespace_selector(),
        of: entity.clone(),
        is: Value::String(concept.namespace.clone()),
        cause: None,
    }));

    let mut attr_summaries: Vec<String> = Vec::new();

    for attr in &concept.attributes {
        // Use the explicit qualified reference if one was provided in the YAML
        // (e.g., `handle: carry.links/handle`). Otherwise fall back to
        // generating the path from the concept name (e.g., `member/handle`).
        let qualified = if let Some(ref qr) = attr.qualified_ref {
            if let Some(name) = qr.strip_prefix('.') {
                // `.name` — concept-relative, resolve now
                let prefix = cname.to_lowercase();
                format!("{}/{}", prefix, name)
            } else {
                // Fully qualified — use as-is
                qr.clone()
            }
        } else {
            qualify_attribute(&concept.namespace, &attr.short_name)?
        };

        assert_instructions.push(Instruction::Assert(Artifact {
            the: concept_attribute_selector(),
            of: entity.clone(),
            is: Value::String(qualified.clone()),
            cause: None,
        }));

        let meta_entity = attribute_meta_entity(&qualified)?;

        if let Some(desc) = &attr.description {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: parse_claim_attribute(ATTR_ATTRIBUTE_DESCRIPTION)?,
                of: meta_entity.clone(),
                is: Value::String(desc.clone()),
                cause: None,
            }));
        }

        if let Some(type_str) = &attr.type_str {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: parse_claim_attribute(ATTR_ATTRIBUTE_TYPE)?,
                of: meta_entity.clone(),
                is: Value::String(type_str.clone()),
                cause: None,
            }));
        }

        if let Some(cardinality) = &attr.cardinality {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: parse_claim_attribute(ATTR_ATTRIBUTE_CARDINALITY)?,
                of: meta_entity.clone(),
                is: Value::String(cardinality.clone()),
                cause: None,
            }));
        }

        if attr.optional {
            assert_instructions.push(Instruction::Assert(Artifact {
                the: parse_claim_attribute(ATTR_ATTRIBUTE_OPTIONAL)?,
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

    Ok(())
}

/// Build a list of fully-qualified attribute names for a concept definition.
fn build_concept_attr_list(cname: &ConceptName, concept: &ParsedConcept) -> Result<Vec<String>> {
    let mut attrs = Vec::new();

    for attr in &concept.attributes {
        let qualified = if let Some(ref qr) = attr.qualified_ref {
            if let Some(name) = qr.strip_prefix('.') {
                // `.name` — concept-relative, resolve now
                let prefix = cname.to_lowercase();
                format!("{}/{}", prefix, name)
            } else {
                // Fully qualified — use as-is
                qr.clone()
            }
        } else {
            qualify_attribute(&concept.namespace, &attr.short_name)?
        };

        attrs.push(qualified);
    }

    Ok(attrs)
}

/// Build a cardinality map for a concept definition from parsed attributes.
fn build_concept_cardinalities(
    cname: &ConceptName,
    concept: &ParsedConcept,
) -> Result<std::collections::HashMap<String, dialog_query::Cardinality>> {
    let mut cardinalities = std::collections::HashMap::new();

    for attr in &concept.attributes {
        let Some(cardinality) = &attr.cardinality else {
            continue;
        };
        if cardinality.to_lowercase() != "many" {
            continue;
        }

        let qualified = if let Some(ref qr) = attr.qualified_ref {
            if let Some(name) = qr.strip_prefix('.') {
                let prefix = cname.to_lowercase();
                format!("{}/{}", prefix, name)
            } else {
                qr.clone()
            }
        } else {
            qualify_attribute(&concept.namespace, &attr.short_name)?
        };

        cardinalities.insert(qualified, dialog_query::Cardinality::Many);
    }

    Ok(cardinalities)
}

// ---------------------------------------------------------------------------
// Shared helpers: standalone attribute assertion
// ---------------------------------------------------------------------------

/// Build assert instructions for a standalone attribute.
///
/// Standalone attributes are stored as attribute metadata triples
/// without a parent concept, using a deterministic entity derived
/// from the namespace and attribute name.
fn build_standalone_attr_assertions(
    space_did: &str,
    attr: &ParsedStandaloneAttribute,
    assert_instructions: &mut Vec<Instruction>,
) -> Result<()> {
    // Use a deterministic entity for standalone attributes
    let qualified = format!("{}/{}", attr.namespace, attr.short_name);
    let entity_input = format!("{}\0standalone-attr\0{}", space_did, qualified);
    let meta_entity = derive_entity(&entity_input)?;

    if let Some(desc) = &attr.description {
        assert_instructions.push(Instruction::Assert(Artifact {
            the: parse_claim_attribute(ATTR_ATTRIBUTE_DESCRIPTION)?,
            of: meta_entity.clone(),
            is: Value::String(desc.clone()),
            cause: None,
        }));
    }

    if let Some(type_str) = &attr.type_str {
        assert_instructions.push(Instruction::Assert(Artifact {
            the: parse_claim_attribute(ATTR_ATTRIBUTE_TYPE)?,
            of: meta_entity.clone(),
            is: Value::String(type_str.clone()),
            cause: None,
        }));
    }

    if let Some(cardinality) = &attr.cardinality {
        assert_instructions.push(Instruction::Assert(Artifact {
            the: parse_claim_attribute(ATTR_ATTRIBUTE_CARDINALITY)?,
            of: meta_entity.clone(),
            is: Value::String(cardinality.clone()),
            cause: None,
        }));
    }

    Ok(())
}

/// Check if a standalone attribute already exists and build retract instructions if `force` is true.
async fn retract_standalone_attr_if_exists<
    S: dialog_artifacts::ArtifactStore + ArtifactStoreMut,
>(
    branch: &S,
    space_did: &str,
    attr: &ParsedStandaloneAttribute,
    force: bool,
    retract_instructions: &mut Vec<Instruction>,
) -> Result<()> {
    let qualified = format!("{}/{}", attr.namespace, attr.short_name);
    let entity_input = format!("{}\0standalone-attr\0{}", space_did, qualified);
    let meta_entity = derive_entity(&entity_input)?;

    let mut found_any = false;
    for meta_attr in [
        ATTR_ATTRIBUTE_DESCRIPTION,
        ATTR_ATTRIBUTE_TYPE,
        ATTR_ATTRIBUTE_CARDINALITY,
        ATTR_ATTRIBUTE_OPTIONAL,
    ] {
        let claim_attr = parse_claim_attribute(meta_attr)?;
        if let Some(val) = fetch_string(branch, &meta_entity, claim_attr.clone()).await? {
            found_any = true;
            if force {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: claim_attr,
                    of: meta_entity.clone(),
                    is: Value::String(val),
                    cause: None,
                }));
            }
        }
    }

    if found_any && !force {
        anyhow::bail!(
            "Standalone attribute '{}/{}' already exists. Use --force to overwrite.",
            attr.namespace,
            attr.short_name
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers: rule retraction, validation, and assertion
// ---------------------------------------------------------------------------

/// Check if a rule already exists and build retract instructions if `force` is true.
async fn retract_rule_if_exists<S: dialog_artifacts::ArtifactStore + ArtifactStoreMut>(
    branch: &S,
    parsed: &ParsedRule,
    force: bool,
    retract_instructions: &mut Vec<Instruction>,
) -> Result<()> {
    let existing = lookup_rule_by_name(branch, &parsed.name).await?;

    if let Some(rule_ent) = existing {
        if force {
            let rule_name_attr = parse_claim_attribute(ATTR_RULE_NAME)?;
            if let Some(name) = fetch_string(branch, &rule_ent, rule_name_attr.clone()).await? {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: rule_name_attr,
                    of: rule_ent.clone(),
                    is: Value::String(name),
                    cause: None,
                }));
            }
            let rule_conclusion_attr = parse_claim_attribute(ATTR_RULE_CONCLUSION)?;
            if let Some(conclusion) =
                fetch_string(branch, &rule_ent, rule_conclusion_attr.clone()).await?
            {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: rule_conclusion_attr,
                    of: rule_ent.clone(),
                    is: Value::String(conclusion),
                    cause: None,
                }));
            }
            let rule_def_attr = parse_claim_attribute(ATTR_RULE_DEFINITION)?;
            if let Some(def) = fetch_string(branch, &rule_ent, rule_def_attr.clone()).await? {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: rule_def_attr,
                    of: rule_ent.clone(),
                    is: Value::String(def),
                    cause: None,
                }));
            }
            let rule_desc_attr = parse_claim_attribute(ATTR_RULE_DESCRIPTION)?;
            if let Some(desc) = fetch_string(branch, &rule_ent, rule_desc_attr.clone()).await? {
                retract_instructions.push(Instruction::Retract(Artifact {
                    the: rule_desc_attr,
                    of: rule_ent.clone(),
                    is: Value::String(desc),
                    cause: None,
                }));
            }
        } else {
            anyhow::bail!(
                "Rule '{}' already exists. Use --force to overwrite, \
                 or delete it first with 'tonk rule delete {}'.",
                parsed.name,
                parsed.name
            );
        }
    }

    Ok(())
}

/// Validate a rule against the space's concept schemas.
async fn validate_rule_against_space<S: dialog_artifacts::ArtifactStore + ArtifactStoreMut>(
    branch: &S,
    parsed: &ParsedRule,
    definition: &RuleDefinition,
) -> Result<()> {
    let conclusion_concept = ConceptName::new(&definition.conclusion.concept)?;
    let concept_ent = lookup_concept_by_name(branch, &conclusion_concept)
        .await?
        .context(format!(
            "Conclusion concept '{}' for rule '{}' not found. Define it first.",
            definition.conclusion.concept, parsed.name
        ))?;
    let concept_name = ConceptName::from_stored(
        fetch_string(branch, &concept_ent, concept_name_selector())
            .await?
            .unwrap_or_else(|| conclusion_concept.to_string()),
    );

    let concept_attrs =
        fetch_string_values(branch, &concept_ent, concept_attribute_selector()).await?;
    let concept_ns = fetch_string(branch, &concept_ent, concept_namespace_selector())
        .await?
        .unwrap_or_default();
    let cardinalities = fetch_attribute_cardinalities(branch, &concept_attrs).await?;

    validate_rule_against_schema(
        parsed,
        definition,
        &concept_name,
        &concept_attrs,
        &cardinalities,
        &concept_ns,
    )?;

    Ok(())
}

/// Validate a rule against a provided concept schema.
fn validate_rule_against_schema(
    parsed: &ParsedRule,
    definition: &RuleDefinition,
    concept_name: &ConceptName,
    concept_attrs: &[String],
    cardinalities: &std::collections::HashMap<String, dialog_query::Cardinality>,
    namespace: &str,
) -> Result<()> {
    // Validate bindings match concept schema
    crate::rule::validate_definition(definition, concept_attrs, concept_name, namespace)
        .with_context(|| format!("Rule '{}' validation failed", parsed.name))?;

    // Trial-compile
    crate::rule::compile_rule(
        definition,
        concept_name,
        concept_attrs,
        cardinalities,
        namespace,
    )
    .with_context(|| {
        format!(
            "Rule '{}' failed to compile. Check variable names match between \
                 conclusion bindings and premises.",
            parsed.name
        )
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers: rule assertion building
// ---------------------------------------------------------------------------

/// Build assert instructions for a rule definition.
///
/// Creates triples for the rule's name, conclusion concept, serialised
/// definition, and (optional) description. Registry assertions are no
/// longer needed since rules are discovered structurally.
fn build_rule_assertions(
    space_did: &str,
    parsed: &ParsedRule,
    definition: &RuleDefinition,
    assert_instructions: &mut Vec<Instruction>,
    import_summary: &mut Vec<serde_json::Value>,
) -> Result<()> {
    let rule_ent = rule_entity(space_did, &parsed.name)?;
    let concept_name = &definition.conclusion.concept;
    let definition_str = serde_json::to_string(definition)?;

    assert_instructions.push(Instruction::Assert(Artifact {
        the: parse_claim_attribute(ATTR_RULE_NAME)?,
        of: rule_ent.clone(),
        is: Value::String(parsed.name.clone()),
        cause: None,
    }));

    assert_instructions.push(Instruction::Assert(Artifact {
        the: parse_claim_attribute(ATTR_RULE_CONCLUSION)?,
        of: rule_ent.clone(),
        is: Value::String(concept_name.clone()),
        cause: None,
    }));

    assert_instructions.push(Instruction::Assert(Artifact {
        the: parse_claim_attribute(ATTR_RULE_DEFINITION)?,
        of: rule_ent.clone(),
        is: Value::String(definition_str),
        cause: None,
    }));

    if let Some(desc) = &parsed.description {
        assert_instructions.push(Instruction::Assert(Artifact {
            the: parse_claim_attribute(ATTR_RULE_DESCRIPTION)?,
            of: rule_ent.clone(),
            is: Value::String(desc.clone()),
            cause: None,
        }));
    }

    import_summary.push(serde_json::json!({
        "name": parsed.name,
        "namespace": parsed.namespace,
        "description": parsed.description,
        "conclusion": concept_name,
        "when_count": definition.when.len(),
        "unless_count": definition.unless.len(),
    }));

    Ok(())
}
