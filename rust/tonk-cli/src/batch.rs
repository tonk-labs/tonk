//! Batch instance operations: create, update, and delete multiple instances
//! of a concept in a single atomic commit.
//!
//! All batch operations accept a JSON array via `--file` or `--stdin`.
//! If any item fails validation, the entire batch aborts with no changes
//! committed.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Batch create
// ---------------------------------------------------------------------------

/// Create multiple instances of a concept in a single atomic commit.
///
/// Input is a JSON array of objects, where each object maps short attribute
/// names to values.
pub async fn batch_create(
    concept_name: String,
    file: Option<String>,
    stdin: bool,
    json: bool,
) -> Result<()> {
    let input = read_json_input(file.as_deref(), stdin)?;
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&input).context("Input must be a JSON array")?;

    if items.is_empty() {
        anyhow::bail!("Empty array — nothing to create.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!(
            "Concept '{}' not found. Define it first with 'tonk concept define {}'.",
            concept_name, concept_name
        ))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let now = chrono::Utc::now().timestamp();
    let mut instructions = Vec::new();
    let mut results: Vec<serde_json::Value> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let obj = item
            .as_object()
            .context(format!("Item at index {} is not a JSON object", idx))?;

        if obj.is_empty() {
            anyhow::bail!("Item at index {} has no fields.", idx);
        }

        // Parse and validate fields
        let mut qualified_fields: Vec<(String, String)> = Vec::new();
        for (key, value) in obj {
            let value_str = json_value_to_string(value);
            let qualified = qualify_attribute(&stored_name, key)?;
            if !schema_attrs.contains(&qualified) {
                anyhow::bail!(
                    "Item at index {}: attribute '{}' is not defined in concept '{}'. Known attributes: {}",
                    idx,
                    key,
                    stored_name,
                    schema_attrs
                        .iter()
                        .map(|a| short_attribute(&stored_name, a))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            qualified_fields.push((qualified, value_str));
        }

        let instance_entity = Entity::new().context("Failed to generate instance entity")?;

        // Instance type reference
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_INSTANCE_TYPE)?,
            of: instance_entity.clone(),
            is: Value::Entity(concept.clone()),
            cause: None,
        }));

        // Instance creation timestamp
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_INSTANCE_CREATED)?,
            of: instance_entity.clone(),
            is: Value::SignedInt(now as i128),
            cause: None,
        }));

        // Attribute values
        for (attr_name, value_str) in &qualified_fields {
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(attr_name)?,
                of: instance_entity.clone(),
                is: parse_value(value_str),
                cause: None,
            }));
        }

        // Back-reference from concept to instance
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_INSTANCE)?,
            of: concept.clone(),
            is: Value::Entity(instance_entity.clone()),
            cause: None,
        }));

        // Collect result data
        let mut data = serde_json::Map::new();
        for (attr_name, value_str) in &qualified_fields {
            let short = short_attribute(&stored_name, attr_name);
            data.insert(short, serde_json::json!(value_str));
        }
        results.push(serde_json::json!({
            "id": instance_entity.to_string(),
            "data": data,
        }));
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": results.len(),
            "created": results,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Created {} {} instance(s)", results.len(), stored_name);
        for result in &results {
            println!("  {}", result["id"].as_str().unwrap_or("???"));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Batch update
// ---------------------------------------------------------------------------

/// Update multiple instances of a concept in a single atomic commit.
///
/// Input is a JSON array of objects, where each object must include an `"id"`
/// field (the instance DID) plus the fields to update.
pub async fn batch_update(
    concept_name: String,
    file: Option<String>,
    stdin: bool,
    json: bool,
) -> Result<()> {
    let input = read_json_input(file.as_deref(), stdin)?;
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&input).context("Input must be a JSON array")?;

    if items.is_empty() {
        anyhow::bail!("Empty array — nothing to update.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut instructions = Vec::new();
    let mut results: Vec<serde_json::Value> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let obj = item
            .as_object()
            .context(format!("Item at index {} is not a JSON object", idx))?;

        let id_str = obj.get("id").and_then(|v| v.as_str()).context(format!(
            "Item at index {} is missing required \"id\" field (string)",
            idx
        ))?;

        let entity = Entity::from_str(id_str).context(format!(
            "Item at index {}: invalid instance ID '{}'",
            idx, id_str
        ))?;

        // Verify this entity is actually an instance of the expected concept
        let instance_type = fetch_value(&branch, &entity, ATTR_INSTANCE_TYPE)
            .await?
            .context(format!(
                "Item at index {}: instance '{}' not found (no instance/type)",
                idx, id_str
            ))?;

        match &instance_type {
            Value::Entity(e) if *e == concept => {}
            _ => anyhow::bail!(
                "Item at index {}: instance '{}' does not belong to concept '{}'",
                idx,
                id_str,
                stored_name
            ),
        }

        let mut updated_fields: Vec<(String, String)> = Vec::new();

        for (key, value) in obj {
            if key == "id" {
                continue;
            }
            let value_str = json_value_to_string(value);
            let qualified = qualify_attribute(&stored_name, key)?;
            if !schema_attrs.contains(&qualified) {
                anyhow::bail!(
                    "Item at index {}: attribute '{}' is not defined in concept '{}'",
                    idx,
                    key,
                    stored_name
                );
            }

            let new_value = parse_value(&value_str);

            // Retract old value if it exists and differs
            if let Some(old_value) = fetch_value(&branch, &entity, &qualified).await?
                && old_value != new_value
            {
                instructions.push(Instruction::Retract(Artifact {
                    the: Attribute::from_str(&qualified)?,
                    of: entity.clone(),
                    is: old_value,
                    cause: None,
                }));
            }

            // Assert new value
            instructions.push(Instruction::Assert(Artifact {
                the: Attribute::from_str(&qualified)?,
                of: entity.clone(),
                is: new_value,
                cause: None,
            }));

            updated_fields.push((short_attribute(&stored_name, &qualified), value_str));
        }

        if updated_fields.is_empty() {
            anyhow::bail!(
                "Item at index {}: no fields to update (only \"id\" was provided)",
                idx
            );
        }

        results.push(serde_json::json!({
            "id": id_str,
            "updated": updated_fields.iter().map(|(k, v)| serde_json::json!({k: v})).collect::<Vec<_>>(),
        }));
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": results.len(),
            "updated": results,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Updated {} {} instance(s)", results.len(), stored_name);
        for result in &results {
            if let Some(id) = result["id"].as_str() {
                println!("  {}", id);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Batch delete
// ---------------------------------------------------------------------------

/// Delete multiple instances of a concept in a single atomic commit.
///
/// Input is a JSON array of instance ID strings.
pub async fn batch_delete(
    concept_name: String,
    file: Option<String>,
    stdin: bool,
    json: bool,
) -> Result<()> {
    let input = read_json_input(file.as_deref(), stdin)?;
    let ids: Vec<String> =
        serde_json::from_str(&input).context("Input must be a JSON array of ID strings")?;

    if ids.is_empty() {
        anyhow::bail!("Empty array — nothing to delete.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut instructions = Vec::new();
    let mut deleted_ids: Vec<String> = Vec::new();

    for (idx, id_str) in ids.iter().enumerate() {
        let entity = Entity::from_str(id_str).context(format!(
            "Item at index {}: invalid instance ID '{}'",
            idx, id_str
        ))?;

        // Verify this entity is actually an instance of the expected concept
        let instance_type = fetch_value(&branch, &entity, ATTR_INSTANCE_TYPE)
            .await?
            .context(format!(
                "Item at index {}: instance '{}' not found (no instance/type)",
                idx, id_str
            ))?;

        match &instance_type {
            Value::Entity(e) if *e == concept => {}
            _ => anyhow::bail!(
                "Item at index {}: instance '{}' does not belong to concept '{}'",
                idx,
                id_str,
                stored_name
            ),
        }

        // Retract all attribute values
        for attr_name in &schema_attrs {
            if let Some(val) = fetch_value(&branch, &entity, attr_name).await? {
                instructions.push(Instruction::Retract(Artifact {
                    the: Attribute::from_str(attr_name)?,
                    of: entity.clone(),
                    is: val,
                    cause: None,
                }));
            }
        }

        // Retract instance/type
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_INSTANCE_TYPE)?,
            of: entity.clone(),
            is: Value::Entity(concept.clone()),
            cause: None,
        }));

        // Retract instance/created
        if let Some(ts) = fetch_value(&branch, &entity, ATTR_INSTANCE_CREATED).await? {
            instructions.push(Instruction::Retract(Artifact {
                the: Attribute::from_str(ATTR_INSTANCE_CREATED)?,
                of: entity.clone(),
                is: ts,
                cause: None,
            }));
        }

        // Retract concept/instance back-reference
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_CONCEPT_INSTANCE)?,
            of: concept.clone(),
            is: Value::Entity(entity.clone()),
            cause: None,
        }));

        deleted_ids.push(id_str.clone());
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": deleted_ids.len(),
            "deleted": deleted_ids,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted {} {} instance(s)", deleted_ids.len(), stored_name);
        for id in &deleted_ids {
            println!("  {}", id);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read JSON input from a file path or stdin.
///
/// Exactly one of `file` or `stdin` must be specified.
fn read_json_input(file: Option<&str>, stdin: bool) -> Result<String> {
    if let Some(path) = file {
        if stdin {
            anyhow::bail!("Specify either --file or --stdin, not both.");
        }
        std::fs::read_to_string(path).context(format!("Failed to read file: {}", path))
    } else if stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read from stdin")?;
        Ok(buf.trim().to_string())
    } else {
        anyhow::bail!("Provide input via --file or --stdin.");
    }
}

/// Convert a serde_json::Value to a string for use with parse_value.
fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}
