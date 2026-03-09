//! `carry retract` — retract claims from entities.
//!
//! Supports domain targets, concept targets (builtin and user-defined),
//! file input, and stdin.

use crate::schema;
use crate::site::SiteContext;
use crate::target::{Field, FirstArg, Target};
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use std::slice::from_ref;

/// Execute `carry retract <TARGET>|<FILE>|- [this=<ENTITY>] [FIELD[=VALUE]...]`.
pub async fn execute(
    ctx: &SiteContext,
    first_arg: FirstArg,
    this_entity: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    match first_arg {
        FirstArg::Target(target) => {
            retract_with_target(ctx, target, this_entity, fields, format).await
        }
        FirstArg::Stdin => retract_from_stdin(ctx, format).await,
        FirstArg::File(path) => retract_from_file(ctx, &path, format).await,
    }
}

/// Retract claims for a target + fields.
async fn retract_with_target(
    ctx: &SiteContext,
    target: Target,
    this_entity: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    let entity_str = this_entity.ok_or_else(|| {
        anyhow::anyhow!("Retract requires `this=<ENTITY>` to identify the entity")
    })?;

    let entity = if entity_str.starts_with("did:") {
        use std::str::FromStr;
        dialog_query::Entity::from_str(&entity_str).context("Invalid entity DID")?
    } else {
        schema::derive_entity(&entity_str)?
    };

    match target {
        Target::Domain(ref domain) => retract_domain(ctx, domain, &entity, &fields, format).await,
        Target::Concept(ref concept_name) => {
            retract_concept(ctx, concept_name, &entity, &fields, format).await
        }
    }
}

/// Retract claims using a domain target.
async fn retract_domain(
    ctx: &SiteContext,
    domain: &str,
    entity: &dialog_query::Entity,
    fields: &[Field],
    format: &str,
) -> Result<()> {
    let mut branch = ctx.open_branch().await?;

    if fields.is_empty() {
        // Retract ALL claims about this entity
        let all_claims = schema::fetch_all_entity_claims(&branch, entity).await?;
        if all_claims.is_empty() {
            anyhow::bail!("Entity '{}' not found (no claims to retract)", entity);
        }

        let instructions: Vec<Instruction> = all_claims
            .into_iter()
            .map(|artifact| {
                Instruction::Retract(Artifact {
                    the: artifact.the,
                    of: artifact.of,
                    is: artifact.is,
                    cause: artifact.cause,
                })
            })
            .collect();

        let count = instructions.len();
        branch
            .commit(futures_util::stream::iter(instructions))
            .await?;

        print_retract_result(entity, count, format);
    } else {
        retract_specific_fields(&mut branch, entity, domain, fields, format).await?;
    }

    Ok(())
}

