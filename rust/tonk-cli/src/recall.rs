use crate::authority;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use dialog_artifacts::replica::{BranchId, Replica, SigningAuthority};
use dialog_artifacts::{ArtifactSelector, ArtifactStore};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::FsBackend;

/// A recalled item with all its metadata
#[derive(Debug)]
struct Item {
    id: String,
    topic: String,
    kind: String,
    timestamp: i64,
    content: String,
    summary: Option<String>,
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

/// Fetch a string value for a given entity+attribute, or return a default
async fn fetch_string<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr: &Attribute,
    default: &str,
) -> String {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr.clone()))
        .try_collect()
        .await
        .unwrap_or_default();

    results
        .into_iter()
        .next()
        .and_then(|a| match a.is {
            Value::String(s) => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| default.to_string())
}

/// Fetch a signed int value for a given entity+attribute
async fn fetch_timestamp<S: ArtifactStore>(store: &S, entity: &Entity, attr: &Attribute) -> i64 {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr.clone()))
        .try_collect()
        .await
        .unwrap_or_default();

    results
        .into_iter()
        .next()
        .and_then(|a| match a.is {
            Value::SignedInt(n) => Some(n as i64),
            Value::UnsignedInt(n) => Some(n as i64),
            _ => None,
        })
        .unwrap_or(0)
}

/// Fetch full item details for an entity
async fn fetch_item<S: ArtifactStore>(store: &S, entity: &Entity) -> Option<Item> {
    let content_attr = Attribute::from_str("ctx/content").ok()?;
    let topic_attr = Attribute::from_str("ctx/topic").ok()?;
    let kind_attr = Attribute::from_str("ctx/kind").ok()?;
    let ts_attr = Attribute::from_str("ctx/ts").ok()?;
    let summary_attr = Attribute::from_str("ctx/summary").ok()?;

    let content = fetch_string(store, entity, &content_attr, "").await;
    if content.is_empty() {
        return None;
    }

    let topic = fetch_string(store, entity, &topic_attr, "general").await;
    let kind = fetch_string(store, entity, &kind_attr, "note").await;
    let timestamp = fetch_timestamp(store, entity, &ts_attr).await;

    let summary_str = fetch_string(store, entity, &summary_attr, "").await;
    let summary = if summary_str.is_empty() {
        None
    } else {
        Some(summary_str)
    };

    Some(Item {
        id: entity.to_string(),
        topic,
        kind,
        timestamp,
        content,
        summary,
    })
}

/// Collect all item entity DIDs from the space index
async fn collect_item_entities<S: ArtifactStore>(
    store: &S,
    space_entity: &Entity,
) -> Result<Vec<Entity>> {
    let idx_item_attr = Attribute::from_str("idx/item").context("Invalid attribute")?;

    let results: Vec<_> = store
        .select(
            ArtifactSelector::new()
                .of(space_entity.clone())
                .the(idx_item_attr),
        )
        .try_collect()
        .await?;

    let entities: Vec<Entity> = results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::Entity(e) => Some(e),
            _ => None,
        })
        .collect();

    Ok(entities)
}

pub async fn execute(
    topic: Option<String>,
    kind: Option<String>,
    recent: Option<usize>,
    id: Option<String>,
    json: bool,
) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    let (storage_path, space_did) = get_active_space_info()?;
    let backend = FsBackend::new(&storage_path).await?;
    let authority = SigningAuthority::from(&operator);
    let replica = Replica::open(authority, space_did.clone().into(), backend)?;
    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    // Case 1: Recall by specific ID
    if let Some(item_id) = &id {
        let entity = Entity::from_str(item_id).context("Invalid item ID")?;
        let item = fetch_item(&branch, &entity).await;

        if json {
            match item {
                Some(item) => {
                    let output = serde_json::json!({
                        "id": item.id,
                        "topic": item.topic,
                        "kind": item.kind,
                        "timestamp": item.timestamp,
                        "content": item.content,
                    });
                    println!("{}", serde_json::to_string(&output)?);
                }
                None => {
                    println!("null");
                }
            }
        } else {
            match item {
                Some(item) => {
                    println!(
                        "[{}] {} ({})",
                        item.topic,
                        item.kind,
                        format_ts(item.timestamp)
                    );
                    println!("{}", item.content);
                }
                None => {
                    println!("Item not found: {}", item_id);
                }
            }
        }
        return Ok(());
    }

    // For all other cases, we need the space index
    let space_entity =
        Entity::from_str(&space_did).context("Failed to parse space DID as entity")?;
    let all_item_entities = collect_item_entities(&branch, &space_entity).await?;

    // Fetch all items
    let mut items: Vec<Item> = Vec::new();
    for entity in &all_item_entities {
        if let Some(item) = fetch_item(&branch, entity).await {
            items.push(item);
        }
    }

    // Filter by topic
    if let Some(topic_filter) = &topic {
        items.retain(|item| &item.topic == topic_filter);
    }

    // Filter by kind
    if let Some(kind_filter) = &kind {
        items.retain(|item| &item.kind == kind_filter);
    }

    // Sort by timestamp descending (most recent first)
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Apply --recent limit
    if let Some(n) = recent {
        items.truncate(n);
    }

    if json {
        let json_items: Vec<serde_json::Value> = items
            .iter()
            .map(|item| {
                let content_or_summary = if item.content.len() > 500 {
                    item.summary
                        .as_deref()
                        .unwrap_or_else(|| &item.content[..200])
                        .to_string()
                } else {
                    item.content.clone()
                };

                let mut obj = serde_json::json!({
                    "id": item.id,
                    "topic": item.topic,
                    "kind": item.kind,
                    "timestamp": item.timestamp,
                    "content": content_or_summary,
                });

                if item.content.len() > 500 {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("truncated".to_string(), serde_json::Value::Bool(true));
                    obj.as_object_mut().unwrap().insert(
                        "content_length".to_string(),
                        serde_json::json!(item.content.len()),
                    );
                }

                obj
            })
            .collect();
        println!("{}", serde_json::to_string(&json_items)?);
    } else {
        if items.is_empty() {
            println!("No items found.");
            return Ok(());
        }

        for item in &items {
            println!(
                "--- [{}] {} ({}) ---",
                item.topic,
                item.kind,
                format_ts(item.timestamp)
            );
            if item.content.len() > 500 {
                println!(
                    "{}...\n  ({} chars total, use --id {} to see full content)\n",
                    item.summary.as_deref().unwrap_or(&item.content[..200]),
                    item.content.len(),
                    item.id,
                );
            } else {
                println!("{}\n", item.content);
            }
        }
    }

    Ok(())
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
