//! `carry assert` -- assert claims on entities.
//!
//! Supports domain targets, concept targets (both builtin and user-defined),
//! file input, and stdin.
//!
//! ## Concept assertion sugar
//!
//! Builtin concepts (`attribute`, `concept`, `bookmark`) have special handling:
//! - Fields are mapped through the builtin schema (e.g. `the` -> `dialog.attribute/id`)
//! - Variable-keyed fields (e.g. `with.name=...`) are expanded
//! - `@name` asserts `dialog.meta/name` on the entity
//!
//! User-defined concepts resolve via `dialog.meta/name` -> concept entity ->
//! `dialog.concept.with/*` claims -> attribute selectors.

use crate::schema;
use crate::site::Site;
use crate::target::{Field, FirstArg, Target};
use anyhow::{Context, Result};
use dialog_query::Value;
use std::collections::BTreeMap;
use std::slice::from_ref;
use std::str::FromStr;

/// Execute `carry assert <TARGET>|<FILE>|- [this=<ENTITY>] [@name] [FIELD=VALUE...]`.
///
/// Returns the DID of the asserted entity. For file/stdin input that may
/// touch multiple entities, returns the last entity asserted.
pub async fn execute(
    site: &Site,
    first_arg: FirstArg,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<String> {
    match first_arg {
        FirstArg::Stdin => {
            assert_from_stdin(site, format).await?;
            Ok(String::new())
        }
        FirstArg::File(path) => {
            assert_from_file(site, &path, format).await?;
            Ok(String::new())
        }
        FirstArg::Target(target) => {
            assert_with_target(site, target, this_entity, entity_name, fields, format).await
        }
    }
}

/// Assert claims from a target + fields.
async fn assert_with_target(
    site: &Site,
    target: Target,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<String> {
    if fields.is_empty() {
        anyhow::bail!("At least one FIELD=VALUE pair is required for assert");
    }

    match target {
        Target::Domain(ref domain) => {
            assert_domain(site, domain, this_entity, entity_name, &fields, format).await
        }
        Target::Concept(ref concept_name) => {
            // Check if it's a builtin concept first
            if let Some(builtin) = schema::lookup_builtin(concept_name) {
                assert_builtin_concept(site, builtin, this_entity, entity_name, &fields, format)
                    .await
            } else {
                assert_user_concept(
                    site,
                    concept_name,
                    this_entity,
                    entity_name,
                    &fields,
                    format,
                )
                .await
            }
        }
    }
}

/// Build retract statements for existing values of a cardinality-one attribute.
///
/// If the attribute's cardinality is `"one"` (the default) and the entity already
/// has values for it, returns retract statements for each existing value so
/// they can be committed alongside the new assert.
///
/// Returns an empty vec when:
/// - The attribute has `cardinality: many`
/// - The entity has no existing values for the attribute
async fn retract_cardinality_one_values(
    site: &Site,
    entity: &dialog_query::Entity,
    attr: &crate::schema::ClaimAttribute,
    attr_selector: &str,
) -> Result<Vec<schema::AttributeStatement>> {
    let cardinality =
        schema::fetch_attribute_cardinality(&site.branch, &site.operator, attr_selector).await?;
    if cardinality == "many" {
        return Ok(Vec::new());
    }

    let existing = schema::fetch_values(&site.branch, &site.operator, entity, attr.clone()).await?;
    existing
        .into_iter()
        .map(|value| schema::make_statement(&attr.to_string(), entity.clone(), value))
        .collect()
}

/// Assert claims using a domain target (open-ended, no schema).
async fn assert_domain(
    site: &Site,
    domain: &str,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<String> {
    // All fields must have values for assert
    for f in fields {
        if f.value.is_none() {
            anyhow::bail!(
                "Field '{}' requires a value (use {}=<VALUE>)",
                f.name,
                f.name
            );
        }
    }

    let is_update = this_entity.is_some();

    let entity = if is_update {
        resolve_entity(this_entity.as_ref().unwrap())?
    } else {
        let field_pairs: Vec<(String, String)> = fields
            .iter()
            .map(|f| {
                (
                    f.qualified_name(domain),
                    f.value.clone().unwrap_or_default(),
                )
            })
            .collect();
        schema::derive_entity_from_fields(&field_pairs)?
    };

    let mut tx = site.branch.transaction();

    if is_update {
        // Retract existing cardinality-one values before asserting new ones
        for f in fields {
            let attr_name = f.qualified_name(domain);
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::parse_claim_attribute(&attr_name)?,
                &attr_name,
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
    }

    for f in fields {
        let attr_name = f.qualified_name(domain);
        let value = schema::parse_value(f.value.as_deref().unwrap());
        tx = tx.assert(schema::make_statement(&attr_name, entity.clone(), value)?);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        if is_update {
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::dialog_meta::Name::the().into(),
                "dialog.meta/name",
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
        tx = tx.assert(schema::dialog_meta::Name::of(entity.clone()).is(name.clone()));
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    print_assert_result(&entity, fields.len(), entity_name.as_deref(), format);
    Ok(entity.to_string())
}

/// Assert claims using a builtin concept target (attribute, concept, bookmark).
async fn assert_builtin_concept(
    site: &Site,
    builtin: &schema::BuiltinConceptSchema,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<String> {
    // All fields must have values
    for f in fields {
        if f.value.is_none() {
            anyhow::bail!(
                "Field '{}' requires a value (use {}=<VALUE>)",
                f.name,
                f.name
            );
        }
    }

    // Apply defaults for attribute assertions: cardinality defaults to "one"
    let mut fields_with_defaults;
    let effective_fields: &[Field] = if builtin.name == "attribute" {
        fields_with_defaults = fields.to_vec();
        if !fields_with_defaults.iter().any(|f| f.name == "cardinality") {
            fields_with_defaults.push(Field {
                name: "cardinality".to_string(),
                value: Some("one".to_string()),
            });
        }
        &fields_with_defaults
    } else {
        fields
    };

    // Resolve each CLI field to its relation via the builtin schema
    let mut resolved_claims: Vec<(String, Value)> = Vec::new();
    let mut concept_with_fields: BTreeMap<String, dialog_query::Entity> = BTreeMap::new();

    for f in effective_fields {
        let value_str = f.value.as_deref().unwrap();

        if let Some((relation, is_variable)) = schema::resolve_builtin_field(builtin, &f.name) {
            if is_variable && builtin.name == "concept" {
                // Variable-keyed field on concept: value is an attribute reference.
                let attr_entity = if value_str.contains('/') {
                    resolve_or_create_attribute(site, value_str, "Text", "one").await?
                } else {
                    schema::lookup_entity_by_name(&site.branch, &site.operator, value_str)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("Attribute '{}' not found", value_str))?
                };

                let field_key = f.name.split_once('.').map(|(_, k)| k).unwrap_or(&f.name);
                concept_with_fields.insert(field_key.to_string(), attr_entity.clone());
                resolved_claims.push((relation, Value::Entity(attr_entity)));
            } else {
                resolved_claims.push((relation, schema::parse_value(value_str)));
            }
        } else {
            anyhow::bail!(
                "Unknown field '{}' for concept '{}'. Available fields: {}",
                f.name,
                builtin.name,
                builtin
                    .with_fields
                    .iter()
                    .chain(builtin.maybe_fields.iter())
                    .map(|f| {
                        if f.variable_keyed {
                            format!("{}.{{name}}", f.cli_name)
                        } else {
                            f.cli_name.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    // Derive entity based on concept type
    let entity = if let Some(ref entity_str) = this_entity {
        resolve_entity(entity_str)?
    } else {
        match builtin.name {
            "attribute" => {
                let selector = find_resolved_string(&resolved_claims, "dialog.attribute/id")
                    .ok_or_else(|| anyhow::anyhow!("Attribute requires `the` field"))?;
                let value_type = find_resolved_string(&resolved_claims, "dialog.attribute/type")
                    .unwrap_or_else(|| "Text".to_string());
                let cardinality =
                    find_resolved_string(&resolved_claims, "dialog.attribute/cardinality")
                        .unwrap_or_else(|| "one".to_string());
                schema::derive_attribute_entity(&selector, &value_type, &cardinality)?
            }
            "concept" => {
                if concept_with_fields.is_empty() {
                    anyhow::bail!("Concept requires at least one `with.{{name}}` field");
                }
                schema::derive_concept_entity(&concept_with_fields)?
            }
            _ => {
                let field_pairs: Vec<(String, String)> = resolved_claims
                    .iter()
                    .map(|(rel, val)| (rel.clone(), schema::format_value(val).to_string()))
                    .collect();
                schema::derive_entity_from_fields(&field_pairs)?
            }
        }
    };

    let mut tx = site.branch.transaction();

    if this_entity.is_some() {
        // Updating -- retract old cardinality-one values
        for (relation, _) in &resolved_claims {
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::parse_claim_attribute(relation)?,
                relation,
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
    }

    for (relation, value) in &resolved_claims {
        tx = tx.assert(schema::make_statement(
            relation,
            entity.clone(),
            value.clone(),
        )?);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        if this_entity.is_some() {
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::dialog_meta::Name::the().into(),
                "dialog.meta/name",
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
        tx = tx.assert(schema::dialog_meta::Name::of(entity.clone()).is(name.clone()));
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    print_assert_result(
        &entity,
        resolved_claims.len(),
        entity_name.as_deref(),
        format,
    );
    Ok(entity.to_string())
}

/// Assert claims using a user-defined concept target.
async fn assert_user_concept(
    site: &Site,
    concept_name: &str,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<String> {
    // All fields must have values
    for f in fields {
        if f.value.is_none() {
            anyhow::bail!(
                "Field '{}' requires a value (use {}=<VALUE>)",
                f.name,
                f.name
            );
        }
    }

    // Resolve the concept from the database
    let concept = schema::resolve_concept(&site.branch, &site.operator, concept_name).await?;

    // Map each field to its attribute selector
    let mut qualified_fields: Vec<(String, String)> = Vec::new();
    for f in fields {
        let selector = schema::resolve_field_selector(&concept, &f.name)?;
        qualified_fields.push((selector, f.value.clone().unwrap_or_default()));
    }

    let is_update = this_entity.is_some();

    let entity = if is_update {
        resolve_entity(this_entity.as_ref().unwrap())?
    } else {
        schema::derive_entity_from_fields(&qualified_fields)?
    };

    let mut tx = site.branch.transaction();

    if is_update {
        for (attr_name, _) in &qualified_fields {
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::parse_claim_attribute(attr_name)?,
                attr_name,
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
    }

    for (attr_name, value_str) in &qualified_fields {
        let value = schema::parse_value(value_str);
        tx = tx.assert(schema::make_statement(attr_name, entity.clone(), value)?);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        if is_update {
            for stmt in retract_cardinality_one_values(
                site,
                &entity,
                &schema::dialog_meta::Name::the().into(),
                "dialog.meta/name",
            )
            .await?
            {
                tx = tx.retract(stmt);
            }
        }
        tx = tx.assert(schema::dialog_meta::Name::of(entity.clone()).is(name.clone()));
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    print_assert_result(
        &entity,
        qualified_fields.len(),
        entity_name.as_deref(),
        format,
    );
    Ok(entity.to_string())
}

/// Look up or create an attribute entity from its selector, with default type and cardinality.
async fn resolve_or_create_attribute(
    site: &Site,
    selector: &str,
    default_type: &str,
    default_cardinality: &str,
) -> Result<dialog_query::Entity> {
    let entity = schema::derive_attribute_entity(selector, default_type, default_cardinality)?;

    // Check if it already exists
    let existing = schema::fetch_string(
        &site.branch,
        &site.operator,
        &entity,
        schema::dialog_attribute::Id::the().into(),
    )
    .await?;

    if existing.is_some() {
        return Ok(entity);
    }

    // Create it with defaults
    site.branch
        .transaction()
        .assert(schema::dialog_attribute::Id::of(entity.clone()).is(selector.to_string()))
        .assert(schema::dialog_attribute::Type::of(entity.clone()).is(default_type.to_string()))
        .assert(
            schema::dialog_attribute::Cardinality::of(entity.clone())
                .is(default_cardinality.to_string()),
        )
        .commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create attribute: {}", e))?;

    // Report the defaulting in non-interactive mode
    if !atty_stdout() {
        eprintln!(
            "Note: created attribute '{}' with defaults (as={}, cardinality={}). \
             Use `carry assert attribute the={} as=<TYPE> cardinality=<CARD>` to change.",
            selector, default_type, default_cardinality, selector
        );
    }

    Ok(entity)
}

/// Find a string value in resolved claims by relation name.
fn find_resolved_string(claims: &[(String, Value)], relation: &str) -> Option<String> {
    claims.iter().find_map(|(rel, val)| {
        if rel == relation {
            Some(schema::format_value(val))
        } else {
            None
        }
    })
}

/// Print assertion result.
fn print_assert_result(
    entity: &dialog_query::Entity,
    count: usize,
    name: Option<&str>,
    format: &str,
) {
    match format {
        "json" => {
            let mut obj = serde_json::json!({
                "entity": entity.to_string(),
                "asserted": count,
            });
            if let Some(n) = name {
                obj["name"] = serde_json::Value::String(n.to_string());
            }
            println!("{}", obj);
        }
        _ => {
            println!("{}", entity);
        }
    }
}

/// Check if stdout is a TTY (interactive mode).
fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

// ---------------------------------------------------------------------------
// File/stdin input (retained, mostly unchanged)
// ---------------------------------------------------------------------------

/// Assert claims from a YAML/JSON file.
async fn assert_from_file(site: &Site, path: &str, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read '{}'. If this is a target (not a file), \
             use a dotted domain (e.g. books.dune) instead of a slash",
            path
        )
    })?;
    assert_from_content(site, &content, path, format).await
}

/// Assert claims from stdin.
async fn assert_from_stdin(site: &Site, format: &str) -> Result<()> {
    let content = std::io::read_to_string(std::io::stdin())?;
    assert_from_content(site, &content, "-", format).await
}

/// Assert claims from file/stdin content.
async fn assert_from_content(
    site: &Site,
    content: &str,
    source: &str,
    _format: &str,
) -> Result<()> {
    let trimmed = content.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        assert_from_json(site, trimmed).await
    } else {
        assert_from_yaml(site, trimmed).await
    }
    .with_context(|| format!("Failed to process {}", source))
}

/// Assert claims from formal JSON content.
async fn assert_from_json(site: &Site, content: &str) -> Result<()> {
    let triples: Vec<serde_json::Value> = serde_json::from_str(content)?;

    let mut tx = site.branch.transaction();
    let mut count = 0;

    for triple in &triples {
        let the = triple["the"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'the' in triple"))?;
        let of = triple["of"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'of' in triple"))?;
        let is = &triple["is"];

        let entity = resolve_entity(of)?;
        let value = json_to_value(is)?;

        // Retract existing values if cardinality is "one"
        let attr = schema::parse_claim_attribute(the)?;
        for stmt in retract_cardinality_one_values(site, &entity, &attr, the).await? {
            tx = tx.retract(stmt);
        }

        tx = tx.assert(schema::make_statement(the, entity, value)?);
        count += 1;
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    println!("Asserted {} claims", count);
    Ok(())
}

/// Assert claims from formal YAML content.
///
/// Supports three formats:
/// 1. EAV triple notation (sequence of `{the, of, is}` mappings)
/// 2. Asserted notation with domain context (entity -> domain -> fields)
/// 3. Asserted notation with concept context (name -> concept_type -> fields)
async fn assert_from_yaml(site: &Site, content: &str) -> Result<()> {
    let doc: serde_yaml::Value = serde_yaml::from_str(content)?;

    match &doc {
        serde_yaml::Value::Sequence(seq) => {
            // Sequence of EAV triples
            assert_from_eav_yaml(site, seq).await
        }
        serde_yaml::Value::Mapping(map) => {
            if map.get("the").is_some() {
                // Single EAV triple (not wrapped in a sequence)
                assert_from_eav_yaml(site, from_ref(&doc)).await
            } else if map.iter().any(|(_, v)| v.is_mapping()) {
                // Asserted notation
                assert_from_asserted_yaml(site, map).await
            } else {
                anyhow::bail!(
                    "Unrecognized YAML format: expected EAV triples (sequence of {{the, of, is}}) \
                     or asserted notation (entity -> context -> fields)"
                )
            }
        }
        _ => anyhow::bail!("Expected YAML sequence or mapping"),
    }
}

/// Assert claims from EAV triple YAML (sequence of `{the, of, is}` mappings).
async fn assert_from_eav_yaml(site: &Site, triples: &[serde_yaml::Value]) -> Result<()> {
    let mut tx = site.branch.transaction();
    let mut count = 0;

    for triple in triples {
        let the = triple["the"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'the' in triple"))?;
        let of = triple["of"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'of' in triple"))?;

        let entity = resolve_entity(of)?;
        let is = &triple["is"];
        let value = yaml_to_value(is)?;

        // Retract existing values if cardinality is "one"
        let attr = schema::parse_claim_attribute(the)?;
        for stmt in retract_cardinality_one_values(site, &entity, &attr, the).await? {
            tx = tx.retract(stmt);
        }

        tx = tx.assert(schema::make_statement(the, entity, value)?);
        count += 1;
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    println!("Asserted {} claims", count);
    Ok(())
}

/// Assert claims from asserted notation YAML (entity-grouped mapping).
///
/// Handles both domain context (level-2 key contains '.') and concept context
/// (level-2 key has no '.'), as specified by the carry RFC.
async fn assert_from_asserted_yaml(site: &Site, top_map: &serde_yaml::Mapping) -> Result<()> {
    struct ConceptEntry<'a> {
        entity_name: Option<String>,
        this_entity: Option<String>,
        concept_type: String,
        fields_yaml: &'a serde_yaml::Value,
    }

    let mut domain_map = serde_yaml::Mapping::new();
    let mut concept_entries: Vec<ConceptEntry> = Vec::new();

    for (entity_key, context_val) in top_map {
        let entity_id = entity_key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Expected entity key to be a string"))?;

        let ctx_map = context_val.as_mapping().ok_or_else(|| {
            anyhow::anyhow!(
                "Expected context mapping for entity '{}', got {:?}",
                entity_id,
                context_val
            )
        })?;

        for (ctx_key, fields_val) in ctx_map {
            let ctx_name = ctx_key
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Expected context key to be a string"))?;

            if ctx_name.contains('.') {
                let entity_entry = domain_map
                    .entry(entity_key.clone())
                    .or_insert(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                if let serde_yaml::Value::Mapping(m) = entity_entry {
                    m.insert(ctx_key.clone(), fields_val.clone());
                }
            } else {
                let (entity_name, this_entity) = classify_entity_id(entity_id);
                concept_entries.push(ConceptEntry {
                    entity_name,
                    this_entity,
                    concept_type: ctx_name.to_string(),
                    fields_yaml: fields_val,
                });
            }
        }
    }

    // Sort concept entries: attributes first, then concepts, then others.
    concept_entries.sort_by_key(|e| match e.concept_type.as_str() {
        "attribute" => 0,
        "concept" => 1,
        "bookmark" => 2,
        "rule" => 3,
        _ => 4,
    });

    // Process concept-context entries via the target-based assertion pipeline.
    for entry in &concept_entries {
        assert_concept_from_yaml(
            site,
            &entry.concept_type,
            entry.entity_name.clone(),
            entry.this_entity.clone(),
            entry.fields_yaml,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to assert {} '{}'",
                entry.concept_type,
                entry
                    .entity_name
                    .as_deref()
                    .or(entry.this_entity.as_deref())
                    .unwrap_or("_")
            )
        })?;
    }

    // Process domain-context entries using transaction
    if !domain_map.is_empty() {
        assert_domain_entries_from_yaml(site, &domain_map).await?;
    }

    Ok(())
}

/// Classify a level-1 entity identifier from asserted notation YAML.
fn classify_entity_id(id: &str) -> (Option<String>, Option<String>) {
    if id.contains(':') {
        (None, Some(id.to_string()))
    } else if id == "_" {
        (None, None)
    } else {
        (Some(id.to_string()), None)
    }
}

/// Assert a concept-context entry from asserted notation YAML.
async fn assert_concept_from_yaml(
    site: &Site,
    concept_type: &str,
    entity_name: Option<String>,
    this_entity: Option<String>,
    fields_yaml: &serde_yaml::Value,
) -> Result<()> {
    let map = fields_yaml
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("Expected mapping for {} fields", concept_type))?;

    let mut fields = Vec::new();

    for (key, value) in map {
        let key_str = key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Expected string key in {} fields", concept_type))?;

        if (key_str == "with" || key_str == "maybe") && value.is_mapping() {
            let sub_map = value.as_mapping().unwrap();
            for (sub_key, sub_value) in sub_map {
                let sub_key_str = sub_key
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Expected string key in {}.{{}}", key_str))?;

                if sub_value.is_mapping() {
                    let selector = assert_inline_attribute_from_yaml(site, sub_value)
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to process inline attribute definition for {}.{}",
                                key_str, sub_key_str
                            )
                        })?;
                    fields.push(Field {
                        name: format!("{}.{}", key_str, sub_key_str),
                        value: Some(selector),
                    });
                } else {
                    let value_str = yaml_scalar_to_string(sub_value).with_context(|| {
                        format!("Invalid value for {}.{}", key_str, sub_key_str)
                    })?;
                    fields.push(Field {
                        name: format!("{}.{}", key_str, sub_key_str),
                        value: Some(value_str),
                    });
                }
            }
        } else {
            let value_str = yaml_scalar_to_string(value)
                .with_context(|| format!("Invalid value for field '{}'", key_str))?;
            fields.push(Field {
                name: key_str.to_string(),
                value: Some(value_str),
            });
        }
    }

    let target = Target::parse(concept_type)?;
    assert_with_target(site, target, this_entity, entity_name, fields, "yaml").await?;
    Ok(())
}

/// Assert an inline attribute definition from YAML and return its selector.
async fn assert_inline_attribute_from_yaml(
    site: &Site,
    fields_yaml: &serde_yaml::Value,
) -> Result<String> {
    let map = fields_yaml
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("Expected mapping for inline attribute definition"))?;

    let mut fields = Vec::new();
    let mut selector = None;

    for (key, value) in map {
        let key_str = key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Expected string key in inline attribute"))?;
        let value_str = yaml_scalar_to_string(value)
            .with_context(|| format!("Invalid value for inline attribute field '{}'", key_str))?;

        if key_str == "the" {
            selector = Some(value_str.clone());
        }

        fields.push(Field {
            name: key_str.to_string(),
            value: Some(value_str),
        });
    }

    let selector = selector.ok_or_else(|| {
        anyhow::anyhow!("Inline attribute definition missing required 'the' field")
    })?;

    let target = Target::parse("attribute")?;
    assert_with_target(site, target, None, None, fields, "yaml").await?;

    Ok(selector)
}

/// Convert a YAML scalar value to its string representation for use in Field values.
fn yaml_scalar_to_string(v: &serde_yaml::Value) -> Result<String> {
    match v {
        serde_yaml::Value::String(s) => Ok(s.clone()),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(f) = n.as_f64() {
                Ok(f.to_string())
            } else {
                Ok(format!("{:?}", n))
            }
        }
        serde_yaml::Value::Bool(b) => Ok(b.to_string()),
        serde_yaml::Value::Null => anyhow::bail!("Unexpected null value"),
        _ => {
            let s = serde_yaml::to_string(v)?;
            Ok(s.trim().to_string())
        }
    }
}

/// Assert domain-context entries from asserted notation YAML (batch via transaction).
async fn assert_domain_entries_from_yaml(
    site: &Site,
    domain_map: &serde_yaml::Mapping,
) -> Result<()> {
    let mut tx = site.branch.transaction();
    let mut count = 0;

    for (entity_key, namespace_map) in domain_map {
        let entity_id = entity_key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Expected entity key to be a string"))?;
        let entity = resolve_entity(entity_id)?;

        let ns_map = namespace_map.as_mapping().ok_or_else(|| {
            anyhow::anyhow!("Expected namespace mapping for entity '{}'", entity_id)
        })?;

        for (ns_key, fields_val) in ns_map {
            let namespace = ns_key
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Expected namespace key to be a string"))?;

            let fields_map = fields_val.as_mapping().ok_or_else(|| {
                anyhow::anyhow!("Expected fields mapping under namespace '{}'", namespace)
            })?;

            for (field_key, value) in fields_map {
                let field_name = field_key
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Expected field key to be a string"))?;

                let qualified = format!("{}/{}", namespace, field_name);

                // Retract existing values if cardinality is "one"
                let attr = schema::parse_claim_attribute(&qualified)?;
                for stmt in retract_cardinality_one_values(site, &entity, &attr, &qualified).await?
                {
                    tx = tx.retract(stmt);
                }

                // Handle multi-valued fields (YAML sequences)
                match value {
                    serde_yaml::Value::Sequence(seq) => {
                        for item in seq {
                            let val = yaml_to_value(item)?;
                            tx =
                                tx.assert(schema::make_statement(&qualified, entity.clone(), val)?);
                            count += 1;
                        }
                    }
                    _ => {
                        let val = yaml_to_value(value)?;
                        tx = tx.assert(schema::make_statement(&qualified, entity.clone(), val)?);
                        count += 1;
                    }
                }
            }
        }
    }

    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to commit: {}", e))?;

    println!("Asserted {} claims", count);
    Ok(())
}

fn json_to_value(v: &serde_json::Value) -> Result<Value> {
    match v {
        serde_json::Value::String(s) => Ok(schema::parse_value(s)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Ok(Value::UnsignedInt(i as u128))
                } else {
                    Ok(Value::SignedInt(i as i128))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                anyhow::bail!("Unsupported number: {}", n)
            }
        }
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        _ => Ok(Value::String(v.to_string())),
    }
}

fn yaml_to_value(v: &serde_yaml::Value) -> Result<Value> {
    match v {
        serde_yaml::Value::String(s) => Ok(schema::parse_value(s)),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Ok(Value::UnsignedInt(i as u128))
                } else {
                    Ok(Value::SignedInt(i as i128))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                anyhow::bail!("Unsupported number: {:?}", n)
            }
        }
        serde_yaml::Value::Bool(b) => Ok(Value::Boolean(*b)),
        _ => {
            let s = serde_yaml::to_string(v)?;
            Ok(Value::String(s.trim().to_string()))
        }
    }
}

/// Resolve an entity identifier string to an Entity.
fn resolve_entity(s: &str) -> Result<dialog_query::Entity> {
    if s.starts_with("did:") {
        dialog_query::Entity::from_str(s).context("Invalid entity DID")
    } else {
        schema::derive_entity(s)
    }
}
