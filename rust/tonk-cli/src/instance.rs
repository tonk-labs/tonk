//! Instance CRUD: create, query, show, update, and delete instances.
//!
//! An instance is an entity whose attributes conform to a concept's schema.
//! Each instance stores its attribute values as EAV triples, plus metadata
//! (`instance/type` pointing to the concept, `instance/created` timestamp).

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactSelector, ArtifactStore, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Create an instance
// ---------------------------------------------------------------------------

/// Create a new instance of a concept.
///
/// `fields` are `key=value` pairs where keys are short attribute names
/// (auto-prefixed to the concept namespace).
pub async fn create(
    concept_name: String,
    fields: Vec<String>,
    file: Option<String>,
    stdin: bool,
    json: bool,
) -> Result<()> {
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    // Verify concept exists and get its schema
    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!(
            "Concept '{}' not found. Define it first with 'tonk concept define {}'.",
            concept_name, concept_name
        ))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Parse field values from args, file, or stdin
    let field_map = if let Some(path) = &file {
        parse_json_fields(
            &std::fs::read_to_string(path).context(format!("Failed to read file: {}", path))?,
        )?
    } else if stdin {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read from stdin")?;
        parse_json_fields(buf.trim())?
    } else if !fields.is_empty() {
        parse_kv_fields(&fields)?
    } else {
        anyhow::bail!("No field values provided. Pass key=value pairs, --file, or --stdin.");
    };

    if field_map.is_empty() {
        anyhow::bail!("No field values provided.");
    }

    // Qualify and validate field names against schema
    let mut qualified_fields: Vec<(String, String)> = Vec::new();
    for (key, value) in &field_map {
        let qualified = qualify_attribute(&stored_name, key)?;
        if !schema_attrs.contains(&qualified) {
            anyhow::bail!(
                "Attribute '{}' is not defined in concept '{}'. Known attributes: {}",
                key,
                stored_name,
                schema_attrs
                    .iter()
                    .map(|a| short_attribute(&stored_name, a))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        qualified_fields.push((qualified, value.clone()));
    }

    // Generate a new random entity for the instance
    let instance_entity = Entity::new().context("Failed to generate instance entity")?;

    let now = chrono::Utc::now().timestamp();

    // Build instructions
    let mut instructions = Vec::new();

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

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let mut data = serde_json::Map::new();
        for (attr_name, value_str) in &qualified_fields {
            let short = short_attribute(&stored_name, attr_name);
            data.insert(short, serde_json::json!(value_str));
        }
        let output = serde_json::json!({
            "ok": true,
            "id": instance_entity.to_string(),
            "concept": stored_name.as_str(),
            "data": data,
            "created": now,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Created {} instance: {}", stored_name, instance_entity);
        for (attr_name, value_str) in &qualified_fields {
            println!(
                "  {}: {}",
                short_attribute(&stored_name, attr_name),
                value_str
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Query instances
// ---------------------------------------------------------------------------

/// Query instances of a concept, with optional filters.
///
/// Filters are `key=value` pairs for exact matching.
pub async fn query(concept_name: String, filters: Vec<String>, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Parse filters
    let filter_map = parse_kv_fields(&filters)?;
    let mut qualified_filters: Vec<(String, Value)> = Vec::new();
    for (key, value) in &filter_map {
        let qualified = qualify_attribute(&stored_name, key)?;
        qualified_filters.push((qualified, parse_value(value)));
    }

    // Get all instance entities for this concept
    let instance_entities = fetch_entity_values(&branch, &concept, ATTR_CONCEPT_INSTANCE).await?;

    if instance_entities.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No {} instances found.", stored_name);
        }
        return Ok(());
    }

    // Hybrid query strategy:
    // If exactly one filter, use dialog's value index for fast lookup,
    // then intersect with concept instances.
    // Otherwise, enumerate all instances and filter client-side.
    let matching_entities: Vec<Entity> = if qualified_filters.len() == 1 {
        let (attr_name, filter_value) = &qualified_filters[0];
        fast_filter_by_value(&branch, attr_name, filter_value, &instance_entities).await?
    } else if qualified_filters.is_empty() {
        instance_entities.clone()
    } else {
        // Multi-filter: fetch all, then filter client-side
        let mut result = Vec::new();
        for entity in &instance_entities {
            let mut matches = true;
            for (attr_name, filter_value) in &qualified_filters {
                let val = fetch_value(&branch, entity, attr_name).await?;
                if val.as_ref() != Some(filter_value) {
                    matches = false;
                    break;
                }
            }
            if matches {
                result.push(entity.clone());
            }
        }
        result
    };

    // Fetch full data for matching instances
    let mut rows: Vec<(String, serde_json::Map<String, serde_json::Value>)> = Vec::new();

    for entity in &matching_entities {
        let mut data = serde_json::Map::new();
        for attr_name in &schema_attrs {
            if let Some(val) = fetch_value(&branch, entity, attr_name).await? {
                let short = short_attribute(&stored_name, attr_name);
                data.insert(short, value_to_json(&val));
            }
        }
        rows.push((entity.to_string(), data));
    }

    if json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, data)| {
                serde_json::json!({
                    "id": id,
                    "data": data,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&items)?);
    } else {
        if rows.is_empty() {
            println!("No matching {} instances found.", stored_name);
            return Ok(());
        }

        // Collect all attribute short names for column headers
        let short_attrs: Vec<String> = schema_attrs
            .iter()
            .map(|a| short_attribute(&stored_name, a))
            .collect();

        // Print as table
        println!("{} ({} found)\n", stored_name, rows.len());

        for (id, data) in &rows {
            println!("  id: {}", id);
            for attr in &short_attrs {
                if let Some(val) = data.get(attr) {
                    let display = match val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    println!("  {}: {}", attr, display);
                }
            }
            println!();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show instance details
// ---------------------------------------------------------------------------

/// Show full details of an instance by ID.
pub async fn show(id: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid instance ID")?;

    // Get the concept this instance belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_INSTANCE_TYPE)
        .await?
        .context(format!("Instance '{}' not found (no instance/type)", id))?;

    let concept_entity = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Instance type is not an entity reference"),
    };

    let concept_name = fetch_string(&branch, &concept_entity, ATTR_CONCEPT_NAME)
        .await?
        .ok_or_else(|| anyhow::anyhow!(
            "Concept entity '{}' is missing its 'concept/name' attribute — possible data corruption",
            concept_entity
        ))?;
    let concept_name = ConceptName::from_stored(concept_name);

    let schema_attrs =
        fetch_string_values(&branch, &concept_entity, ATTR_CONCEPT_ATTRIBUTE).await?;

    let created = fetch_value(&branch, &entity, ATTR_INSTANCE_CREATED).await?;

    // Fetch all attribute values
    let mut data = serde_json::Map::new();
    for attr_name in &schema_attrs {
        if let Some(val) = fetch_value(&branch, &entity, attr_name).await? {
            let short = short_attribute(&concept_name, attr_name);
            data.insert(short, value_to_json(&val));
        }
    }

    if json {
        let mut output = serde_json::json!({
            "id": id,
            "concept": concept_name.as_str(),
            "data": data,
        });
        if let Some(ts) = &created {
            output
                .as_object_mut()
                .unwrap()
                .insert("created".to_string(), value_to_json(ts));
        }
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{} instance: {}", concept_name, id);
        if let Some(Value::SignedInt(ts)) = &created {
            println!("  created: {}", format_ts(*ts as i64));
        }
        println!();
        for (key, val) in &data {
            let display = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            println!("  {}: {}", key, display);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Update an instance
// ---------------------------------------------------------------------------

/// Update attribute values of an existing instance.
pub async fn update(id: String, fields: Vec<String>, json: bool) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("No fields to update. Pass key=value pairs.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid instance ID")?;

    // Get the concept this instance belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_INSTANCE_TYPE)
        .await?
        .context(format!("Instance '{}' not found", id))?;

    let concept_entity_val = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Instance type is not an entity reference"),
    };

    let concept_name = fetch_string(&branch, &concept_entity_val, ATTR_CONCEPT_NAME)
        .await?
        .ok_or_else(|| anyhow::anyhow!(
            "Concept entity '{}' is missing its 'concept/name' attribute — possible data corruption",
            concept_entity_val
        ))?;
    let concept_name = ConceptName::from_stored(concept_name);

    let schema_attrs =
        fetch_string_values(&branch, &concept_entity_val, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Parse and qualify fields
    let field_map = parse_kv_fields(&fields)?;
    let mut qualified_fields: Vec<(String, String)> = Vec::new();
    for (key, value) in &field_map {
        let qualified = qualify_attribute(&concept_name, key)?;
        if !schema_attrs.contains(&qualified) {
            anyhow::bail!(
                "Attribute '{}' is not defined in concept '{}'",
                key,
                concept_name
            );
        }
        qualified_fields.push((qualified, value.clone()));
    }

    // Build instructions: for each field, retract old value (if any) and assert new
    let mut instructions = Vec::new();
    let mut updated = Vec::new();

    for (attr_name, value_str) in &qualified_fields {
        let new_value = parse_value(value_str);

        // Retract old value if it exists
        if let Some(old_value) = fetch_value(&branch, &entity, attr_name).await?
            && old_value != new_value
        {
            instructions.push(Instruction::Retract(Artifact {
                the: Attribute::from_str(attr_name)?,
                of: entity.clone(),
                is: old_value,
                cause: None,
            }));
        }

        // Assert new value
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(attr_name)?,
            of: entity.clone(),
            is: new_value,
            cause: None,
        }));

        updated.push((short_attribute(&concept_name, attr_name), value_str.clone()));
    }

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "id": id,
            "updated": updated.iter().map(|(k, v)| serde_json::json!({k: v})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Updated {} instance: {}", concept_name, id);
        for (key, val) in &updated {
            println!("  {}: {}", key, val);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delete an instance
// ---------------------------------------------------------------------------

/// Delete an instance by ID.
pub async fn delete(id: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid instance ID")?;

    // Get the concept this instance belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_INSTANCE_TYPE)
        .await?
        .context(format!("Instance '{}' not found", id))?;

    let concept_entity_val = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Instance type is not an entity reference"),
    };

    let concept_name = fetch_string(&branch, &concept_entity_val, ATTR_CONCEPT_NAME)
        .await?
        .ok_or_else(|| anyhow::anyhow!(
            "Concept entity '{}' is missing its 'concept/name' attribute — possible data corruption",
            concept_entity_val
        ))?;
    let concept_name = ConceptName::from_stored(concept_name);

    let schema_attrs =
        fetch_string_values(&branch, &concept_entity_val, ATTR_CONCEPT_ATTRIBUTE).await?;

    let mut instructions = Vec::new();

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
        is: Value::Entity(concept_entity_val.clone()),
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
        of: concept_entity_val.clone(),
        is: Value::Entity(entity.clone()),
        cause: None,
    }));

    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "id": id,
            "concept": concept_name.as_str(),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Deleted {} instance: {}", concept_name, id);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `key=value` pairs from a list of strings.
fn parse_kv_fields(fields: &[String]) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for field in fields {
        let (key, value) = field.split_once('=').context(format!(
            "Invalid field '{}'. Expected key=value format.",
            field
        ))?;
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if key.is_empty() {
            anyhow::bail!("Empty key in field '{}'", field);
        }
        result.push((key, value));
    }
    Ok(result)
}

/// Parse fields from a JSON string (object with string keys).
fn parse_json_fields(input: &str) -> Result<Vec<(String, String)>> {
    let obj: serde_json::Value = serde_json::from_str(input).context("Invalid JSON input")?;

    let map = obj.as_object().context("JSON input must be an object")?;

    let mut result = Vec::new();
    for (key, value) in map {
        let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => serde_json::to_string(value)?,
        };
        result.push((key.clone(), value_str));
    }
    Ok(result)
}

/// Fast path: use dialog's value index to find entities matching a single
/// attribute+value filter, then intersect with the known instance set.
async fn fast_filter_by_value<S: ArtifactStore>(
    store: &S,
    attr_name: &str,
    filter_value: &Value,
    instance_set: &[Entity],
) -> Result<Vec<Entity>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;

    // Query by attribute + value to get all matching entities
    let results: Vec<_> = store
        .select(ArtifactSelector::new().the(attr).is(filter_value.clone()))
        .try_collect()
        .await?;

    let matching_entities: Vec<Entity> = results.into_iter().map(|a| a.of).collect();

    // Intersect with instance set
    Ok(matching_entities
        .into_iter()
        .filter(|e| instance_set.contains(e))
        .collect())
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
