use crate::authority;
use crate::crypto::Operator;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use base64::Engine as _;
use dialog_artifacts::repository::{BranchId, Credentials, Repository};
use dialog_query::claim::{Attribute, Claim, Relation};
use dialog_query::{Entity, Session, Value};
use ed25519_dalek::Signer;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::FsBackend;

/// Resolve an entity identifier to an Entity.
///
/// Rules:
/// - If starts with `~/` - derive by signing path with operator key, then blake3 hash, format as did:key
/// - If parses as a valid URI (Entity) - use as-is
/// - Otherwise - blake3 hash the input and format as did:key
fn resolve_entity(input: &str, operator: &Operator) -> Result<Entity> {
    if input.starts_with("~/") {
        // Sign the path with operator key, then hash
        let path_bytes = input.as_bytes();
        let signature = operator.signer().sign(path_bytes);
        let hash = blake3::hash(signature.to_bytes().as_ref());
        let hash_b58 = bs58::encode(hash.as_bytes()).into_string();
        let uri = format!("did:key:z{}", hash_b58);
        Entity::from_str(&uri).context(format!("Failed to create entity from path: {}", input))
    } else if let Ok(entity) = Entity::from_str(input) {
        // Valid URI, use as-is
        Ok(entity)
    } else {
        // Hash the input and format as did:key
        let hash = blake3::hash(input.as_bytes());
        let hash_b58 = bs58::encode(hash.as_bytes()).into_string();
        let uri = format!("did:key:z{}", hash_b58);
        Entity::from_str(&uri).context(format!("Failed to create entity from: {}", input))
    }
}

/// Parse a value from string input.
/// Tries to detect type: numbers or strings.
fn parse_value(input: &str) -> Value {
    // Try parsing as integer
    if let Ok(n) = input.parse::<i128>() {
        if n >= 0 {
            return Value::UnsignedInt(n as u128);
        } else {
            return Value::SignedInt(n);
        }
    }

    // Try parsing as float
    if let Ok(f) = input.parse::<f64>() {
        return Value::Float(f);
    }

    // Default to string
    Value::String(input.to_string())
}

/// Get the storage path and space DID for the active space's facts database
fn get_active_space_storage_path() -> Result<(PathBuf, String)> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.did().to_string();

    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;

    let space_did = state::get_active_space(&authority.did)?
        .context("No active space. Please run 'tonk space create' or 'tonk space select' first")?;

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

/// Convert a Value to a serde_json::Value for JSON output
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::UnsignedInt(n) => serde_json::json!(*n),
        Value::SignedInt(n) => serde_json::json!(*n),
        Value::Float(f) => serde_json::json!(*f),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Entity(e) => serde_json::Value::String(e.to_string()),
        Value::Symbol(s) => serde_json::json!({"symbol": s.to_string()}),
        Value::Bytes(b) => serde_json::json!({"bytes": base64::engine::general_purpose::STANDARD.encode(b)}),
        Value::Record(r) => serde_json::json!({"record": base64::engine::general_purpose::STANDARD.encode(r)}),
    }
}

/// Assert a fact into the active space
pub async fn assert(the: String, of: String, is: String, json: bool) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Resolve entity
    let entity = resolve_entity(&of, &operator)?;

    // Parse attribute
    let attribute =
        Attribute::from_str(&the).context(format!("Invalid attribute format: {}", the))?;

    // Parse value
    let value = parse_value(&is);

    // Get storage path and create session
    let (storage_path, space_did) = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let credentials = Credentials::from(&operator);
    let space_did_parsed: dialog_varsig::Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let replica = Repository::open(credentials, space_did_parsed, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let mut session = Session::open(branch);

    // Create and commit the fact
    let mut transaction = session.edit();
    let relation = Relation::new(attribute.clone(), entity.clone(), value.clone());
    relation.assert(&mut transaction);
    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "op": "assert",
            "the": the,
            "of": entity.to_string(),
            "is": value_to_json(&value),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("✓ Asserted fact:");
        println!("  the: {}", the);
        println!("  of:  {} ({})", entity, of);
        println!("  is:  {:?}", value);
    }

    Ok(())
}

/// Retract a fact from the active space
pub async fn retract(the: String, of: String, is: String, json: bool) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Resolve entity
    let entity = resolve_entity(&of, &operator)?;

    // Parse attribute
    let attribute =
        Attribute::from_str(&the).context(format!("Invalid attribute format: {}", the))?;

    // Parse value
    let value = parse_value(&is);

    // Get storage path and create session
    let (storage_path, space_did) = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let credentials = Credentials::from(&operator);
    let space_did_parsed: dialog_varsig::Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let replica = Repository::open(credentials, space_did_parsed, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let mut session = Session::open(branch);

    // Create and commit the retraction
    let mut transaction = session.edit();
    let relation = Relation::new(attribute.clone(), entity.clone(), value.clone());
    relation.retract(&mut transaction);
    session.commit(transaction).await?;

    if json {
        let output = serde_json::json!({
            "ok": true,
            "op": "retract",
            "the": the,
            "of": entity.to_string(),
            "is": value_to_json(&value),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("✓ Retracted fact:");
        println!("  the: {}", the);
        println!("  of:  {} ({})", entity, of);
        println!("  is:  {:?}", value);
    }

    Ok(())
}

/// Supported byte format types
#[derive(Debug, Clone, Copy, PartialEq)]
enum ByteFormat {
    /// Show as <N bytes>
    Default,
    /// Decode as CBOR and pretty-print
    Cbor,
    /// Decode as JSON and pretty-print
    Json,
    /// Decode as UTF-8 text
    Text,
    /// Decode as UCAN delegation
    Ucan,
}

impl ByteFormat {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cbor" | "application/cbor" => ByteFormat::Cbor,
            "json" | "application/json" => ByteFormat::Json,
            "text" | "text/plain" => ByteFormat::Text,
            "ucan" => ByteFormat::Ucan,
            _ => ByteFormat::Default,
        }
    }
}

