//! Entity CRUD: create, query, show, assert, and retract entities.
//!
//! An entity is a data record whose attributes conform to a concept's schema.
//! Each entity stores its attribute values as EAV triples, plus metadata
//! (`entity/type` pointing to the concept, `entity/created` timestamp).

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactSelector, ArtifactStore, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Create an entity
// ---------------------------------------------------------------------------

/// Create a new entity
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

    // Generate a new random entity ID
    let entity = derive_entity_from_fields(&qualified_fields)?;

    let now = chrono::Utc::now().timestamp();

    // Build instructions
    let mut instructions = Vec::new();

    // Attribute values
    for (attr_name, value_str) in &qualified_fields {
        instructions.push(Instruction::Assert(Artifact {
            the: Attribute::from_str(attr_name)?,
            of: entity.clone(),
            is: parse_value(value_str),
            cause: None,
        }));
    }

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
            "id": entity.to_string(),
            "concept": stored_name.as_str(),
            "data": data,
            "created": now,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Created {} entity: {}", stored_name, entity);
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

/// Derive an entity from field content
///
/// `fields` are `key=value` pairs where keys are short attribute names
pub fn derive_entity_from_fields(fields: &[(String, String)]) -> Result<Entity> {
    // Sort fields by attribute name for deterministic ordering
    let mut sorted = fields.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    // blake3 hash concantedated key=value pairs
    let mut hasher = blake3::Hasher::new();
    for (attr, value) in &sorted {
        hasher.update(attr.as_bytes());
        hasher.update(b"\0");
        hasher.update(value.as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.finalize();

    // Use hash as Ed25519 signing key seed
    let signing_key = ed25519_dalek::SigningKey::from_bytes(hash.as_bytes());
    let verifying_key = signing_key.verifying_key();

    // Format as did:key
    const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];
    let mut multicodec_key = [0u8; 34];
    multicodec_key[..2].copy_from_slice(&ED25519_MULTICODEC);
    multicodec_key[2..].copy_from_slice(verifying_key.as_bytes());
    let encoded = bs58::encode(&multicodec_key).into_string();
    let url = format!("did:key:z{}", encoded);

    Entity::from_str(&url).context("Failed to derive entity from fields")
}

// ---------------------------------------------------------------------------
// Query entities
// ---------------------------------------------------------------------------

/// Query entities of a concept, with optional filters.
///
/// Filters are `key=value` pairs for exact matching.
///
/// When rules exist for the concept, uses dialog-db's Session-based
/// concept querying which automatically merges stored entities with
/// rule-derived entities. Falls back to direct enumeration when no
/// rules apply.
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

    // Load ALL rules in the space (not just rules for the queried concept),
    // because rules can depend on each other.
    let all_rules = crate::rule::load_all_rules(&branch, &ctx.space_did).await?;

    // Check if any rule targets the queried concept
    let has_rules_for_concept = all_rules
        .iter()
        .any(|(conclusion, _)| conclusion == stored_name.as_str());

    let rows = if has_rules_for_concept {
        // Rules exist for this concept — compile ALL rules (each against its
        // own conclusion concept's schema) and register them with the Session.
        let mut compiled_rules: Vec<dialog_query::DeductiveRule> = Vec::new();
        for (conclusion_name, def) in &all_rules {
            let cname = ConceptName::from_stored(conclusion_name.clone());
            let concept_ent = concept_entity(&ctx.space_did, &cname)?;
            let concept_attrs =
                fetch_string_values(&branch, &concept_ent, ATTR_CONCEPT_ATTRIBUTE).await?;
            let compiled = crate::rule::compile_rule(def, &cname, &concept_attrs)?;
            compiled_rules.push(compiled);
        }

        query_with_rules(
            branch,
            &schema_attrs,
            &stored_name,
            &qualified_filters,
            &compiled_rules,
        )
        .await?
    } else {
        // No rules — use direct enumeration (existing fast path)
        query_direct(
            &branch,
            &concept,
            &schema_attrs,
            &stored_name,
            &qualified_filters,
        )
        .await?
    };

    display_rows(&rows, &stored_name, &schema_attrs, json);
    Ok(())
}

