//! `carry assert` — assert claims on entities.
//!
//! Supports domain targets, concept targets (both builtin and user-defined),
//! file input, and stdin.
//!
//! ## Concept assertion sugar
//!
//! Builtin concepts (`attribute`, `concept`, `bookmark`) have special handling:
//! - Fields are mapped through the builtin schema (e.g. `the` → `dialog.attribute/id`)
//! - Variable-keyed fields (e.g. `with.name=...`) are expanded
//! - `@name` asserts `dialog.meta/name` on the entity
//!
//! User-defined concepts resolve via `dialog.meta/name` → concept entity →
//! `dialog.concept.with/*` claims → attribute selectors.

use crate::schema;
use crate::site::SiteContext;
use crate::target::{Field, FirstArg, Target};
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::Attribute;
use dialog_query::Value;
use dialog_query::claim::{Claim, Relation};
use std::collections::BTreeMap;
use std::slice::from_ref;
use std::str::FromStr;

/// Execute `carry assert <TARGET>|<FILE>|- [this=<ENTITY>] [@name] [FIELD=VALUE...]`.
pub async fn execute(
    ctx: &SiteContext,
    first_arg: FirstArg,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    match first_arg {
        FirstArg::Stdin => assert_from_stdin(ctx, format).await,
        FirstArg::File(path) => assert_from_file(ctx, &path, format).await,
        FirstArg::Target(target) => {
            assert_with_target(ctx, target, this_entity, entity_name, fields, format).await
        }
    }
}

/// Assert claims from a target + fields.
async fn assert_with_target(
    ctx: &SiteContext,
    target: Target,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("At least one FIELD=VALUE pair is required for assert");
    }

    match target {
        Target::Domain(ref domain) => {
            assert_domain(ctx, domain, this_entity, entity_name, &fields, format).await
        }
        Target::Concept(ref concept_name) => {
            // Check if it's a builtin concept first
            if let Some(builtin) = schema::lookup_builtin(concept_name) {
                assert_builtin_concept(ctx, builtin, this_entity, entity_name, &fields, format)
                    .await
            } else {
                assert_user_concept(ctx, concept_name, this_entity, entity_name, &fields, format)
                    .await
            }
        }
    }
}

/// Assert claims using a domain target (open-ended, no schema).
async fn assert_domain(
    ctx: &SiteContext,
    domain: &str,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<()> {
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

    let mut session = ctx.open_session().await?;

    let entity = if let Some(ref entity_str) = this_entity {
        resolve_entity(entity_str)?
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

    let mut transaction = session.edit();

    for f in fields {
        let attr_name = f.qualified_name(domain);
        let attr = dialog_query::claim::Attribute::from_str(&attr_name)
            .context(format!("Invalid attribute: {}", attr_name))?;
        let value = schema::parse_value(f.value.as_deref().unwrap());
        Relation::new(attr, entity.clone(), value).assert(&mut transaction);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        let name_attr = schema::dialog_meta::Name::selector();
        Relation::new(name_attr, entity.clone(), Value::String(name.clone()))
            .assert(&mut transaction);
    }

    session.commit(transaction).await?;
    print_assert_result(&entity, fields.len(), entity_name.as_deref(), format);
    Ok(())
}

/// Assert claims using a builtin concept target (attribute, concept, bookmark).
async fn assert_builtin_concept(
    ctx: &SiteContext,
    builtin: &schema::BuiltinConceptSchema,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<()> {
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

    let mut session = ctx.open_session().await?;

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
                // If it contains '/', treat as a selector (look up or create attribute).
                // Otherwise, treat as a bookmark reference to an existing attribute.
                let attr_entity = if value_str.contains('/') {
                    // Selector: look up or create attribute with defaults
                    resolve_or_create_attribute(&mut session, value_str, "Text", "one").await?
                } else {
                    // Bookmark reference: look up by name
                    schema::lookup_entity_by_name(&session, value_str)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("Attribute '{}' not found", value_str))?
                };

                // Extract the field key from the dotted name (e.g. "with.name" → "name")
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
                // Attribute identity: hash(the, type, cardinality)
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
                // Concept identity: hash(sorted field→attribute pairs)
                if concept_with_fields.is_empty() {
                    anyhow::bail!("Concept requires at least one `with.{{name}}` field");
                }
                schema::derive_concept_entity(&concept_with_fields)?
            }
            _ => {
                // Generic: derive from field content
                let field_pairs: Vec<(String, String)> = resolved_claims
                    .iter()
                    .map(|(rel, val)| (rel.clone(), schema::format_value(val).to_string()))
                    .collect();
                schema::derive_entity_from_fields(&field_pairs)?
            }
        }
    };

    let mut transaction = session.edit();

    for (relation, value) in &resolved_claims {
        let attr = schema::parse_claim_attribute(relation)?;
        Relation::new(attr, entity.clone(), value.clone()).assert(&mut transaction);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        let name_attr = schema::dialog_meta::Name::selector();
        Relation::new(name_attr, entity.clone(), Value::String(name.clone()))
            .assert(&mut transaction);
    }

    session.commit(transaction).await?;
    print_assert_result(
        &entity,
        resolved_claims.len(),
        entity_name.as_deref(),
        format,
    );
    Ok(())
}

