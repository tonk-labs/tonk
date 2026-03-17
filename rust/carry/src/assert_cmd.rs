//! `carry assert` — assert claims on entities.
//!
//! Supports domain targets, concept targets, file input, and stdin.
//!
//! # TODO: Support asserted notation for file/stdin input
//!
//! Currently file/stdin input requires formal triple format: `[{the, of, is}, ...]`
//! The spec says asserted notation (entity -> context -> fields) should also work,
//! enabling round-trip: `carry query ... | carry assert -`
//!
//! To implement:
//! 1. Detect if input is asserted notation (map with entity keys) vs formal triples
//! 2. If asserted notation, expand to triples:
//!    - Level 1 key = entity (DID or `_` for anonymous)
//!    - Level 2 key = context (domain if contains `.`, concept if not)
//!    - Level 3 = field/value pairs -> expand to `{the: context/field, of: entity, is: value}`
//! 3. Handle concept context by resolving bookmarked concept to get attribute relations
//! 4. Handle nested entities (non-scalar values at level 3)

use crate::schema;
use crate::site::SiteContext;
use crate::target::{Field, FirstArg, Target};
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};
use dialog_query::claim::{Claim, Relation};
use std::str::FromStr;

/// Execute `carry assert <TARGET>|<FILE>|- [this=<ENTITY>] [FIELD=VALUE...]`.
pub async fn execute(
    ctx: &SiteContext,
    first_arg: FirstArg,
    this_entity: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    match first_arg {
        FirstArg::Stdin => assert_from_stdin(ctx, format).await,
        FirstArg::File(path) => assert_from_file(ctx, &path, format).await,
        FirstArg::Target(target) => {
            assert_with_target(ctx, target, this_entity, fields, format).await
        }
    }
}

/// Assert claims from a target + fields.
async fn assert_with_target(
    ctx: &SiteContext,
    target: Target,
    this_entity: Option<String>,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("At least one FIELD=VALUE pair is required for assert");
    }

    // All fields must have values for assert
    for f in &fields {
        if f.value.is_none() {
            anyhow::bail!(
                "Field '{}' requires a value (use {}=<VALUE>)",
                f.name,
                f.name
            );
        }
    }

    let namespace = target.namespace();
    let mut session = ctx.open_session().await?;

    // Resolve or derive the entity
    let entity = if let Some(ref entity_str) = this_entity {
        resolve_entity(entity_str)?
    } else {
        // Derive entity from the field values
        let field_pairs: Vec<(String, String)> = fields
            .iter()
            .map(|f| {
                (
                    f.qualified_name(namespace),
                    f.value.clone().unwrap_or_default(),
                )
            })
            .collect();
        schema::derive_entity_from_fields(&field_pairs)?
    };

    // Build transaction using Session's edit API
    let mut transaction = session.edit();

    for f in &fields {
        let attr_name = f.qualified_name(namespace);
        let attr = dialog_query::claim::Attribute::from_str(&attr_name)
            .context(format!("Invalid attribute: {}", attr_name))?;
        let value = schema::parse_value(f.value.as_deref().unwrap());
        let relation = Relation::new(attr, entity.clone(), value);
        relation.assert(&mut transaction);
    }

    session.commit(transaction).await?;

    match format {
        "json" => {
            println!(
                "{}",
                serde_json::json!({
                    "entity": entity.to_string(),
                    "asserted": fields.len(),
                })
            );
        }
        _ => {
            println!("{}", entity);
        }
    }

    Ok(())
}

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
    // Detect format: JSON if starts with '{' or '[', otherwise YAML
    let trimmed = content.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        assert_from_json(ctx, trimmed).await
    } else {
        assert_from_yaml(ctx, trimmed).await
    }
    .with_context(|| format!("Failed to process {}", source))
}

/// Assert claims from formal JSON content.
///
/// Expects an array of `{the, of, is}` triples.
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
/// Expects a sequence of `{the, of, is}` mappings.
async fn assert_from_yaml(ctx: &SiteContext, content: &str) -> Result<()> {
    let docs: Vec<serde_yaml::Value> = serde_yaml::from_str(content)?;
    let mut branch = ctx.open_branch().await?;

    let triples = if docs.len() == 1 {
        match &docs[0] {
            serde_yaml::Value::Sequence(seq) => seq.clone(),
            other => vec![other.clone()],
        }
    } else {
        docs
    };

    let mut instructions = Vec::new();
    for triple in &triples {
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

fn json_to_value(v: &serde_json::Value) -> Result<dialog_query::Value> {
    match v {
        serde_json::Value::String(s) => Ok(schema::parse_value(s)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Ok(dialog_query::Value::UnsignedInt(i as u128))
                } else {
                    Ok(dialog_query::Value::SignedInt(i as i128))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(dialog_query::Value::Float(f))
            } else {
                anyhow::bail!("Unsupported number: {}", n)
            }
        }
        serde_json::Value::Bool(b) => Ok(dialog_query::Value::Boolean(*b)),
        _ => Ok(dialog_query::Value::String(v.to_string())),
    }
}

fn yaml_to_value(v: &serde_yaml::Value) -> Result<dialog_query::Value> {
    match v {
        serde_yaml::Value::String(s) => Ok(schema::parse_value(s)),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Ok(dialog_query::Value::UnsignedInt(i as u128))
                } else {
                    Ok(dialog_query::Value::SignedInt(i as i128))
                }
            } else if let Some(f) = n.as_f64() {
                Ok(dialog_query::Value::Float(f))
            } else {
                anyhow::bail!("Unsupported number: {:?}", n)
            }
        }
        serde_yaml::Value::Bool(b) => Ok(dialog_query::Value::Boolean(*b)),
        _ => {
            let s = serde_yaml::to_string(v)?;
            Ok(dialog_query::Value::String(s.trim().to_string()))
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
