//! Concept management: define, list, show, extend, and delete concepts.
//!
//! A concept is a named schema stored as EAV triples in dialog-db. Each concept
//! defines a set of attributes (with an auto-prefixed namespace) that entities
//! conform to.
//!
//! Concepts are discovered structurally: any entity with a `concept/name`
//! attribute is a concept. No explicit registry entity is needed.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::Value;
use dialog_query::claim::Attribute;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// List all concepts
// ---------------------------------------------------------------------------

/// List all concepts in the active space.
pub async fn list(ctx: &SpaceContext, json: bool) -> Result<()> {
    let session = open_session(ctx).await?;

    let concept_entries = find_all_concepts(&session).await?;

    if concept_entries.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No concepts defined. Use 'tonk concept define <name>' to create one.");
        }
        return Ok(());
    }

    let mut concepts: Vec<(String, Option<String>, usize)> = Vec::new();
    for (entity, name) in &concept_entries {
        let description = fetch_string(&session, entity, ATTR_CONCEPT_DESCRIPTION).await?;
        // Count entities by querying the AEV index on schema attributes
        let attrs = fetch_string_values(&session, entity, ATTR_CONCEPT_ATTRIBUTE).await?;
        let entity_count = if attrs.is_empty() {
            0
        } else {
            find_entities_by_concept(&session, &attrs).await?.len()
        };
        concepts.push((name.clone(), description, entity_count));
    }

    if json {
        let items: Vec<serde_json::Value> = concepts
            .iter()
            .map(|(name, desc, entity_count)| {
                let mut obj = serde_json::json!({
                    "name": name,
                    "entities": entity_count,
                });
                if let Some(d) = desc {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("description".to_string(), serde_json::json!(d));
                }
                obj
            })
            .collect();
        println!("{}", serde_json::to_string(&items)?);
    } else {
        println!("Concepts:\n");
        for (name, desc, entity_count) in &concepts {
            let desc_str = desc
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {}{} ({} entities)", name, desc_str, entity_count);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Define a new concept
// ---------------------------------------------------------------------------

/// Define a new concept with the given name and attributes.
///
/// Attributes are short names (e.g. `"title"`, `"status"`) that will be
/// auto-prefixed with the space name as namespace (e.g. `"my-space/title"`).
///
/// Uses raw Branch + Instruction (not Session/Transaction) because a
/// concept has multi-valued `concept/attribute` entries. Transaction
/// deduplicates by `(entity, attribute)`, so only the last value per pair
/// would survive.
pub async fn define(
    ctx: &SpaceContext,
    name: String,
    attributes: Vec<String>,
    description: String,
    json: bool,
) -> Result<()> {
    let name = ConceptName::new(name)?;

    let namespace = &ctx.space_name;
    let mut branch = open_branch(ctx).await?;

    // Check if concept with this name already exists
    if let Some(_existing) = lookup_concept_by_name(&branch, &name).await? {
        anyhow::bail!(
            "Concept '{}' already exists. Use 'tonk concept extend {}' to add attributes.",
            name,
            name
        );
    }

    // If no attributes provided, prompt interactively (unless --json mode)
    let attrs = if attributes.is_empty() {
        if json {
            anyhow::bail!("No attributes provided. Pass attribute names as arguments.");
        }
        prompt_attributes(namespace)?
    } else {
        attributes
    };

    if attrs.is_empty() {
        anyhow::bail!("A concept must have at least one attribute.");
    }

    // Qualify attribute names with the space namespace
    let qualified_attrs: Vec<String> = attrs
        .iter()
        .map(|a| qualify_attribute(namespace, a))
        .collect::<Result<Vec<_>>>()?;

    // Derive concept entity from attribute set (structural identity)
    let empty_cardinalities = std::collections::HashMap::new();
    let concept = concept_entity_from_attrs(&qualified_attrs, &empty_cardinalities)?;

    // Build instructions
    let mut instructions = Vec::new();

    // Set concept name
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
        of: concept.clone(),
        is: Value::String(name.to_string()),
        cause: None,
    }));

    // Set description
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
        of: concept.clone(),
        is: Value::String(description.clone()),
        cause: None,
    }));

    // Store the namespace
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
        of: concept.clone(),
        is: Value::String(namespace.to_string()),
        cause: None,
    }));

    // Add each attribute
    for attr in &qualified_attrs {
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
            of: concept.clone(),
            is: Value::String(attr.clone()),
            cause: None,
        }));
    }

    // Commit all
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "name": name.as_str(),
            "namespace": namespace,
            "attributes": qualified_attrs,
            "description": description,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Defined concept '{}'", name);
        println!("  Namespace: {}", namespace);
        println!("  Attributes:");
        for attr in &qualified_attrs {
            println!("    {}", short_attribute(namespace, attr));
        }
        println!("  Description: {}", description);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show concept details
