//! Concept management: define, list, show, extend, and delete concepts.
//!
//! A concept is a named schema stored as EAV triples in dialog-db. Each concept
//! defines a set of attributes (with an auto-prefixed namespace) that entities
//! conform to.
//!
//! Concepts are modeled using a typed meta-schema: the `RegisteredConcept`
//! struct and its `concept::*` attribute newtypes define "the concept of a
//! concept" as a first-class concept in dialog-query.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::{Entity, Value};

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
        let description = fetch_string(&session, entity, concept_description_selector()).await?;
        let attrs = fetch_string_values(&session, entity, concept_attribute_selector()).await?;
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
// Instruction builders
// ---------------------------------------------------------------------------

/// Build instructions to assert a concept's core metadata: name, description,
/// namespace, and all attributes.
///
/// Uses the typed `concept::*` selectors from the meta-schema.
///
/// This is the shared instruction set for both fresh defines and concept
/// replacements. It does NOT include provenance or soft-delete instructions.
fn build_concept_instructions(
    entity: &Entity,
    name: &str,
    description: &str,
    namespace: &str,
    attributes: &[String],
) -> Vec<Instruction> {
    let mut instructions = Vec::new();

    instructions.push(Instruction::Assert(Artifact {
        the: concept_name_selector(),
        of: entity.clone(),
        is: Value::String(name.to_string()),
        cause: None,
    }));

    instructions.push(Instruction::Assert(Artifact {
        the: concept_description_selector(),
        of: entity.clone(),
        is: Value::String(description.to_string()),
        cause: None,
    }));

    instructions.push(Instruction::Assert(Artifact {
        the: concept_namespace_selector(),
        of: entity.clone(),
        is: Value::String(namespace.to_string()),
        cause: None,
    }));

    for attr in attributes {
        instructions.push(Instruction::Assert(Artifact {
            the: concept_attribute_selector(),
            of: entity.clone(),
            is: Value::String(attr.clone()),
            cause: None,
        }));
    }

    instructions
}

/// Build instructions to replace one concept entity with another.
///
/// Asserts all core metadata on `new_entity`, links it to `old_entity` via
/// `concept/prior`, optionally records an update rationale, and soft-deletes
/// the old entity by retracting its name.
///
/// Used by both `define` (convergent conflict update) and `extend` (identity
/// change on attribute addition).
#[allow(clippy::too_many_arguments)]
fn build_replace_concept_instructions(
    new_entity: &Entity,
    old_entity: &Entity,
    name: &str,
    description: &str,
    namespace: &str,
    attributes: &[String],
    old_stored_name: &str,
    rationale: Option<&str>,
) -> Vec<Instruction> {
    let mut instructions =
        build_concept_instructions(new_entity, name, description, namespace, attributes);

    // Provenance: link new -> old
    instructions.push(Instruction::Assert(Artifact {
        the: concept_prior_selector(),
        of: new_entity.clone(),
        is: Value::String(old_entity.to_string()),
        cause: None,
    }));

    // Store rationale if provided
    if let Some(r) = rationale {
        let r = r.trim();
        if !r.is_empty() {
            instructions.push(Instruction::Assert(Artifact {
                the: concept_update_rationale_selector(),
                of: new_entity.clone(),
                is: Value::String(r.to_string()),
                cause: None,
            }));
        }
    }

    // Soft-delete old: retract its name so it's no longer discoverable
    instructions.push(Instruction::Retract(Artifact {
        the: concept_name_selector(),
        of: old_entity.clone(),
        is: Value::String(old_stored_name.to_string()),
        cause: None,
    }));

    instructions
}

// ---------------------------------------------------------------------------
// Define a new concept
// ---------------------------------------------------------------------------

/// Define a new concept with the given name and attributes.
///
/// Attributes are short names (e.g. `"title"`, `"status"`) that will be
/// auto-prefixed with the space name as namespace (e.g. `"my-space/title"`).
///
/// Uses convergent semantics: the concept entity is derived deterministically
/// from its attribute set. If a concept with the same name already exists:
///   - **Same attributes** -> noop (idempotent, the assertions converge).
///   - **Different attributes** -> in interactive mode, prompt the user to
///     update (creating a provenance chain via `concept/prior`) or choose a
///     different name. In `--json` mode, return a structured conflict response
///     so programmatic callers can decide.
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
    let new_entity = concept_entity_from_attrs(&qualified_attrs, &empty_cardinalities)?;

    // Check if a concept with this name already exists
    if let Some(existing_entity) = lookup_concept_by_name(&branch, &name).await? {
        if existing_entity == new_entity {
            return handle_converged_define(
                &mut branch,
                &name,
                namespace,
                &qualified_attrs,
                &description,
                &new_entity,
                json,
            )
            .await;
        }

        let existing_attrs =
            fetch_string_values(&branch, &existing_entity, concept_attribute_selector()).await?;

        return handle_conflict_define(
            &mut branch,
            &name,
            namespace,
            &qualified_attrs,
            &description,
            &new_entity,
            &existing_entity,
            &existing_attrs,
            json,
        )
        .await;
    }

    // No existing concept with this name -- fresh define.
    let instructions = build_concept_instructions(
        &new_entity,
        name.as_str(),
        &description,
        namespace,
        &qualified_attrs,
    );

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