/// Find facts in the active space matching the given criteria
pub async fn find(
    the: Option<String>,
    of: Option<String>,
    is: Option<String>,
    format: Option<String>,
    json: bool,
) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Parse format option
    let byte_format = format
        .as_ref()
        .map(|f| ByteFormat::from_str(f))
        .unwrap_or(ByteFormat::Default);

    // Build the query using Fact::select()
    let mut fact = dialog_query::Fact::<Value>::select();

    // Set attribute constraint if provided
    if let Some(the_str) = &the {
        fact = fact.the(the_str.as_str());
    }

    // Set entity constraint if provided
    if let Some(of_str) = &of {
        let entity = resolve_entity(of_str, &operator)?;
        fact = fact.of(entity);
    }

    // Set value constraint if provided
    if let Some(is_str) = &is {
        let value = parse_value(is_str);
        fact = fact.is(value);
    }

    // Compile the query - this will fail if no constraints provided
    let application = fact.compile().context(
        "Failed to compile query. At least one of --the, --of, or --is must be provided",
    )?;

    // Get storage path and create session
    let (storage_path, space_did) = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let credentials = Credentials::from(&operator);
    let space_did_parsed: dialog_varsig::Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let replica = Repository::open(credentials, space_did_parsed, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let session = Session::open(branch);

    // Execute the query
    let results: Vec<dialog_query::Fact<Value>> = application.query(&session).try_collect().await?;

    if json {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|result| match result {
                dialog_query::Fact::Assertion { the, of, is, .. } => {
                    serde_json::json!({
                        "type": "assertion",
                        "the": the.to_string(),
                        "of": of.to_string(),
                        "is": value_to_json(is),
                    })
                }
                dialog_query::Fact::Retraction { the, of, is, .. } => {
                    serde_json::json!({
                        "type": "retraction",
                        "the": the.to_string(),
                        "of": of.to_string(),
                        "is": value_to_json(is),
                    })
                }
            })
            .collect();
        println!("{}", serde_json::to_string(&json_results)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No facts found matching criteria.");
        return Ok(());
    }

    println!("Found {} fact(s):\n", results.len());

    for result in results {
        // Extract fact fields based on variant
        let (the_val, of_val, is_val) = match &result {
            dialog_query::Fact::Assertion { the, of, is, .. } => (
                the.to_string(),
                of.to_string(),
                format_value(is, byte_format),
            ),
            dialog_query::Fact::Retraction { the, of, is, .. } => (
                format!("!{}", the),
                of.to_string(),
                format_value(is, byte_format),
            ),
        };

        println!("  the: {}", the_val);
        println!("  of:  {}", of_val);
        println!("  is:  {}", is_val);
        println!();
    }

    Ok(())
}

/// A batch operation from JSON input
#[derive(Debug, Deserialize)]
struct BatchOp {
    op: String,
    the: String,
    of: String,
    is: String,
}