// ---------------------------------------------------------------------------

/// Show the schema of a concept.
pub async fn show(ctx: &SpaceContext, name: String, json: bool) -> Result<()> {
    let session = open_session(ctx).await?;

    let name = ConceptName::new(name)?;
    let concept = lookup_concept_by_name(&session, &name)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    let stored_name = ConceptName::from_stored(
        fetch_string(&session, &concept, ATTR_CONCEPT_NAME)
            .await?
            .unwrap_or_else(|| name.to_string()),
    );

    let description = fetch_string(&session, &concept, ATTR_CONCEPT_DESCRIPTION).await?;
    let namespace = fetch_string(&session, &concept, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());
    let attrs = fetch_string_values(&session, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;
    // Count entities by querying the AEV index on the first schema attribute
    let entity_count = if attrs.is_empty() {
        0
    } else {
        find_entities_by_concept(&session, &attrs).await?.len()
    };

    if json {
        let mut output = serde_json::json!({
            "name": stored_name.as_str(),
            "namespace": namespace,
            "attributes": attrs,
            "entity_count": entity_count,
            "entity": concept.to_string(),
        });
        if let Some(desc) = &description {
            output
                .as_object_mut()
                .unwrap()
                .insert("description".to_string(), serde_json::json!(desc));
        }
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Concept: {}", stored_name);
        if let Some(desc) = &description {
            println!("  Description: {}", desc);
        }
        println!("  Namespace: {}", namespace);
        println!("  Attributes:");
        for attr in &attrs {
            println!("    {}", short_attribute(&namespace, attr));
        }
        println!("  Entities: {}", entity_count);
        println!("  Entity: {}", concept);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Extend a concept with new attributes
// ---------------------------------------------------------------------------

/// Add attributes to an existing concept.
///
/// Uses raw Branch + Instruction for the same reason as [`define`]:
/// multi-valued `concept/attribute` cannot be accumulated in a single
/// Transaction.
pub async fn extend(
    ctx: &SpaceContext,
    name: String,
    attributes: Vec<String>,
    json: bool,
) -> Result<()> {
    if attributes.is_empty() {
        anyhow::bail!("No attributes provided to add.");
    }

    let mut branch = open_branch(ctx).await?;

    let name = ConceptName::new(name)?;
    let old_entity = lookup_concept_by_name(&branch, &name)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    // Get the concept's namespace (fall back to space name)
    let namespace = fetch_string(&branch, &old_entity, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    // Get existing metadata
    let existing_attrs = fetch_string_values(&branch, &old_entity, ATTR_CONCEPT_ATTRIBUTE).await?;
    let stored_name = fetch_string(&branch, &old_entity, ATTR_CONCEPT_NAME)
        .await?
        .unwrap_or_else(|| name.to_string());
    let description = fetch_string(&branch, &old_entity, ATTR_CONCEPT_DESCRIPTION).await?;

    // Qualify new attributes with the concept's namespace
    let qualified_new: Vec<String> = attributes
        .iter()
        .map(|a| qualify_attribute(&namespace, a))
        .collect::<Result<Vec<_>>>()?;

    // Filter out attributes that already exist
    let mut added = Vec::new();
    for attr in &qualified_new {
        if existing_attrs.contains(attr) {
            if !json {
                eprintln!(
                    "  Attribute '{}' already exists, skipping",
                    short_attribute(&namespace, attr)
                );
            }
        } else {
            added.push(attr.clone());
        }
    }

    if added.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"ok": true, "added": []}))?
            );
        } else {
            println!("No new attributes to add.");
        }
        return Ok(());
    }

    // Compute the full attribute set and derive the new entity
    let mut full_attrs: Vec<String> = existing_attrs.clone();
    full_attrs.extend(added.iter().cloned());
    full_attrs.sort();

    let empty_cardinalities = std::collections::HashMap::new();
    let new_entity = concept_entity_from_attrs(&full_attrs, &empty_cardinalities)?;

    let mut instructions = Vec::new();

    if new_entity != old_entity {
        // Entity identity changed: create a new concept entity with all metadata
        // and provenance link, then soft-delete the old one.

        // Assert name on new entity
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
            of: new_entity.clone(),
            is: Value::String(stored_name.clone()),
            cause: None,
        }));

        // Assert namespace on new entity
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAMESPACE)?,
            of: new_entity.clone(),
            is: Value::String(namespace.clone()),
            cause: None,
        }));

        // Assert description on new entity (if present)
        if let Some(ref desc) = description {
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
                of: new_entity.clone(),
                is: Value::String(desc.clone()),
                cause: None,
            }));
        }

        // Assert all attributes (existing + new) on new entity
        for attr in &full_attrs {
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                of: new_entity.clone(),
                is: Value::String(attr.clone()),
                cause: None,
            }));
        }

        // Assert provenance link: new entity's prior is the old entity
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_PRIOR)?,
            of: new_entity.clone(),
            is: Value::String(old_entity.to_string()),
            cause: None,
        }));

        // Soft-delete old entity: retract its name so it's no longer discoverable
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
            of: old_entity.clone(),
            is: Value::String(stored_name),
            cause: None,
        }));
    } else {
        // Entity identity unchanged (shouldn't normally happen when adding attrs,
        // but handle gracefully): just assert the new attributes
        for attr in &added {
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                of: old_entity.clone(),
                is: Value::String(attr.clone()),
                cause: None,
            }));
        }
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let mut output = serde_json::json!({
            "ok": true,
            "added": added,
        });
        if new_entity != old_entity {
            output.as_object_mut().unwrap().insert(
                "prior".to_string(),
                serde_json::json!(old_entity.to_string()),
            );
            output.as_object_mut().unwrap().insert(
                "entity".to_string(),
                serde_json::json!(new_entity.to_string()),
            );
        }
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Extended concept '{}' with:", name);
        for attr in &added {
            println!("  + {}", short_attribute(&namespace, attr));
        }
        if new_entity != old_entity {
            println!(
                "  Entity identity changed: {} -> {}",
                old_entity, new_entity
            );
            println!("  Prior concept entity: {}", old_entity);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delete a concept
// ---------------------------------------------------------------------------

/// Delete a concept and optionally its entities.
///
/// Uses raw Branch + Instruction for the same reason as [`define`]:
/// retracting multi-valued attributes requires individual instructions.
pub async fn delete(ctx: &SpaceContext, name: String, force: bool, json: bool) -> Result<()> {
    let mut branch = open_branch(ctx).await?;

    let name = ConceptName::new(name)?;
    let concept = lookup_concept_by_name(&branch, &name)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .unwrap_or_else(|| name.to_string());

    let attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Check for entities by querying the AEV index
    let entities = if attrs.is_empty() {
        Vec::new()
    } else {
        find_entities_by_concept(&branch, &attrs).await?
    };
    let entity_count = entities.len();

    if entity_count > 0 && !force {
        if json {
            let output = serde_json::json!({
                "ok": false,
                "error": format!(            "Concept '{}' has {} entities. Use --force to delete.", name, entity_count),
                "entity_count": entity_count,
            });
            println!("{}", serde_json::to_string(&output)?);
        }
        anyhow::bail!(
            "Concept '{}' has {} entities. Use --force to delete concept and all entities.",
            name,
            entity_count
        );
    }

    let mut instructions = Vec::new();

    // If force-deleting, retract all facts about each entity
    if force {
        for entity in &entities {
            let all_facts = fetch_all_entity_facts(&branch, entity).await?;
            for artifact in all_facts {
                instructions.push(Instruction::Retract(Artifact {
                    the: artifact.the,
                    of: artifact.of,
                    is: artifact.is,
                    cause: None,
                }));
            }
        }
    }

    // Soft delete: only retract the concept name.
    // The entity keeps its attributes, description, and namespace data intact.
    // This makes the concept undiscoverable by name but preserves its data.
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
        of: concept.clone(),
        is: Value::String(stored_name),
        cause: None,
    }));

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "deleted": name.as_str(),
            "entities_deleted": if force { entity_count } else { 0 },
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted concept '{}'", name);
        if force && entity_count > 0 {
            println!("  Also deleted {} entities", entity_count);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive attribute prompt
// ---------------------------------------------------------------------------

fn prompt_attributes(namespace: &str) -> Result<Vec<String>> {
    println!("Define attributes (will be stored as '{}/...').", namespace);
    println!("Enter attribute names one per line. Empty line to finish.\n");

    let mut attrs = Vec::new();
    loop {
        let input: String = dialoguer::Input::new()
            .with_prompt(format!("  {}/", namespace))
            .allow_empty(true)
            .interact_text()?;

        let input = input.trim().to_string();
        if input.is_empty() {
            break;
        }
        attrs.push(input);
    }

    Ok(attrs)
}
