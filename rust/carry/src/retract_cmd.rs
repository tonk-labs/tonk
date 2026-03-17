//! `carry retract` — retract claims from entities.
//!
//! Supports domain targets, concept targets, file input, and stdin.

use crate::schema;
use crate::site::SiteContext;
use crate::target::{Field, FirstArg, Target};
use anyhow::{Context, Result};
use dialog_artifacts::{Artifact, ArtifactStoreMut, Instruction};

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
        FirstArg::Stdin | FirstArg::File(_) => {
            anyhow::bail!("File/stdin retract is not yet implemented")
        }
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

    let namespace = target.namespace();
    let mut branch = ctx.open_branch().await?;

    if fields.is_empty() {
        // Retract ALL facts about this entity
        let all_facts = schema::fetch_all_entity_facts(&branch, &entity).await?;
        if all_facts.is_empty() {
            anyhow::bail!("Entity '{}' not found (no facts to retract)", entity);
        }

        let instructions: Vec<Instruction> = all_facts
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
    } else {
        // Retract specific fields
        let mut instructions = Vec::new();

        for f in &fields {
            let attr_name = f.qualified_name(namespace);
            let attr = schema::parse_claim_attribute(&attr_name)?;

            if let Some(ref val_str) = f.value {
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
                let values = schema::fetch_values(&branch, &entity, attr.clone()).await?;
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

    Ok(())
}
