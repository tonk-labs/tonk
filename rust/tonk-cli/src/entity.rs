//! Entity CRUD: create, query, show, assert, and retract entities.
//!
//! An entity is a data record identified by a deterministic `did:key` derived
//! from its initial field values. Concept membership is structural — an entity
//! belongs to a concept if it has facts for that concept's attributes, matching
//! dialog-db's query-time duck typing model.

use crate::schema::*;
use anyhow::{Context, Result};
use dialog_query::claim::{Attribute, Relation};
use dialog_query::{Entity, Session, Value};
use futures_util::TryStreamExt;
use std::io::Read;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Create an entity
// ---------------------------------------------------------------------------

/// Create a new entity.
///
/// The entity ID is deterministically derived from the field values.
/// `fields` are `key=value` pairs where keys are short attribute names
/// (auto-prefixed to the concept namespace).
pub async fn create(
    ctx: &SpaceContext,
    concept_name: String,
    fields: Vec<String>,
    file: Option<String>,
    stdin: bool,
    json: bool,
) -> Result<()> {
    let mut session = open_session(ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = lookup_concept_by_name(&session, &concept_name)
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
        let qualified = qualify_attribute(&namespace, key)?;
        if !schema_attrs.contains(&qualified) {
            anyhow::bail!(
                "Attribute '{}' is not defined in concept '{}'. Known attributes: {}",
                key,
                stored_name,
                schema_attrs
                    .iter()
                    .map(|a| short_attribute(&namespace, a))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        qualified_fields.push((qualified, value.clone()));
    }

    // Derive entity ID deterministically from field content
    let entity = derive_entity_from_fields(&qualified_fields)?;

    // Build and commit transaction
    let mut transaction = session.edit();
    for (attr_name, value_str) in &qualified_fields {
        let relation = Relation::new(
            Attribute::from_str(attr_name)?,
            entity.clone(),
            parse_value(value_str),
        );
        transaction.assert(relation);
    }
    session.commit(transaction).await?;

    if json {
        let mut data = serde_json::Map::new();
        for (attr_name, value_str) in &qualified_fields {
            let short = short_attribute(&namespace, attr_name);
            data.insert(short, serde_json::json!(value_str));
        }
        let output = serde_json::json!({
            "ok": true,
            "id": entity.to_string(),
            "concept": stored_name.as_str(),
            "data": data,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Created {} entity: {}", stored_name, entity);
        for (attr_name, value_str) in &qualified_fields {
            println!(
                "  {}: {}",
                short_attribute(&namespace, attr_name),
                value_str
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Query entities
// ---------------------------------------------------------------------------

/// Query entities of a concept, with optional filters.
///
/// Uses dialog-db's Session-based concept querying which performs a
/// structural join over the concept's attributes. When rules exist,
/// they are registered with the Session to merge stored and derived entities.
pub async fn query(
    ctx: &SpaceContext,
    concept_name: String,
    filters: Vec<String>,
    json: bool,
) -> Result<()> {
    let session = open_session(ctx).await?;

    let concept_name = ConceptName::new(concept_name)?;
    let concept = lookup_concept_by_name(&session, &concept_name)
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

    // Parse filters
    let filter_map = parse_kv_fields(&filters)?;
    let mut qualified_filters: Vec<(String, Value)> = Vec::new();
    for (key, value) in &filter_map {
        let qualified = qualify_attribute(&namespace, key)?;
        qualified_filters.push((qualified, parse_value(value)));
    }

    // Load ALL rules in the space (not just rules for the queried concept),
    // because rules can depend on each other.
    let all_rules = crate::rule::load_all_rules(&session).await?;

    // Compile all rules
    // Fetch cardinalities for the queried concept
    let cardinalities = fetch_attribute_cardinalities(&session, &schema_attrs).await?;

    let mut compiled_rules: Vec<dialog_query::DeductiveRule> = Vec::new();
    for (conclusion_name, def) in &all_rules {
        let cname = ConceptName::from_stored(conclusion_name.clone());
        let concept_ent = lookup_concept_by_name(&session, &cname)
            .await?
            .context(format!("Rule conclusion concept '{}' not found", cname))?;
        let concept_attrs =
            fetch_string_values(&session, &concept_ent, ATTR_CONCEPT_ATTRIBUTE).await?;
        let rule_ns = fetch_string(&session, &concept_ent, ATTR_CONCEPT_NAMESPACE)
            .await?
            .unwrap_or_else(|| ctx.space_name.clone());
        let rule_cardinalities = fetch_attribute_cardinalities(&session, &concept_attrs).await?;
        let compiled =
            crate::rule::compile_rule(def, &cname, &concept_attrs, &rule_cardinalities, &rule_ns)?;
        compiled_rules.push(compiled);
    }

    // Always use Session-based concept query (structural matching via joins)
    let rows = query_concept(
        session,
        &schema_attrs,
        &namespace,
        &qualified_filters,
        &compiled_rules,
        &cardinalities,
    )
    .await?;

    display_rows(&rows, &stored_name, &schema_attrs, &namespace, json);
    Ok(())
}

/// Session-based concept query. Uses dialog-db's structural matching:
/// builds a dynamic Concept from the schema attributes and queries via
/// the Session, which performs a multi-way join over the AEV/EAV indexes.
///
/// Any registered DeductiveRules are also evaluated, merging stored and
/// rule-derived entities with OR semantics.
async fn query_concept(
    mut session: Session<
        dialog_artifacts::replica::Branch<
            tonk_space::FsBackend,
            dialog_artifacts::replica::SigningAuthority,
        >,
    >,
    schema_attrs: &[String],
    namespace: &str,
    qualified_filters: &[(String, Value)],
    compiled_rules: &[dialog_query::DeductiveRule],
    cardinalities: &std::collections::HashMap<String, dialog_query::Cardinality>,
) -> Result<Vec<(String, serde_json::Map<String, serde_json::Value>)>> {
    use dialog_query::{Parameters, Term};

    // Register all rules with the Session
    for rule in compiled_rules {
        session = session.register(rule.clone());
    }

    // Build the dynamic concept from schema attributes (with correct cardinality)
    let dynamic_concept = build_dynamic_concept(schema_attrs, cardinalities)?;

    // Build query parameters: named variables for each attribute
    // plus "this" for the entity
    let mut params = Parameters::new();
    let this_var: Term<Value> = Term::var("this");
    params.insert("this".to_string(), this_var.clone());

    // Create a variable for each attribute
    let mut attr_vars: Vec<(String, String, Term<Value>)> = Vec::new(); // (qualified, short, term)
    for attr in schema_attrs {
        let short = short_attribute(namespace, attr);
        let var: Term<Value> = Term::var(short.as_str());
        params.insert(short.clone(), var.clone());
        attr_vars.push((attr.clone(), short, var));
    }

    // Apply filters as constant terms
    for (attr_name, filter_value) in qualified_filters {
        let short = short_attribute(namespace, attr_name);
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
    namespace: &str,
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
            .map(|a| short_attribute(namespace, a))
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
///
/// Infers the entity's concept by examining its attributes against
/// registered concepts.
pub async fn show(ctx: &SpaceContext, id: String, json: bool) -> Result<()> {
    let session = open_session(ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Infer concept from the entity's attributes
    let (concept_name, concept_ent, schema_attrs) =
        infer_concept_from_entity(&session, &entity).await?;

    let namespace = fetch_string(&session, &concept_ent, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    // Fetch all attribute values (supporting multi-valued attributes)
    let mut data = serde_json::Map::new();
    for attr_name in &schema_attrs {
        let values = fetch_values(&session, &entity, attr_name).await?;
        if !values.is_empty() {
            let short = short_attribute(&namespace, attr_name);
            if values.len() == 1 {
                data.insert(short, value_to_json(&values[0]));
            } else {
                let json_arr: Vec<serde_json::Value> = values.iter().map(value_to_json).collect();
                data.insert(short, serde_json::Value::Array(json_arr));
            }
        }
    }

    if json {
        let output = serde_json::json!({
            "id": id,
            "concept": concept_name.as_str(),
            "data": data,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{} entity: {}", concept_name, id);
        println!();
        for (key, val) in &data {
            match val {
                serde_json::Value::Array(arr) => {
                    println!("  {}:", key);
                    for item in arr {
                        let display = match item {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        println!("    - {}", display);
                    }
                }
                serde_json::Value::String(s) => {
                    println!("  {}: {}", key, s);
                }
                other => {
                    println!("  {}: {}", key, other);
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Assert an entity
// ---------------------------------------------------------------------------

/// Assert new attribute values on an existing entity.
///
/// Infers the entity's concept by examining its attribute namespaces,
/// then validates and applies the field updates.
pub async fn assert(ctx: &SpaceContext, id: String, fields: Vec<String>, json: bool) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("No fields to assert. Pass key=value pairs.");
    }

    let mut session = open_session(ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Infer concept from the entity's attributes
    let (concept_name, concept_ent, schema_attrs) =
        infer_concept_from_entity(&session, &entity).await?;

    let namespace = fetch_string(&session, &concept_ent, ATTR_CONCEPT_NAMESPACE)
        .await?
        .unwrap_or_else(|| ctx.space_name.clone());

    // Parse and qualify fields
    let field_map = parse_kv_fields(&fields)?;
    let mut qualified_fields: Vec<(String, String)> = Vec::new();
    for (key, value) in &field_map {
        let qualified = qualify_attribute(&namespace, key)?;
        if !schema_attrs.contains(&qualified) {
            anyhow::bail!(
                "Attribute '{}' is not defined in concept '{}'",
                key,
                concept_name
            );
        }
        qualified_fields.push((qualified, value.clone()));
    }

    // Build transaction: for each field, retract old value (if any) and assert new
    let mut transaction = session.edit();
    let mut updated = Vec::new();

    for (attr_name, value_str) in &qualified_fields {
        let new_value = parse_value(value_str);
        let attr = Attribute::from_str(attr_name)?;

        // Retract all old values for this attribute (supports multi-valued)
        let old_values = fetch_values(&session, &entity, attr_name).await?;
        for old_value in old_values {
            if old_value != new_value {
                let old_relation = Relation::new(attr.clone(), entity.clone(), old_value);
                transaction.retract(old_relation);
            }
        }

        // Assert new value
        let new_relation = Relation::new(attr, entity.clone(), new_value);
        transaction.assert(new_relation);

        updated.push((short_attribute(&namespace, attr_name), value_str.clone()));
    }

    session.commit(transaction).await?;

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
///
/// Discovers all facts about the entity and retracts them, regardless of
/// which concept they belong to. This is more robust than the old approach
/// of only retracting known schema attributes.
pub async fn retract(ctx: &SpaceContext, id: String, json: bool) -> Result<()> {
    let mut session = open_session(ctx).await?;

    let entity = Entity::from_str(&id).context("Invalid entity ID")?;

    // Fetch all facts about this entity
    let all_facts = fetch_all_entity_facts(&session, &entity).await?;

    if all_facts.is_empty() {
        anyhow::bail!("Entity '{}' not found (no facts)", id);
    }

    // Try to infer the concept name for display purposes
    let concept_label = match infer_concept_from_entity(&session, &entity).await {
        Ok((name, _, _)) => name.to_string(),
        Err(_) => "unknown".to_string(),
    };

    // Build transaction: retract every fact about this entity
    let mut transaction = session.edit();
    for artifact in &all_facts {
        let relation = Relation::new(
            artifact.the.clone(),
            artifact.of.clone(),
            artifact.is.clone(),
        );
        transaction.retract(relation);
    }

    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "id": id,
            "concept": concept_label,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Retracted {} entity: {}", concept_label, id);
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
