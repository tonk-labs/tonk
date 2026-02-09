use crate::authority;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use dialog_artifacts::replica::{BranchId, Replica, SigningAuthority};
use dialog_artifacts::{ArtifactSelector, ArtifactStore};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::FsBackend;

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

/// Fetch a string value for a given entity+attribute
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

/// Fetch timestamp for an entity
async fn fetch_timestamp<S: ArtifactStore>(store: &S, entity: &Entity) -> i64 {
    let ts_attr = match Attribute::from_str("ctx/ts") {
        Ok(a) => a,
        Err(_) => return 0,
    };

    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(ts_attr))
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

pub async fn execute(topic: Option<String>, json: bool) -> Result<()> {
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

    let space_entity =
        Entity::from_str(&space_did).context("Failed to parse space DID as entity")?;

    let space_name = crate::metadata::SpaceMetadata::load(&space_did)
        .ok()
        .flatten()
        .map(|m| m.name);

    // If drilling into a specific topic
    if let Some(topic_name) = &topic {
        return execute_topic_drilldown(&branch, &space_entity, topic_name, json).await;
    }

    // Space-level summary: collect all topics and all items
    let idx_topic_attr = Attribute::from_str("idx/topic").context("Invalid attribute")?;
    let idx_item_attr = Attribute::from_str("idx/item").context("Invalid attribute")?;

    // Get all topics
    let topic_results: Vec<_> = branch
        .select(
            ArtifactSelector::new()
                .of(space_entity.clone())
                .the(idx_topic_attr),
        )
        .try_collect()
        .await?;

    let topics: Vec<String> = topic_results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect();

    // Get all item entities
    let item_results: Vec<_> = branch
        .select(
            ArtifactSelector::new()
                .of(space_entity.clone())
                .the(idx_item_attr),
        )
        .try_collect()
        .await?;

    let item_entities: Vec<Entity> = item_results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::Entity(e) => Some(e),
            _ => None,
        })
        .collect();

    let total_items = item_entities.len();

    // For each item, fetch topic, kind, and timestamp to build aggregates
    let topic_attr = Attribute::from_str("ctx/topic").context("Invalid attribute")?;
    let kind_attr = Attribute::from_str("ctx/kind").context("Invalid attribute")?;

    let mut topic_counts: HashMap<String, (usize, i64)> = HashMap::new(); // topic -> (count, latest_ts)
    let mut kind_counts: HashMap<String, usize> = HashMap::new();

    for entity in &item_entities {
        let item_topic = fetch_string(&branch, entity, &topic_attr, "general").await;
        let item_kind = fetch_string(&branch, entity, &kind_attr, "note").await;
        let item_ts = fetch_timestamp(&branch, entity).await;

        let entry = topic_counts.entry(item_topic).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(item_ts);

        *kind_counts.entry(item_kind).or_insert(0) += 1;
    }

    // Build sorted topic list
    let mut topic_list: Vec<_> = topics
        .iter()
        .map(|t| {
            let (count, latest) = topic_counts.get(t).copied().unwrap_or((0, 0));
            (t.clone(), count, latest)
        })
        .collect();
    topic_list.sort_by(|a, b| b.2.cmp(&a.2)); // Sort by latest timestamp desc

    if json {
        let json_topics: Vec<serde_json::Value> = topic_list
            .iter()
            .map(|(name, count, latest)| {
                serde_json::json!({
                    "name": name,
                    "items": count,
                    "latest": latest,
                })
            })
            .collect();

        let output = serde_json::json!({
            "space": {
                "did": space_did,
                "name": space_name,
            },
            "topics": json_topics,
            "kinds": kind_counts,
            "total_items": total_items,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        let display_name = space_name.as_deref().unwrap_or(&space_did);
        println!("Space: {}", display_name);
        println!("Total items: {}\n", total_items);

        if topic_list.is_empty() {
            println!("No topics yet. Use 'tonk remember' to store context.");
        } else {
            println!("Topics:");
            for (name, count, latest) in &topic_list {
                let ts_str = format_ts(*latest);
                println!("  {} ({} items, latest: {})", name, count, ts_str);
            }
        }

        if !kind_counts.is_empty() {
            println!("\nKinds:");
            let mut kinds: Vec<_> = kind_counts.iter().collect();
            kinds.sort_by(|a, b| b.1.cmp(a.1));
            for (kind, count) in kinds {
                println!("  {}: {}", kind, count);
            }
        }
        println!();
    }

    Ok(())
}

async fn execute_topic_drilldown<S: ArtifactStore>(
    store: &S,
    space_entity: &Entity,
    topic_name: &str,
    json: bool,
) -> Result<()> {
    let idx_item_attr = Attribute::from_str("idx/item").context("Invalid attribute")?;
    let topic_attr = Attribute::from_str("ctx/topic").context("Invalid attribute")?;
    let kind_attr = Attribute::from_str("ctx/kind").context("Invalid attribute")?;
    let summary_attr = Attribute::from_str("ctx/summary").context("Invalid attribute")?;
    let content_attr = Attribute::from_str("ctx/content").context("Invalid attribute")?;

    // Get all item entities
    let item_results: Vec<_> = store
        .select(
            ArtifactSelector::new()
                .of(space_entity.clone())
                .the(idx_item_attr),
        )
        .try_collect()
        .await?;

    let item_entities: Vec<Entity> = item_results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::Entity(e) => Some(e),
            _ => None,
        })
        .collect();

    // Filter to items in this topic and collect details
    let mut items: Vec<(String, String, i64, String)> = Vec::new(); // (id, kind, ts, summary)

    for entity in &item_entities {
        let item_topic = fetch_string(store, entity, &topic_attr, "general").await;
        if item_topic != topic_name {
            continue;
        }

        let item_kind = fetch_string(store, entity, &kind_attr, "note").await;
        let item_ts = fetch_timestamp(store, entity).await;

        // Try summary first, fall back to truncated content
        let summary = fetch_string(store, entity, &summary_attr, "").await;
        let display = if !summary.is_empty() {
            summary
        } else {
            let content = fetch_string(store, entity, &content_attr, "").await;
            if content.len() > 200 {
                format!("{}...", &content[..197])
            } else {
                content
            }
        };

        items.push((entity.to_string(), item_kind, item_ts, display));
    }

    // Sort by timestamp descending
    items.sort_by(|a, b| b.2.cmp(&a.2));

    if json {
        let json_items: Vec<serde_json::Value> = items
            .iter()
            .map(|(id, kind, ts, summary)| {
                serde_json::json!({
                    "id": id,
                    "kind": kind,
                    "timestamp": ts,
                    "summary": summary,
                })
            })
            .collect();

        let output = serde_json::json!({
            "topic": topic_name,
            "items": json_items,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else if items.is_empty() {
        println!("No items found under topic '{}'.", topic_name);
    } else {
        println!("Topic: {} ({} items)\n", topic_name, items.len());
        for (id, kind, ts, summary) in &items {
            println!("  [{}] {} ({})", kind, format_ts(*ts), id);
            println!("  {}\n", summary);
        }
    }

    Ok(())
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