/// Assert claims using a user-defined concept target.
async fn assert_user_concept(
    ctx: &SiteContext,
    concept_name: &str,
    this_entity: Option<String>,
    entity_name: Option<String>,
    fields: &[Field],
    format: &str,
) -> Result<()> {
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

    let session = ctx.open_session().await?;

    // Resolve the concept from the database
    let concept = schema::resolve_concept(&session, concept_name).await?;

    // Map each field to its attribute selector
    let mut qualified_fields: Vec<(String, String)> = Vec::new();
    for f in fields {
        let selector = schema::resolve_field_selector(&concept, &f.name)?;
        qualified_fields.push((selector, f.value.clone().unwrap_or_default()));
    }

    drop(session);
    let mut session = ctx.open_session().await?;

    // Derive or resolve entity
    let entity = if let Some(ref entity_str) = this_entity {
        resolve_entity(entity_str)?
    } else {
        schema::derive_entity_from_fields(&qualified_fields)?
    };

    let mut transaction = session.edit();

    for (attr_name, value_str) in &qualified_fields {
        let attr = schema::parse_claim_attribute(attr_name)?;
        let value = schema::parse_value(value_str);
        Relation::new(attr, entity.clone(), value).assert(&mut transaction);
    }

    // Assert entity name if @name was provided
    if let Some(ref name) = entity_name {
        let name_attr = schema::dialog_meta::Name::selector();
        Relation::new(name_attr, entity.clone(), Value::String(name.clone()))
            .assert(&mut transaction);
    }

    session.commit(transaction).await?;
    print_assert_result(
        &entity,
        qualified_fields.len(),
        entity_name.as_deref(),
        format,
    );
    Ok(())
}

/// Look up or create an attribute entity from its selector, with default type and cardinality.
async fn resolve_or_create_attribute<S: dialog_query::Store>(
    session: &mut dialog_query::Session<S>,
    selector: &str,
    default_type: &str,
    default_cardinality: &str,
) -> Result<dialog_query::Entity> {
    let entity = schema::derive_attribute_entity(selector, default_type, default_cardinality)?;

    // Check if it already exists
    let existing =
        schema::fetch_string(session, &entity, schema::dialog_attribute::Id::selector()).await?;

    if existing.is_some() {
        return Ok(entity);
    }

    // Create it with defaults
    let mut transaction = session.edit();

    let attr_id = schema::dialog_attribute::Id::selector();
    let attr_type = schema::dialog_attribute::Type::selector();
    let attr_card = schema::dialog_attribute::Cardinality::selector();

    Relation::new(attr_id, entity.clone(), Value::String(selector.to_string()))
        .assert(&mut transaction);

    Relation::new(
        attr_type,
        entity.clone(),
        Value::String(default_type.to_string()),
    )
    .assert(&mut transaction);

    Relation::new(
        attr_card,
        entity.clone(),
        Value::String(default_cardinality.to_string()),
    )
    .assert(&mut transaction);

    session.commit(transaction).await?;

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
async fn assert_from_file(ctx: &SiteContext, path: &str, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    assert_from_content(ctx, &content, path, format).await
}

/// Assert claims from stdin.
async fn assert_from_stdin(ctx: &SiteContext, format: &str) -> Result<()> {
    let content = std::io::read_to_string(std::io::stdin())?;
    assert_from_content(ctx, &content, "-", format).await
}

/// Assert claims from file/stdin content.
async fn assert_from_content(
    ctx: &SiteContext,
    content: &str,
    source: &str,
    _format: &str,
) -> Result<()> {
    let trimmed = content.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        assert_from_json(ctx, trimmed).await
    } else {
        assert_from_yaml(ctx, trimmed).await
    }
    .with_context(|| format!("Failed to process {}", source))
}