/// Retract claims using a concept target.
async fn retract_concept(
    ctx: &SiteContext,
    concept_name: &str,
    entity: &dialog_query::Entity,
    fields: &[Field],
    format: &str,
) -> Result<()> {
    if fields.is_empty() {
        // Retract all claims about this entity
        let mut branch = ctx.open_branch().await?;
        let all_claims = schema::fetch_all_entity_claims(&branch, entity).await?;
        if all_claims.is_empty() {
            anyhow::bail!("Entity '{}' not found (no claims to retract)", entity);
        }

        let instructions: Vec<Instruction> = all_claims
            .into_iter()
            .map(|artifact| {
                Instruction::Retract(Artifact {
                    the: artifact.the,
                    of: artifact.of,
                    is: artifact.is,
                    cause: artifact.cause,
                })
            })
            .collect();

        let count = instructions.len();
        branch
            .commit(futures_util::stream::iter(instructions))
            .await?;

        print_retract_result(entity, count, format);
        return Ok(());
    }

    // Resolve the concept to get field→selector mappings
    let session = ctx.open_session().await?;

    // Try builtin first, then user-defined
    let resolved_fields: Vec<(String, Option<String>)> = if let Some(builtin) =
        schema::lookup_builtin(concept_name)
    {
        fields
            .iter()
            .map(|f| {
                let (relation, _) =
                    schema::resolve_builtin_field(builtin, &f.name).ok_or_else(|| {
                        anyhow::anyhow!("Unknown field '{}' for concept '{}'", f.name, concept_name)
                    })?;
                Ok((relation, f.value.clone()))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        let concept = schema::resolve_concept(&session, concept_name).await?;
        fields
            .iter()
            .map(|f| {
                let selector = schema::resolve_field_selector(&concept, &f.name)?;
                Ok((selector, f.value.clone()))
            })
            .collect::<Result<Vec<_>>>()?
    };

    drop(session);
    let mut branch = ctx.open_branch().await?;

    let mut instructions = Vec::new();

    for (attr_name, value) in &resolved_fields {
        let attr = schema::parse_claim_attribute(attr_name)?;

        if let Some(val_str) = value {
            // Retract a specific value
            let value = schema::parse_value(val_str);
            instructions.push(Instruction::Retract(Artifact {
                the: attr,
                of: entity.clone(),
                is: value,
                cause: None,
            }));
        } else {
            // Retract all values for this attribute
            let values = schema::fetch_values(&branch, entity, attr.clone()).await?;
            for value in values {
                instructions.push(Instruction::Retract(Artifact {
                    the: attr.clone(),
                    of: entity.clone(),
                    is: value,
                    cause: None,
                }));
            }
        }
    }

    if instructions.is_empty() {
        anyhow::bail!("No matching claims found to retract");
    }

    let count = instructions.len();
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    print_retract_result(entity, count, format);
    Ok(())
}

/// Retract specific fields using domain-qualified names.
async fn retract_specific_fields(
    branch: &mut dialog_artifacts::repository::Branch<tonk_space::FsBackend>,
    entity: &dialog_query::Entity,
    namespace: &str,
    fields: &[Field],
    format: &str,
) -> Result<()> {
    let mut instructions = Vec::new();

    for f in fields {
        let attr_name = f.qualified_name(namespace);
        let attr = schema::parse_claim_attribute(&attr_name)?;

        if let Some(ref val_str) = f.value {
            let value = schema::parse_value(val_str);
            instructions.push(Instruction::Retract(Artifact {
                the: attr,
                of: entity.clone(),
                is: value,
                cause: None,
            }));
        } else {
            let values = schema::fetch_values(branch, entity, attr.clone()).await?;
            for value in values {
                instructions.push(Instruction::Retract(Artifact {
                    the: attr.clone(),
                    of: entity.clone(),
                    is: value,
                    cause: None,
                }));
            }
        }
    }

    if instructions.is_empty() {
        anyhow::bail!("No matching claims found to retract");
    }

    let count = instructions.len();
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    print_retract_result(entity, count, format);
    Ok(())
}

fn print_retract_result(entity: &dialog_query::Entity, count: usize, format: &str) {
    match format {
        "json" => {
            println!(
                "{}",
                serde_json::json!({
                    "entity": entity.to_string(),
                    "retracted": count,
                })
            );
        }
        _ => {
            println!("Retracted {} claims from {}", count, entity);
        }
    }
}

// ---------------------------------------------------------------------------
// File/stdin input
// ---------------------------------------------------------------------------

/// Retract claims from a YAML/JSON file.
async fn retract_from_file(ctx: &SiteContext, path: &str, format: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    retract_from_content(ctx, &content, path, format).await
}

/// Retract claims from stdin.
async fn retract_from_stdin(ctx: &SiteContext, format: &str) -> Result<()> {
    let content = std::io::read_to_string(std::io::stdin())?;
    retract_from_content(ctx, &content, "-", format).await
}

/// Retract claims from file/stdin content.
async fn retract_from_content(
    ctx: &SiteContext,
    content: &str,
    source: &str,
    _format: &str,
) -> Result<()> {
    let trimmed = content.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        retract_from_json(ctx, trimmed).await
    } else {
        retract_from_yaml(ctx, trimmed).await
    }
    .with_context(|| format!("Failed to process {}", source))
}

/// Retract claims from formal JSON content (EAV triples).
async fn retract_from_json(ctx: &SiteContext, content: &str) -> Result<()> {
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

        instructions.push(Instruction::Retract(Artifact {
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

    println!("Retracted {} claims", count);
    Ok(())
}

/// Retract claims from YAML content.
///
/// Supports two formats:
/// 1. EAV triple notation (sequence of `{the, of, is}` mappings)
/// 2. Asserted notation (entity-grouped: `entity → namespace → field: value`)
async fn retract_from_yaml(ctx: &SiteContext, content: &str) -> Result<()> {
    let doc: serde_yaml::Value = serde_yaml::from_str(content)?;

    match &doc {
        serde_yaml::Value::Sequence(seq) => retract_from_eav_yaml(ctx, seq).await,
        serde_yaml::Value::Mapping(map) => {
            let is_asserted = map
                .iter()
                .any(|(k, v)| k.as_str().is_some_and(|s| s.starts_with("did:")) && v.is_mapping());

            if is_asserted {
                retract_from_asserted_yaml(ctx, map).await
            } else if map.get("the").is_some() {
                retract_from_eav_yaml(ctx, from_ref(&doc)).await
            } else {
                anyhow::bail!(
                    "Unrecognized YAML format: expected EAV triples (sequence of {{the, of, is}}) \
                     or asserted notation (entity → namespace → fields)"
                )
            }
        }
        _ => anyhow::bail!("Expected YAML sequence or mapping"),
    }
}

/// Retract claims from EAV triple YAML.
async fn retract_from_eav_yaml(ctx: &SiteContext, triples: &[serde_yaml::Value]) -> Result<()> {
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

        instructions.push(Instruction::Retract(Artifact {
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

    println!("Retracted {} claims", count);
    Ok(())
}

/// Retract claims from asserted notation YAML (entity-grouped mapping).
async fn retract_from_asserted_yaml(
    ctx: &SiteContext,
    top_map: &serde_yaml::Mapping,
) -> Result<()> {
    let mut branch = ctx.open_branch().await?;
    let mut instructions = Vec::new();

    for (entity_key, namespace_map) in top_map {
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
                let attr = schema::parse_claim_attribute(&qualified)?;

                match value {
                    serde_yaml::Value::Sequence(seq) => {
                        for item in seq {
                            let val = yaml_to_value(item)?;
                            instructions.push(Instruction::Retract(Artifact {
                                the: attr.clone(),
                                of: entity.clone(),
                                is: val,
                                cause: None,
                            }));
                        }
                    }
                    _ => {
                        let val = yaml_to_value(value)?;
                        instructions.push(Instruction::Retract(Artifact {
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

    println!("Retracted {} claims", count);
    Ok(())
}

fn resolve_entity(s: &str) -> Result<dialog_query::Entity> {
    if s.starts_with("did:") {
        use std::str::FromStr;
        dialog_query::Entity::from_str(s).context("Invalid entity DID")
    } else {
        schema::derive_entity(s)
    }
}

fn json_to_value(v: &serde_json::Value) -> Result<dialog_query::Value> {
    use dialog_query::Value;
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

fn yaml_to_value(v: &serde_yaml::Value) -> Result<dialog_query::Value> {
    use dialog_query::Value;
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
