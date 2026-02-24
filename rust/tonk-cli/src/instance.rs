//! Instance CRUD: create, query, show, update, and delete instances.
//!
//! An instance is an entity whose attributes conform to a concept's schema.
//! Each instance stores its attribute values as EAV triples, plus metadata
//! (`instance/type` pointing to the concept, `instance/created` timestamp).

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Term, Value};
use std::io::Read;
use std::str::FromStr;

/// A schema attribute variable used during concept querying.
struct AttrVar {
    /// Fully qualified attribute name (e.g., `"person/name"`).
    #[allow(dead_code)]
    attribute_name: String,
    /// Short/unqualified name used as the query parameter key (e.g., `"name"`).
    param_name: String,
    /// The query term variable bound to this attribute.
    term: Term<Value>,
}

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

/// Query instances of a concept, with optional selectors.
///
/// Selectors are `key=value` pairs for exact matching.
///
/// Uses dialog-db's Session-based concept querying which automatically
/// merges stored instances with rule-derived instances.
pub async fn query(concept_name: String, selectors: Vec<String>, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = concept_entity(&ctx.space_did, &concept_name)?;

    let stored_name = fetch_string(&branch, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let schema_attrs = fetch_string_values(&branch, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;

    // Parse selectors
    let selector_map = parse_kv_fields(&selectors)?;
    let mut qualified_selectors: Vec<(String, Value)> = Vec::new();
    for (key, value) in &selector_map {
        let qualified = qualify_attribute(&stored_name, key)?;
        qualified_selectors.push((qualified, parse_value(value)));
    }

    // Load any rules targeting this concept (may be empty)
    let rule_defs =
        crate::rule::load_rules_for_concept(&branch, &ctx.space_did, &stored_name).await?;

    // Use Session-based querying which handles both stored instances
    // and rule-derived instances in a single code path.
    let rows = query_with_rules(
        branch,
        &schema_attrs,
        &stored_name,
        &qualified_selectors,
        &rule_defs,
    )
    .await?;

    display_rows(&rows, &stored_name, &schema_attrs, json);
    Ok(())
}

/// Session-based query with rules. Compiles rules into DeductiveRules,
/// registers them with a Session, and uses dialog-db's concept application
/// to merge stored and derived instances.
async fn query_with_rules(
    branch: dialog_artifacts::repository::Branch<tonk_space::FsBackend>,
    schema_attrs: &[String],
    stored_name: &ConceptName,
    qualified_selectors: &[(String, Value)],
    rule_defs: &[crate::rule::RuleDefinition],
) -> Result<Vec<(String, serde_json::Map<String, serde_json::Value>)>> {
    use dialog_query::{Parameters, Session};
    use futures_util::TryStreamExt;

    // Compile rules
    let compiled_rules: Vec<dialog_query::DeductiveRule> = rule_defs
        .iter()
        .map(|def| crate::rule::compile_rule(def, stored_name, schema_attrs))
        .collect::<Result<Vec<_>>>()?;

    // Open a Session and register rules
    let mut session = Session::open(branch);
    for rule in compiled_rules {
        session = session.register(rule);
    }

    // Build the dynamic concept
    let dynamic_concept = build_dynamic_concept(schema_attrs)?;

    // Build query parameters: named variables for each attribute
    // plus "this" for the entity
    let mut params = Parameters::new();
    let this_var: Term<Value> = Term::var("this");
    params.insert("this".to_string(), this_var.clone());

    // Create a variable for each attribute
    let mut attr_vars: Vec<AttrVar> = Vec::new();
    for attr in schema_attrs {
        let param_name = short_attribute(stored_name, attr);
        let term: Term<Value> = Term::var(param_name.as_str());
        params.insert(param_name.clone(), term.clone());
        attr_vars.push(AttrVar {
            attribute_name: attr.clone(),
            param_name,
            term,
        });
    }

    // Apply selectors as constant terms
    for (attr_name, selector_value) in qualified_selectors {
        let short = short_attribute(stored_name, attr_name);
        params.insert(short, Term::Constant(selector_value.clone()));
    }

    // Execute the concept query
    let application = dynamic_concept
        .apply(params)
        .map_err(|e| anyhow::anyhow!("Failed to apply concept query: {}", e))?;

    let answers: Vec<dialog_query::Answer> = application
        .query(&session)
        .try_collect()
        .await
        .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;

    // Convert answers to rows
    let mut rows = Vec::new();
    for answer in &answers {
        let mut data = serde_json::Map::new();

        // Resolve the entity (this)
        let entity_str = match answer.resolve(&this_var) {
            Ok(Value::Entity(e)) => e.to_string(),
            Ok(v) => format_value(&v),
            Err(e) => {
                eprintln!("Warning: could not resolve entity for query answer: {}", e);
                "???".to_string()
            }
        };

        // Resolve each attribute value
        for attr_var in &attr_vars {
            match answer.resolve(&attr_var.term) {
                Ok(val) => {
                    data.insert(attr_var.param_name.clone(), value_to_json(&val));
                }
                Err(e) => {
                    eprintln!(
                        "Warning: could not resolve attribute '{}': {}",
                        attr_var.param_name, e
                    );
                    data.insert(attr_var.param_name.clone(), serde_json::Value::Null);
                }
            }
        }

        rows.push((entity_str, data));
    }

    Ok(rows)
}

/// Display query result rows.
fn display_rows(
    rows: &[(String, serde_json::Map<String, serde_json::Value>)],
    stored_name: &ConceptName,
    schema_attrs: &[String],
    json: bool,
) {
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
        match serde_json::to_string(&items) {
            Ok(s) => println!("{}", s),
            Err(e) => {
                eprintln!("Error: failed to serialize query results: {}", e);
                println!("[]");
            }
        }
    } else {
        if rows.is_empty() {
            println!("No matching {} instances found.", stored_name);
            return;
        }

        let short_attrs: Vec<String> = schema_attrs
            .iter()
            .map(|a| short_attribute(stored_name, a))
            .collect();

        println!("{} ({} found)\n", stored_name, rows.len());

        for (id, data) in rows {
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

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
