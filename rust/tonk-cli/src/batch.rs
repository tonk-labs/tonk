//! Batch entity operations: create, update, and delete multiple entities
//! of a concept in a single atomic commit.
//!
//! All batch operations accept a JSON array via `--file` or `--stdin`.
//! If any item fails validation, the entire batch aborts with no changes
//! committed.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_query::Entity;
use dialog_query::claim::{Attribute, Relation};
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Batch create
// ---------------------------------------------------------------------------

/// Create multiple entities of a concept in a single atomic commit.
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
    let mut session = open_session(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = lookup_concept_by_name(&session, &ctx.space_did, &concept_name)
        .await?
        .context(format!(
            "Concept '{}' not found. Define it first with 'tonk concept define {}'.",
            concept_name, concept_name
        ))?;

    let stored_name = ConceptName::from_stored(
        fetch_string(&session, &concept, ATTR_CONCEPT_NAME)
            .await?
            .unwrap_or_else(|| concept_name.to_string()),
    );

    let namespace = fetch_string(&session, &concept, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    let schema_attrs = fetch_string_values(&session, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut transaction = session.edit();
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
            let qualified = qualify_attribute(&namespace, key)?;
            if !schema_attrs.contains(&qualified) {
                anyhow::bail!(
                    "Item at index {}: attribute '{}' is not defined in concept '{}'. Known attributes: {}",
                    idx,
                    key,
                    stored_name,
                    schema_attrs
                        .iter()
                        .map(|a| short_attribute(&namespace, a))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            qualified_fields.push((qualified, value_str));
        }

        let entity = derive_entity_from_fields(&qualified_fields)?;

        // Assert attribute values via Transaction
        for (attr_name, value_str) in &qualified_fields {
            let relation = Relation::new(
                Attribute::from_str(attr_name)?,
                entity.clone(),
                parse_value(value_str),
            );
            transaction.assert(relation);
        }

        // Collect result data
        let mut data = serde_json::Map::new();
        for (attr_name, value_str) in &qualified_fields {
            let short = short_attribute(&namespace, attr_name);
            data.insert(short, serde_json::json!(value_str));
        }
        results.push(serde_json::json!({
            "id": entity.to_string(),
            "data": data,
        }));
    }

    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": results.len(),
            "created": results,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Created {} {} entities", results.len(), stored_name);
        for result in &results {
            println!("  {}", result["id"].as_str().unwrap_or("???"));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Batch update
// ---------------------------------------------------------------------------

/// Update multiple entities of a concept in a single atomic commit.
///
/// Input is a JSON array of objects, where each object must include an `"id"`
/// field (the entity DID) plus the fields to update.
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
    let mut session = open_session(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = lookup_concept_by_name(&session, &ctx.space_did, &concept_name)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;

    let stored_name = ConceptName::from_stored(
        fetch_string(&session, &concept, ATTR_CONCEPT_NAME)
            .await?
            .unwrap_or_else(|| concept_name.to_string()),
    );

    let namespace = fetch_string(&session, &concept, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    let schema_attrs = fetch_string_values(&session, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut transaction = session.edit();
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
            "Item at index {}: invalid entity ID '{}'",
            idx, id_str
        ))?;

        // Verify entity exists by checking it has ALL of the concept's
        // schema attributes (structural typing — matches inner-join semantics)
        let has_all_attrs = {
            let mut all = true;
            for attr in &schema_attrs {
                if fetch_value(&session, &entity, attr).await?.is_none() {
                    all = false;
                    break;
                }
            }
            all && !schema_attrs.is_empty()
        };

        if !has_all_attrs {
            anyhow::bail!(
                "Item at index {}: entity '{}' not found or does not belong to concept '{}'",
                idx,
                id_str,
                stored_name
            );
        }

        let mut updated_fields: Vec<(String, String)> = Vec::new();

        for (key, value) in obj {
            if key == "id" {
                continue;
            }
            let value_str = json_value_to_string(value);
            let qualified = qualify_attribute(&namespace, key)?;
            if !schema_attrs.contains(&qualified) {
                anyhow::bail!(
                    "Item at index {}: attribute '{}' is not defined in concept '{}'",
                    idx,
                    key,
                    stored_name
                );
            }

            let new_value = parse_value(&value_str);
            let attr = Attribute::from_str(&qualified)?;

            // Retract all old values for this attribute (supports multi-valued)
            let old_values = fetch_values(&session, &entity, &qualified).await?;
            for old_value in old_values {
                if old_value != new_value {
                    let old_relation = Relation::new(attr.clone(), entity.clone(), old_value);
                    transaction.retract(old_relation);
                }
            }

            // Assert new value
            let new_relation = Relation::new(attr, entity.clone(), new_value);
            transaction.assert(new_relation);

            updated_fields.push((short_attribute(&namespace, &qualified), value_str));
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

    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": results.len(),
            "updated": results,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Updated {} {} entities", results.len(), stored_name);
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

/// Delete multiple entities of a concept in a single atomic commit.
///
/// Input is a JSON array of entity ID strings. All facts about each entity
/// are discovered and retracted.
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
    let mut session = open_session(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = lookup_concept_by_name(&session, &ctx.space_did, &concept_name)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;

    let stored_name = ConceptName::from_stored(
        fetch_string(&session, &concept, ATTR_CONCEPT_NAME)
            .await?
            .unwrap_or_else(|| concept_name.to_string()),
    );

    let schema_attrs = fetch_string_values(&session, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut transaction = session.edit();
    let mut deleted_ids: Vec<String> = Vec::new();

    for (idx, id_str) in ids.iter().enumerate() {
        let entity = Entity::from_str(id_str).context(format!(
            "Item at index {}: invalid entity ID '{}'",
            idx, id_str
        ))?;

        // Verify entity belongs to the specified concept by checking
        // it has ALL schema attributes (structural membership check)
        let has_all_attrs = {
            let mut all = true;
            for attr in &schema_attrs {
                if fetch_value(&session, &entity, attr).await?.is_none() {
                    all = false;
                    break;
                }
            }
            all && !schema_attrs.is_empty()
        };

        if !has_all_attrs {
            anyhow::bail!(
                "Item at index {}: entity '{}' does not belong to concept '{}'",
                idx,
                id_str,
                stored_name
            );
        }

        // Fetch all facts about this entity and retract them
        let all_facts = fetch_all_entity_facts(&session, &entity).await?;

        for artifact in &all_facts {
            let relation = Relation::new(
                artifact.the.clone(),
                artifact.of.clone(),
                artifact.is.clone(),
            );
            transaction.retract(relation);
        }

        deleted_ids.push(id_str.clone());
    }

    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "concept": stored_name.as_str(),
            "count": deleted_ids.len(),
            "deleted": deleted_ids,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted {} {} entities", deleted_ids.len(), stored_name);
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