/// Direct enumeration query (no rules). Uses the concept/entity
/// back-references and hybrid filtering strategy.
async fn query_direct<S: ArtifactStore>(
    store: &S,
    concept: &Entity,
    schema_attrs: &[String],
    stored_name: &ConceptName,
    qualified_filters: &[(String, Value)],
) -> Result<Vec<(String, serde_json::Map<String, serde_json::Value>)>> {
    let entities = fetch_entity_values(store, concept, ATTR_CONCEPT_ENTITY).await?;

    if entities.is_empty() {
        return Ok(Vec::new());
    }

    // Hybrid query strategy
    let matching_entities: Vec<Entity> = if qualified_filters.len() == 1 {
        let (attr_name, filter_value) = &qualified_filters[0];
        fast_filter_by_value(store, attr_name, filter_value, &entities).await?
    } else if qualified_filters.is_empty() {
        entities.clone()
    } else {
        let mut result = Vec::new();
        for entity in &entities {
            let mut matches = true;
            for (attr_name, filter_value) in qualified_filters {
                let val = fetch_value(store, entity, attr_name).await?;
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

    let mut rows = Vec::new();
    for entity in &matching_entities {
        let mut data = serde_json::Map::new();
        for attr_name in schema_attrs {
            if let Some(val) = fetch_value(store, entity, attr_name).await? {
                let short = short_attribute(stored_name, attr_name);
                data.insert(short, value_to_json(&val));
            }
        }
        rows.push((entity.to_string(), data));
    }

    Ok(rows)
}

/// Session-based query with rules. Registers pre-compiled DeductiveRules
/// with a Session and uses dialog-db's concept application to merge
/// stored and derived entities.
async fn query_with_rules(
    branch: dialog_artifacts::replica::Branch<
        tonk_space::FsBackend,
        dialog_artifacts::replica::SigningAuthority,
    >,
    schema_attrs: &[String],
    stored_name: &ConceptName,
    qualified_filters: &[(String, Value)],
    compiled_rules: &[dialog_query::DeductiveRule],
) -> Result<Vec<(String, serde_json::Map<String, serde_json::Value>)>> {
    use dialog_query::{Parameters, Session, Term};
    use futures_util::TryStreamExt;

    // Open a Session and register all rules
    let mut session = Session::open(branch);
    for rule in compiled_rules {
        session = session.register(rule.clone());
    }

    // Build the dynamic concept
    let dynamic_concept = build_dynamic_concept(schema_attrs)?;

    // Build query parameters: named variables for each attribute
    // plus "this" for the entity
    let mut params = Parameters::new();
    let this_var: Term<Value> = Term::var("this");
    params.insert("this".to_string(), this_var.clone());

    // Create a variable for each attribute
    let mut attr_vars: Vec<(String, String, Term<Value>)> = Vec::new(); // (qualified, short, term)
    for attr in schema_attrs {
        let short = short_attribute(stored_name, attr);
        let var: Term<Value> = Term::var(short.as_str());
        params.insert(short.clone(), var.clone());
        attr_vars.push((attr.clone(), short, var));
    }

    // Apply filters as constant terms
    for (attr_name, filter_value) in qualified_filters {
        let short = short_attribute(stored_name, attr_name);
        params.insert(short, Term::Constant(filter_value.clone()));
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

    // Convert answers to rows, deduplicating by entity ID.
    // Rules involving multi-valued attributes (like ingredient) can produce
    // duplicate rows for the same derived entity — we keep only the first.
    let mut seen_entities = std::collections::HashSet::new();
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

        // Deduplicate by entity ID
        if !seen_entities.insert(entity_str.clone()) {
            continue;
        }

        // Resolve each attribute value
        for (_qualified, short, var) in &attr_vars {
            match answer.resolve(var) {
                Ok(val) => {
                    data.insert(short.clone(), value_to_json(&val));
                }
                Err(e) => {
                    eprintln!("Warning: could not resolve attribute '{}': {}", short, e);
                    data.insert(short.clone(), serde_json::Value::Null);
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
            println!("No matching {} entities found.", stored_name);
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
// Show entity details
// ---------------------------------------------------------------------------

/// Show full details of an entity by ID.
pub async fn show(id: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Get the concept this entity belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_ENTITY_TYPE)
        .await?
        .context(format!("Entity '{}' not found (no entity/type)", id))?;

    let concept_entity = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Entity type is not an entity reference"),
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

    let created = fetch_value(&branch, &entity, ATTR_ENTITY_CREATED).await?;

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
        println!("{} entity: {}", concept_name, id);
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
// Assert an entity
// ---------------------------------------------------------------------------

/// Assert new attribute values on an existing entity.
pub async fn assert(id: String, fields: Vec<String>, json: bool) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("No fields to assert. Pass key=value pairs.");
    }

    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Get the concept this entity belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_ENTITY_TYPE)
        .await?
        .context(format!("Entity '{}' not found", id))?;

    let concept_entity_val = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Entity type is not an entity reference"),
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
        println!("Updated {} entity: {}", concept_name, id);
        for (key, val) in &updated {
            println!("  {}: {}", key, val);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Retract an entity
// ---------------------------------------------------------------------------

/// Retract an entity by ID.
pub async fn retract(id: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut branch = open_branch(&ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Get the concept this entity belongs to
    let concept_val = fetch_value(&branch, &entity, ATTR_ENTITY_TYPE)
        .await?
        .context(format!("Entity '{}' not found", id))?;

    let concept_entity_val = match concept_val {
        Value::Entity(e) => e,
        _ => anyhow::bail!("Entity type is not an entity reference"),
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

    // Retract entity/type
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_ENTITY_TYPE)?,
        of: entity.clone(),
        is: Value::Entity(concept_entity_val.clone()),
        cause: None,
    }));

    // Retract entity/created
    if let Some(ts) = fetch_value(&branch, &entity, ATTR_ENTITY_CREATED).await? {
        instructions.push(Instruction::Retract(Artifact {
            the: Attribute::from_str(ATTR_ENTITY_CREATED)?,
            of: entity.clone(),
            is: ts,
            cause: None,
        }));
    }

    // Retract concept/entity back-reference
    instructions.push(Instruction::Retract(Artifact {
        the: Attribute::from_str(ATTR_CONCEPT_ENTITY)?,
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
        println!("Retracted {} entity: {}", concept_name, id);
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
/// attribute+value filter, then intersect with the known entity set.
async fn fast_filter_by_value<S: ArtifactStore>(
    store: &S,
    attr_name: &str,
    filter_value: &Value,
    entity_set: &[Entity],
) -> Result<Vec<Entity>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;

    // Query by attribute + value to get all matching entities
    let results: Vec<_> = store
        .select(ArtifactSelector::new().the(attr).is(filter_value.clone()))
        .try_collect()
        .await?;

    let matching_entities: Vec<Entity> = results.into_iter().map(|a| a.of).collect();

    // Intersect with entity set
    Ok(matching_entities
        .into_iter()
        .filter(|e| entity_set.contains(e))
        .collect())
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