/// Handle the convergent case: same name and same structural identity.
///
/// Re-asserts the description (in case it changed) and reports success.
/// This is effectively a noop for identical definitions.
async fn handle_converged_define(
    branch: &mut impl ArtifactStoreMut,
    name: &ConceptName,
    namespace: &str,
    qualified_attrs: &[String],
    description: &str,
    entity: &Entity,
    json: bool,
) -> Result<()> {
    // Re-assert description in case it changed; harmless if identical.
    let instructions = vec![Instruction::Assert(Artifact {
        the: concept_description_selector(),
        of: entity.clone(),
        is: Value::String(description.to_string()),
        cause: None,
    })];
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "converged": true,
            "name": name.as_str(),
            "namespace": namespace,
            "attributes": qualified_attrs,
            "description": description,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "Concept '{}' already exists with identical schema -- no changes needed.",
            name
        );
    }

    Ok(())
}

/// Handle the conflict case: same name but different structural identity.
///
/// In `--json` mode, returns structured conflict info for the caller to decide.
/// In interactive mode, prompts the user to update (with provenance chain and
/// optional rationale) or cancel.
#[allow(clippy::too_many_arguments)]
async fn handle_conflict_define(
    branch: &mut impl ArtifactStoreMut,
    name: &ConceptName,
    namespace: &str,
    proposed_attrs: &[String],
    description: &str,
    new_entity: &Entity,
    existing_entity: &Entity,
    existing_attrs: &[String],
    json: bool,
) -> Result<()> {
    if json {
        let output = serde_json::json!({
            "ok": false,
            "conflict": true,
            "name": name.as_str(),
            "existing_entity": existing_entity.to_string(),
            "existing_attributes": existing_attrs.iter()
                .map(|a| short_attribute(namespace, a))
                .collect::<Vec<_>>(),
            "proposed_entity": new_entity.to_string(),
            "proposed_attributes": proposed_attrs.iter()
                .map(|a| short_attribute(namespace, a))
                .collect::<Vec<_>>(),
            "message": format!(
                "A different concept already exists under the name '{}'. \
                 Re-run with --update to replace it (a provenance link will be created), \
                 or choose a different name.",
                name
            ),
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    // Interactive conflict resolution
    println!(
        "A concept named '{}' already exists with a different attribute set.\n",
        name
    );
    println!("  Existing attributes:");
    for attr in existing_attrs {
        println!("    {}", short_attribute(namespace, attr));
    }
    println!("  Proposed attributes:");
    for attr in proposed_attrs {
        println!("    {}", short_attribute(namespace, attr));
    }
    println!();

    let choices = &[
        "Update -- replace with the new definition (provenance link preserved)",
        "Cancel -- keep the existing concept unchanged",
    ];
    let selection = dialoguer::Select::new()
        .with_prompt("How would you like to proceed?")
        .items(choices)
        .default(0)
        .interact()?;

    if selection == 1 {
        println!("Cancelled. Existing concept '{}' is unchanged.", name);
        return Ok(());
    }

    // Prompt for optional rationale
    let rationale: String = dialoguer::Input::new()
        .with_prompt("Rationale for this update (optional, enter to skip)")
        .allow_empty(true)
        .interact_text()?;

    let stored_name = fetch_string(branch, existing_entity, concept_name_selector())
        .await?
        .unwrap_or_else(|| name.to_string());

    let instructions = build_replace_concept_instructions(
        new_entity,
        existing_entity,
        name.as_str(),
        description,
        namespace,
        proposed_attrs,
        &stored_name,
        Some(&rationale),
    );

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    println!("Updated concept '{}'", name);
    println!("  Namespace: {}", namespace);
    println!("  Attributes:");
    for attr in proposed_attrs {
        println!("    {}", short_attribute(namespace, attr));
    }
    println!("  Description: {}", description);
    println!("  Prior concept entity: {}", existing_entity);

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
        fetch_string(&session, &concept, concept_name_selector())
            .await?
            .unwrap_or_else(|| name.to_string()),
    );

    let description = fetch_string(&session, &concept, concept_description_selector()).await?;
    let namespace = fetch_string(&session, &concept, concept_namespace_selector())
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());
    let attrs = fetch_string_values(&session, &concept, concept_attribute_selector()).await?;
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
    let namespace = fetch_string(&branch, &old_entity, concept_namespace_selector())
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    // Get existing metadata
    let existing_attrs =
        fetch_string_values(&branch, &old_entity, concept_attribute_selector()).await?;
    let stored_name = fetch_string(&branch, &old_entity, concept_name_selector())
        .await?
        .unwrap_or_else(|| name.to_string());
    let description = fetch_string(&branch, &old_entity, concept_description_selector()).await?;

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

    let instructions = if new_entity != old_entity {
        // Entity identity changed: create a new concept entity with all metadata
        // and provenance link, then soft-delete the old one.
        build_replace_concept_instructions(
            &new_entity,
            &old_entity,
            &stored_name,
            description.as_deref().unwrap_or(""),
            &namespace,
            &full_attrs,
            &stored_name,
            None,
        )
    } else {
        // Entity identity unchanged (shouldn't normally happen when adding attrs,
        // but handle gracefully): just assert the new attributes
        let mut instr = Vec::new();
        for attr in &added {
            instr.push(Instruction::Assert(Artifact {
                the: concept_attribute_selector(),
                of: old_entity.clone(),
                is: Value::String(attr.clone()),
                cause: None,
            }));
        }
        instr
    };

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

    let stored_name = fetch_string(&branch, &concept, concept_name_selector())
        .await?
        .unwrap_or_else(|| name.to_string());

    let attrs = fetch_string_values(&branch, &concept, concept_attribute_selector()).await?;

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
        the: concept_name_selector(),
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
