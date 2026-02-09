use crate::authority;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use dialog_artifacts::replica::{BranchId, Replica, SigningAuthority};
use dialog_artifacts::{Artifact, ArtifactSelector, ArtifactStore, ArtifactStoreMut, Instruction};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::FsBackend;

/// Derive a deterministic item entity from topic + content.
/// Same topic + same content = same entity (natural dedup).
fn item_entity(topic: &str, content: &str) -> Result<Entity> {
    let hash = blake3::hash(format!("{}\0{}", topic, content).as_bytes());
    let b58 = bs58::encode(hash.as_bytes()).into_string();
    let uri = format!("did:key:z{}", b58);
    Entity::from_str(&uri).context("Failed to create item entity")
}

/// Get the storage path and space DID for the active space
fn get_active_space_info() -> Result<(PathBuf, String)> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.did().to_string();

    let authority = authority::get_active_authority()?
        .context("No active authority. Run 'tonk login' first")?;

    let space_did = state::get_active_space(&authority.did)?
        .context("No active space. Run 'tonk space create' first")?;

    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let path = home
        .join(".tonk")
        .join("operator")
        .join(&operator_did)
        .join("session")
        .join(&authority.did)
        .join("space")
        .join(&space_did)
        .join("facts");

    Ok((path, space_did))
}

pub async fn execute(
    content_args: Vec<String>,
    topic: Option<String>,
    kind: Option<String>,
    file: Option<String>,
    json: bool,
) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Resolve content from args, file, or stdin
    let content = if let Some(path) = &file {
        std::fs::read_to_string(path).context(format!("Failed to read file: {}", path))?
    } else if !content_args.is_empty() {
        content_args.join(" ")
    } else {
        // Read from stdin
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read from stdin")?;
        buf
    };

    let content = content.trim().to_string();
    if content.is_empty() {
        anyhow::bail!("No content provided. Pass content as arguments, --file, or pipe to stdin.");
    }

    let topic = topic.unwrap_or_else(|| "general".to_string());
    let kind = kind.unwrap_or_else(|| "note".to_string());
    let now = chrono::Utc::now().timestamp();

    // Derive item entity from topic + content (deterministic)
    let entity = item_entity(&topic, &content)?;

    // Build attributes
    let content_attr = Attribute::from_str("ctx/content").context("Invalid attribute")?;
    let topic_attr = Attribute::from_str("ctx/topic").context("Invalid attribute")?;
    let kind_attr = Attribute::from_str("ctx/kind").context("Invalid attribute")?;
    let ts_attr = Attribute::from_str("ctx/ts").context("Invalid attribute")?;
    let idx_topic_attr = Attribute::from_str("idx/topic").context("Invalid attribute")?;
    let idx_item_attr = Attribute::from_str("idx/item").context("Invalid attribute")?;

    // Open storage directly via Branch (needed for cause-based upsert on timestamp)
    let (storage_path, space_did) = get_active_space_info()?;
    let backend = FsBackend::new(&storage_path).await?;
    let authority = SigningAuthority::from(&operator);
    let replica = Replica::open(authority, space_did.clone().into(), backend)?;
    let branch_id = BranchId::new("main".to_string());
    let mut branch = replica.branches.open(&branch_id).await?;

    // Generate a summary for large content
    let summary = if content.len() > 200 {
        let first_line = content.lines().next().unwrap_or(&content);
        if first_line.len() > 200 {
            format!("{}...", &first_line[..197])
        } else {
            first_line.to_string()
        }
    } else {
        content.clone()
    };

    // Parse space DID as entity for index facts
    let space_entity =
        Entity::from_str(&space_did).context("Failed to parse space DID as entity")?;

    // Check if timestamp already exists for this item (for upsert)
    let existing_ts: Vec<Artifact> = branch
        .select(
            ArtifactSelector::new()
                .of(entity.clone())
                .the(ts_attr.clone()),
        )
        .try_collect()
        .await?;

    let ts_instruction = if let Some(old) = existing_ts.into_iter().next() {
        Instruction::Assert(old.update(Value::SignedInt(now as i128)))
    } else {
        Instruction::Assert(Artifact {
            the: ts_attr,
            of: entity.clone(),
            is: Value::SignedInt(now as i128),
            cause: None,
        })
    };

    // Build all instructions
    // Content, topic, kind are idempotent (same entity+attr+value = same key = overwrite)
    let instructions = vec![
        Instruction::Assert(Artifact {
            the: content_attr,
            of: entity.clone(),
            is: Value::String(content.clone()),
            cause: None,
        }),
        Instruction::Assert(Artifact {
            the: topic_attr,
            of: entity.clone(),
            is: Value::String(topic.clone()),
            cause: None,
        }),
        Instruction::Assert(Artifact {
            the: kind_attr,
            of: entity.clone(),
            is: Value::String(kind.clone()),
            cause: None,
        }),
        ts_instruction,
        // Index facts (against space DID)
        Instruction::Assert(Artifact {
            the: idx_topic_attr,
            of: space_entity.clone(),
            is: Value::String(topic.clone()),
            cause: None,
        }),
        Instruction::Assert(Artifact {
            the: idx_item_attr,
            of: space_entity,
            is: Value::Entity(entity.clone()),
            cause: None,
        }),
    ];

    // Also store summary if content is long
    let instructions = if content.len() > 200 {
        let summary_attr = Attribute::from_str("ctx/summary").context("Invalid attribute")?;
        let mut instrs = instructions;

        // Check for existing summary (upsert)
        let existing_summary: Vec<Artifact> = branch
            .select(
                ArtifactSelector::new()
                    .of(entity.clone())
                    .the(summary_attr.clone()),
            )
            .try_collect()
            .await?;

        let summary_instr = if let Some(old) = existing_summary.into_iter().next() {
            Instruction::Assert(old.update(Value::String(summary.clone())))
        } else {
            Instruction::Assert(Artifact {
                the: summary_attr,
                of: entity.clone(),
                is: Value::String(summary.clone()),
                cause: None,
            })
        };
        instrs.push(summary_instr);
        instrs
    } else {
        instructions
    };

    // Commit all in one batch
    branch
        .commit(futures_util::stream::iter(instructions))
        .await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "id": entity.to_string(),
            "topic": topic,
            "kind": kind,
            "timestamp": now,
            "summary": summary,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("Remembered under topic '{}' ({})", topic, kind);
        if content.len() > 200 {
            println!("  {}", summary);
        }
    }

    Ok(())
}