/// Result of a single batch operation
#[derive(Debug, Serialize)]
struct BatchResult {
    ok: bool,
    op: String,
    the: String,
    of: String,
    is: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Batch assert/retract facts from stdin (JSON lines).
/// Each line: {"op": "assert"|"retract", "the": "...", "of": "...", "is": "..."}
pub async fn batch(json: bool) -> Result<()> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Get storage path and create session
    let (storage_path, space_did) = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let credentials = Credentials::from(&operator);
    let replica = Repository::open(credentials, space_did.into(), backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let mut session = Session::open(branch);

    // Read lines from stdin
    let stdin = std::io::stdin();
    let lines: Vec<String> = stdin.lock().lines().collect::<Result<Vec<_>, _>>()?;

    let mut results: Vec<BatchResult> = Vec::new();
    let mut transaction = session.edit();

    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let batch_op: BatchOp = match serde_json::from_str(line) {
            Ok(op) => op,
            Err(e) => {
                results.push(BatchResult {
                    ok: false,
                    op: "unknown".to_string(),
                    the: String::new(),
                    of: String::new(),
                    is: serde_json::Value::Null,
                    error: Some(format!("Parse error: {}", e)),
                });
                continue;
            }
        };

        let entity = match resolve_entity(&batch_op.of, &operator) {
            Ok(e) => e,
            Err(e) => {
                results.push(BatchResult {
                    ok: false,
                    op: batch_op.op.clone(),
                    the: batch_op.the.clone(),
                    of: batch_op.of.clone(),
                    is: serde_json::Value::String(batch_op.is.clone()),
                    error: Some(format!("Entity error: {}", e)),
                });
                continue;
            }
        };

        let attribute = match Attribute::from_str(&batch_op.the) {
            Ok(a) => a,
            Err(e) => {
                results.push(BatchResult {
                    ok: false,
                    op: batch_op.op.clone(),
                    the: batch_op.the.clone(),
                    of: batch_op.of.clone(),
                    is: serde_json::Value::String(batch_op.is.clone()),
                    error: Some(format!("Attribute error: {}", e)),
                });
                continue;
            }
        };

        let value = parse_value(&batch_op.is);
        let relation = Relation::new(attribute, entity.clone(), value.clone());

        match batch_op.op.as_str() {
            "assert" => {
                relation.assert(&mut transaction);
                results.push(BatchResult {
                    ok: true,
                    op: "assert".to_string(),
                    the: batch_op.the,
                    of: entity.to_string(),
                    is: value_to_json(&value),
                    error: None,
                });
            }
            "retract" => {
                relation.retract(&mut transaction);
                results.push(BatchResult {
                    ok: true,
                    op: "retract".to_string(),
                    the: batch_op.the,
                    of: entity.to_string(),
                    is: value_to_json(&value),
                    error: None,
                });
            }
            other => {
                results.push(BatchResult {
                    ok: false,
                    op: other.to_string(),
                    the: batch_op.the,
                    of: batch_op.of,
                    is: serde_json::Value::String(batch_op.is),
                    error: Some(format!("Unknown operation: {}. Use 'assert' or 'retract'", other)),
                });
            }
        }
    }

    // Commit all operations in a single transaction
    session.commit(transaction).await?;

    if json {
        println!("{}", serde_json::to_string(&results)?);
    } else {
        let ok_count = results.iter().filter(|r| r.ok).count();
        let err_count = results.iter().filter(|r| !r.ok).count();
        println!("Batch complete: {} succeeded, {} failed", ok_count, err_count);
        for result in &results {
            if !result.ok {
                println!("  ERROR: {} - {}", result.the, result.error.as_deref().unwrap_or("unknown"));
            }
        }
    }

    Ok(())
}

/// Format a Value for display
fn format_value(value: &Value, byte_format: ByteFormat) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s),
        Value::UnsignedInt(n) => n.to_string(),
        Value::SignedInt(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bytes(b) => format_bytes(b, byte_format),
        Value::Entity(e) => e.to_string(),
        Value::Symbol(s) => format!(":{}", s),
        Value::Boolean(b) => b.to_string(),
        Value::Record(r) => format_bytes(r, byte_format),
    }
}

/// Format bytes according to the specified format
fn format_bytes(bytes: &[u8], format: ByteFormat) -> String {
    match format {
        ByteFormat::Default => format!("<{} bytes>", bytes.len()),
        ByteFormat::Text => match String::from_utf8(bytes.to_vec()) {
            Ok(s) => format!("\"{}\"", s),
            Err(_) => format!("<{} bytes, invalid UTF-8>", bytes.len()),
        },
        ByteFormat::Json => {
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => {
                    // Try to parse and pretty-print JSON
                    match serde_json::from_str::<serde_json::Value>(&s) {
                        Ok(json) => serde_json::to_string_pretty(&json).unwrap_or(s),
                        Err(_) => format!("<{} bytes, invalid JSON>", bytes.len()),
                    }
                }
                Err(_) => format!("<{} bytes, invalid UTF-8>", bytes.len()),
            }
        }
        ByteFormat::Cbor => {
            // Try to decode CBOR and display as JSON
            // First try generic serde_json::Value, then fall back to hex
            match serde_ipld_dagcbor::from_slice::<serde_json::Value>(bytes) {
                Ok(value) => {
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{:?}", value))
                }
                Err(_) => {
                    // CBOR with specialized types (like UCANs) can't be decoded to JSON
                    // Show as hex which can be decoded with external tools
                    format!("0x{}", hex::encode(bytes))
                }
            }
        }
        ByteFormat::Ucan => {
            // Try to decode as UCAN delegation
            match crate::delegation::Delegation::from_cbor_bytes(bytes) {
                Ok(delegation) => {
                    let subject = match delegation.subject() {
                        dialog_ucan::subject::Subject::Specific(did) => did.to_string(),
                        dialog_ucan::subject::Subject::Any => "*".to_string(),
                    };
                    let exp = delegation
                        .expiration()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "never".to_string());
                    let cmd_display = delegation.command_str();
                    format!(
                        "UCAN {{\n  iss: {},\n  aud: {},\n  sub: {},\n  cmd: {},\n  exp: {}\n}}",
                        delegation.issuer(),
                        delegation.audience(),
                        subject,
                        cmd_display,
                        exp
                    )
                }
                Err(_) => format!("<{} bytes, invalid UCAN>", bytes.len()),
            }
        }
    }
}