/// Assert claims from formal JSON content.
async fn assert_from_json(ctx: &SiteContext, content: &str) -> Result<()> {
    let triples: Vec<serde_json::Value> = serde_json::from_str(content)?;
    let mut branch = ctx.open_branch().await?;

    let mut instructions = Vec::new();
    for triple in &triples {
        let the = triple["the"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'the' in triple"))?;
        let of = triple["of"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'of' in triple"))?;
        let is = &triple["is"];

        let attr = schema::parse_claim_attribute(the)?;
        let entity = resolve_entity(of)?;
        let value = json_to_value(is)?;

        instructions.push(Instruction::Assert(Artifact {
            the: attr,
            of: entity,
            is: value,
            cause: None,
        }));
    }

    let count = instructions.len();
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    println!("Asserted {} claims", count);
    Ok(())
}

/// Assert claims from formal YAML content.
///
/// Supports three formats:
/// 1. EAV triple notation (sequence of `{the, of, is}` mappings)
/// 2. Asserted notation with domain context (entity → domain → fields)
/// 3. Asserted notation with concept context (name → concept_type → fields)
async fn assert_from_yaml(ctx: &SiteContext, content: &str) -> Result<()> {
    let doc: serde_yaml::Value = serde_yaml::from_str(content)?;

    match &doc {
        serde_yaml::Value::Sequence(seq) => {
            // Sequence of EAV triples
            assert_from_eav_yaml(ctx, seq).await
        }
        serde_yaml::Value::Mapping(map) => {
            if map.get("the").is_some() {
                // Single EAV triple (not wrapped in a sequence)
                assert_from_eav_yaml(ctx, from_ref(&doc)).await
            } else if map.iter().any(|(_, v)| v.is_mapping()) {
                // Asserted notation: entity → context → fields.
                // Covers both domain context (level-2 key contains '.')
                // and concept context (level-2 key has no '.').
                assert_from_asserted_yaml(ctx, map).await
            } else {
                anyhow::bail!(
                    "Unrecognized YAML format: expected EAV triples (sequence of {{the, of, is}}) \
                     or asserted notation (entity → context → fields)"
                )
            }
        }
        _ => anyhow::bail!("Expected YAML sequence or mapping"),
    }
}

/// Assert claims from EAV triple YAML (sequence of `{the, of, is}` mappings).
async fn assert_from_eav_yaml(ctx: &SiteContext, triples: &[serde_yaml::Value]) -> Result<()> {
    let mut branch = ctx.open_branch().await?;

    let mut instructions = Vec::new();
    for triple in triples {
        let the = triple["the"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'the' in triple"))?;
        let of = triple["of"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'of' in triple"))?;

        let attr = schema::parse_claim_attribute(the)?;
        let entity = resolve_entity(of)?;
        let is = &triple["is"];
        let value = yaml_to_value(is)?;

        instructions.push(Instruction::Assert(Artifact {
            the: attr,
            of: entity,
            is: value,
            cause: None,
        }));
    }

    let count = instructions.len();
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    println!("Asserted {} claims", count);
    Ok(())
}

/// Assert claims from asserted notation YAML (entity-grouped mapping).
///
/// Handles both domain context (level-2 key contains '.') and concept context
/// (level-2 key has no '.'), as specified by the carry RFC.
///
/// Domain context example:
/// ```yaml
/// did:key:zAlice:
///   io.gozala.person:
///     name: Alice
///     age: 28
/// ```
///
/// Concept context example:
/// ```yaml
/// task-title:
///   attribute:
///     description: Title of a task
///     the: com.app.task/title
///     as: Text
///     cardinality: one
/// ```
async fn assert_from_asserted_yaml(ctx: &SiteContext, top_map: &serde_yaml::Mapping) -> Result<()> {
    // Collect entries classified by context type.
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
                // Domain context — collect for batch processing
                let entity_entry = domain_map
                    .entry(entity_key.clone())
                    .or_insert(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
                if let serde_yaml::Value::Mapping(m) = entity_entry {
                    m.insert(ctx_key.clone(), fields_val.clone());
                }
            } else {
                // Concept context — entity name from level-1 key
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
    // This ensures attributes exist before concepts reference them.
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
            ctx,
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

    // Process domain-context entries using raw branch (existing batch logic).
    if !domain_map.is_empty() {
        assert_domain_entries_from_yaml(ctx, &domain_map).await?;
    }

    Ok(())
}

/// Classify a level-1 entity identifier from asserted notation YAML.
///
/// Returns (entity_name, this_entity):
/// - DID/URI (contains ':'): this_entity = the DID, no entity_name
/// - `_` (anonymous): both None (entity derived from fields)
/// - Bookmark name: entity_name = the name, no this_entity
fn classify_entity_id(id: &str) -> (Option<String>, Option<String>) {
    if id.contains(':') {
        // DID or URI — use as explicit entity
        (None, Some(id.to_string()))
    } else if id == "_" {
        // Anonymous entity
        (None, None)
    } else {
        // Bookmark name → becomes @name
        (Some(id.to_string()), None)
    }
}

/// Assert a concept-context entry from asserted notation YAML.
///
/// Converts YAML fields to CLI-style `Field`s and dispatches through
/// `assert_with_target`, which handles builtin and user-defined concepts.
///
/// Handles:
/// - Simple fields: `key: value` → `Field { name: key, value: Some(value) }`
/// - Nested `with`/`maybe` maps: `with: { title: task-title }` → `Field { name: "with.title", value: Some("task-title") }`
/// - Inline attribute definitions: `with: { name: { the: ..., as: ... } }` → asserts the
///   attribute first, then references it by selector
async fn assert_concept_from_yaml(
    ctx: &SiteContext,
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
            // Nested mapping: expand each sub-entry as a variable-keyed field
            let sub_map = value.as_mapping().unwrap();
            for (sub_key, sub_value) in sub_map {
                let sub_key_str = sub_key
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Expected string key in {}.{{}}", key_str))?;

                if sub_value.is_mapping() {
                    // Inline attribute definition: assert it first, then use its selector
                    let selector = assert_inline_attribute_from_yaml(ctx, sub_value)
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
    assert_with_target(ctx, target, this_entity, entity_name, fields, "yaml").await
}

/// Assert an inline attribute definition from YAML and return its selector.
///
/// Used when a `with` block in a concept definition contains an inline mapping
/// instead of a string reference:
/// ```yaml
/// with:
///   name:
///     description: The person's name
///     the: io.gozala.person/name
///     as: Text
///     cardinality: one
/// ```
async fn assert_inline_attribute_from_yaml(
    ctx: &SiteContext,
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
    assert_with_target(ctx, target, None, None, fields, "yaml").await?;

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

/// Assert domain-context entries from asserted notation YAML (batch via raw branch).
///
/// This handles the `entity → domain → field: value` structure where the
/// domain key contains '.'.
async fn assert_domain_entries_from_yaml(
    ctx: &SiteContext,
    domain_map: &serde_yaml::Mapping,
) -> Result<()> {
    let mut branch = ctx.open_branch().await?;
    let mut instructions = Vec::new();

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

                // Build the qualified attribute name: namespace/field
                let qualified = format!("{}/{}", namespace, field_name);
                let attr = schema::parse_claim_attribute(&qualified)?;

                // Handle multi-valued fields (YAML sequences)
                match value {
                    serde_yaml::Value::Sequence(seq) => {
                        for item in seq {
                            let val = yaml_to_value(item)?;
                            instructions.push(Instruction::Assert(Artifact {
                                the: attr.clone(),
                                of: entity.clone(),
                                is: val,
                                cause: None,
                            }));
                        }
                    }
                    _ => {
                        let val = yaml_to_value(value)?;
                        instructions.push(Instruction::Assert(Artifact {
                            the: attr,
                            of: entity.clone(),
                            is: val,
                            cause: None,
                        }));
                    }
                }
            }
        }
    }

    let count = instructions.len();
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

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
