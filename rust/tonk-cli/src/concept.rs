//! Concept management: define, list, show, extend, and delete concepts.
//!
//! A concept is a named schema stored as EAV triples in dialog-db. Each concept
//! defines a set of attributes (with an auto-prefixed namespace) that instances
//! conform to.
//!
//! The concept registry is a well-known entity per space that indexes all
//! concepts, enabling enumeration without wildcard queries.

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
pub async fn list(json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let concept_entities = fetch_entity_values(&branch, &registry, ATTR_REGISTRY_CONCEPT).await?;

    if concept_entities.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No concepts defined. Use 'tonk concept define <name>' to create one.");
        }
        return Ok(());
    }

    let mut concepts: Vec<(String, Option<String>, usize)> = Vec::new();
    for entity in &concept_entities {
        let name = fetch_string(&branch, entity, ATTR_CONCEPT_NAME)
            .await?
            .unwrap_or_else(|| "???".to_string());
        let description = fetch_string(&branch, entity, ATTR_CONCEPT_DESCRIPTION).await?;
        let instances = fetch_entity_values(&branch, entity, ATTR_CONCEPT_INSTANCE).await?;
        concepts.push((name, description, instances.len()));
    }

    if json {
        let items: Vec<serde_json::Value> = concepts
            .iter()
            .map(|(name, desc, instance_count)| {
                let mut obj = serde_json::json!({
                    "name": name,
                    "instances": instance_count,
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
        for (name, desc, instance_count) in &concepts {
            let desc_str = desc
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {}{} ({} instances)", name, desc_str, instance_count);
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
/// auto-prefixed with the concept namespace (e.g. `"task/title"`).
pub async fn define(
    name: String,
    attributes: Vec<String>,
    description: Option<String>,
    json: bool,
) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Concept name cannot be empty");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let concept = concept_entity(&ctx.space_did, &name)?;

    // Check if concept already exists
    let existing_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME).await?;
    if existing_name.is_some() {
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
        prompt_attributes(&name)?
    } else {
        attributes
    };

    if attrs.is_empty() {
        anyhow::bail!("A concept must have at least one attribute.");
    }

    // Qualify attribute names
    let qualified_attrs: Vec<String> = attrs
        .iter()
        .map(|a| qualify_attribute(&name, a))
        .collect::<Result<Vec<_>>>()?;

    // Build instructions
    let mut instructions = Vec::new();

    // Register concept in registry
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_REGISTRY_CONCEPT)?,
        of: registry.clone(),
        is: Value::Entity(concept.clone()),
        cause: None,
    }));

    // Set concept name
    instructions.push(Instruction::Assert(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
        of: concept.clone(),
        is: Value::String(name.clone()),
        cause: None,
    }));

    // Set description if provided
    if let Some(desc) = &description {
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
            of: concept.clone(),
            is: Value::String(desc.clone()),
            cause: None,
        }));
    }

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
            "name": name,
            "attributes": qualified_attrs,
            "description": description,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Defined concept '{}'", name);
        println!("  Attributes:");
        for attr in &qualified_attrs {
            println!("    {}", short_attribute(&name, attr));
        }
        if let Some(desc) = &description {
            println!("  Description: {}", desc);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show concept details
// ---------------------------------------------------------------------------

/// Show the schema of a concept.
pub async fn show(name: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let concept = concept_entity(&ctx.space_did, &name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    let description = fetch_string(&branch, &concept, ATTR_CONCEPT_DESCRIPTION).await?;
    let attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;
    let instances = fetch_entity_values(&branch, &concept, ATTR_CONCEPT_INSTANCE).await?;

    if json {
        let mut output = serde_json::json!({
            "name": stored_name,
            "attributes": attrs,
            "instance_count": instances.len(),
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
        println!("  Attributes:");
        for attr in &attrs {
            println!("    {}", short_attribute(&stored_name, attr));
        }
        println!("  Instances: {}", instances.len());
        println!("  Entity: {}", concept);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Extend a concept with new attributes
// ---------------------------------------------------------------------------

/// Add attributes to an existing concept.
pub async fn extend(name: String, attributes: Vec<String>, json: bool) -> Result<()> {
    if attributes.is_empty() {
        anyhow::bail!("No attributes provided to add.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let concept = concept_entity(&ctx.space_did, &name)?;

    // Verify concept exists
    fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    // Get existing attributes to check for duplicates
    let existing = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Qualify new attributes
    let qualified_new: Vec<String> = attributes
        .iter()
        .map(|a| qualify_attribute(&name, a))
        .collect::<Result<Vec<_>>>()?;

    // Filter out attributes that already exist
    let mut added = Vec::new();
    let mut instructions = Vec::new();

    for attr in &qualified_new {
        if existing.contains(attr) {
            if !json {
                eprintln!(
                    "  Attribute '{}' already exists, skipping",
                    short_attribute(&name, attr)
                );
            }
        } else {
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
                of: concept.clone(),
                is: Value::String(attr.clone()),
                cause: None,
            }));
            added.push(attr.clone());
        }
    }

    if instructions.is_empty() {
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

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "added": added,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Extended concept '{}' with:", name);
        for attr in &added {
            println!("  + {}", short_attribute(&name, attr));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delete a concept
// ---------------------------------------------------------------------------

/// Delete a concept and optionally its instances.
pub async fn delete(name: String, force: bool, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let registry = registry_entity(&ctx.space_did)?;
    let concept = concept_entity(&ctx.space_did, &name)?;

    // Verify concept exists
    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", name))?;

    // Check for instances
    let instances = fetch_entity_values(&branch, &concept, ATTR_CONCEPT_INSTANCE).await?;
    let instance_count = instances.len();

    if instance_count > 0 && !force {
        if json {
            let output = serde_json::json!({
                "ok": false,
                "error": format!("Concept '{}' has {} instance(s). Use --force to delete.", name, instance_count),
                "instance_count": instance_count,
            });
            println!("{}", serde_json::to_string(&output)?);
        } else {
            println!(
                "Concept '{}' has {} instance(s). Use --force to delete concept and all instances.",
                name, instance_count
            );
        }
        return Ok(());
    }

    let attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut instructions = Vec::new();

    // If force-deleting, retract all instance data
    if force {
        for instance_entity in &instances {
            // Retract each attribute value for this instance
            for attr_name in &attrs {
                let values = fetch_values(&branch, instance_entity, attr_name).await?;
                for val in values {
                    instructions.push(Instruction::Retract(Artifact {
                        the: Attribute::from_str(attr_name)?,
                        of: instance_entity.clone(),
                        is: val,
                        cause: None,
                    }));
                }
            }

            // Retract instance/type
            instructions.push(Instruction::Retract(Artifact {
                the: Attribute::from_str(ATTR_INSTANCE_TYPE)?,
                of: instance_entity.clone(),
                is: Value::Entity(concept.clone()),
                cause: None,
            }));

            // Retract instance/created
            if let Some(ts) = fetch_value(&branch, instance_entity, ATTR_INSTANCE_CREATED).await? {
                instructions.push(Instruction::Retract(Artifact {
                    the: Attribute::from_str(ATTR_INSTANCE_CREATED)?,
                    of: instance_entity.clone(),
                    is: ts,
                    cause: None,
                }));
            }

            // Retract concept/instance back-reference
            instructions.push(Instruction::Retract(Artifact {
                the: Attribute::from_str(ATTR_CONCEPT_INSTANCE)?,
                of: concept.clone(),
                is: Value::Entity(instance_entity.clone()),
                cause: None,
            }));
        }
    }

    // Retract concept attributes
    for attr_name in &attrs {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_ATTRIBUTE)?,
            of: concept.clone(),
            is: Value::String(attr_name.clone()),
            cause: None,
        }));
    }

    // Retract concept name
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_NAME)?,
        of: concept.clone(),
        is: Value::String(stored_name),
        cause: None,
    }));

    // Retract concept description if present
    if let Some(desc) = fetch_string(&branch, &concept, ATTR_CONCEPT_DESCRIPTION).await? {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_DESCRIPTION)?,
            of: concept.clone(),
            is: Value::String(desc),
            cause: None,
        }));
    }

    // Retract registry entry
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_REGISTRY_CONCEPT)?,
        of: registry.clone(),
        is: Value::Entity(concept.clone()),
        cause: None,
    }));

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "deleted": name,
            "instances_deleted": if force { instance_count } else { 0 },
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted concept '{}'", name);
        if force && instance_count > 0 {
            println!("  Also deleted {} instance(s)", instance_count);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive attribute prompt
// ---------------------------------------------------------------------------

fn prompt_attributes(concept_name: &str) -> Result<Vec<String>> {
    let prefix = concept_name.to_lowercase();
    println!(
        "Define attributes for '{}' (will be stored as '{}/...').",
        concept_name, prefix
    );
    println!("Enter attribute names one per line. Empty line to finish.\n");

    let mut attrs = Vec::new();
    loop {
        let input: String = dialoguer::Input::new()
            .with_prompt(format!("  {}/", prefix))
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
