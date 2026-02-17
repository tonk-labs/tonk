use crate::crypto::Operator;
use crate::schema::{derive_entity_from_hash, get_space_context, open_session, parse_value, value_to_json};
use anyhow::{Context, Result};

use dialog_query::claim::{Attribute, Claim, Relation};
use dialog_query::{Entity, Value};
use ed25519_dalek::Signer;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::str::FromStr;

/// Resolve an entity identifier to an Entity.
///
/// Rules:
/// - If starts with `~/` - derive by signing path with operator key, then
///   blake3 hash, and format as a proper Ed25519 `did:key`.
/// - If parses as a valid URI (Entity) - use as-is.
/// - Otherwise - blake3 hash the input and format as Ed25519 `did:key`.
fn resolve_entity(input: &str, operator: &Operator) -> Result<Entity> {
    if input.starts_with("~/") {
        let signature = operator.signer().sign(input.as_bytes());
        let hash = blake3::hash(signature.to_bytes().as_ref());
        derive_entity_from_hash(&hash)
            .context(format!("Failed to create entity from path: {}", input))
    } else if let Ok(entity) = Entity::from_str(input) {
        Ok(entity)
    } else {
        let hash = blake3::hash(input.as_bytes());
        derive_entity_from_hash(&hash).context(format!("Failed to create entity from: {}", input))
    }
}

/// Assert a fact into the active space
pub async fn assert(the: String, of: String, is: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut session = open_session(&ctx).await?;

    let entity = resolve_entity(&of, &ctx.operator)?;
    let attribute =
        Attribute::from_str(&the).context(format!("Invalid attribute format: {}", the))?;
    let value = parse_value(&is);

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
        println!("Asserted fact:");
        println!("  the: {}", the);
        println!("  of:  {} ({})", entity, of);
        println!("  is:  {:?}", value);
    }

    Ok(())
}

/// Retract a fact from the active space
pub async fn retract(the: String, of: String, is: String, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut session = open_session(&ctx).await?;

    let entity = resolve_entity(&of, &ctx.operator)?;
    let attribute =
        Attribute::from_str(&the).context(format!("Invalid attribute format: {}", the))?;
    let value = parse_value(&is);

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
        println!("Retracted fact:");
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
    let ctx = get_space_context()?;

    // Parse format option
    let byte_format = format
        .as_ref()
        .map(|f| ByteFormat::from_str(f))
        .unwrap_or(ByteFormat::Default);

    // Build the query using Fact::select()
    let mut fact = dialog_query::Fact::<Value>::select();

    if let Some(the_str) = &the {
        fact = fact.the(the_str.as_str());
    }

    if let Some(of_str) = &of {
        let entity = resolve_entity(of_str, &ctx.operator)?;
        fact = fact.of(entity);
    }

    if let Some(is_str) = &is {
        let value = parse_value(is_str);
        fact = fact.is(value);
    }

    let application = fact.compile().context(
        "Failed to compile query. At least one of --the, --of, or --is must be provided",
    )?;

    let session = open_session(&ctx).await?;

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

/// Default operation for batch ops (used when `op` is absent in YAML input)
fn default_op() -> String {
    "assert".to_string()
}

/// Deserialize the `is` field directly into a `dialog_query::Value`.
///
/// Maps serde scalars to `Value` variants without an intermediate string
/// representation, so `is: 42` becomes `Value::UnsignedInt(42)` and
/// `is: "hello"` becomes `Value::String("hello")`. Works with both
/// serde_yaml and serde_json.
fn deserialize_is_as_value<'de, D>(deserializer: D) -> std::result::Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ValueVisitor;

    impl<'de> serde::de::Visitor<'de> for ValueVisitor {
        type Value = dialog_query::Value;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string, number, or boolean")
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
            Ok(Value::String(v.to_string()))
        }

        fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
            Ok(Value::Boolean(v))
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
            if v >= 0 {
                Ok(Value::UnsignedInt(v as u128))
            } else {
                Ok(Value::SignedInt(v as i128))
            }
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
            Ok(Value::UnsignedInt(v as u128))
        }

        fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
            Ok(Value::Float(v))
        }
    }

    deserializer.deserialize_any(ValueVisitor)
}

/// A batch operation from JSON or YAML input
#[derive(Debug, Deserialize)]
struct BatchOp {
    #[serde(default = "default_op")]
    op: String,
    the: String,
    of: String,
    #[serde(deserialize_with = "deserialize_is_as_value")]
    is: Value,
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

/// Batch assert/retract facts from a YAML file or stdin (JSON lines).
///
/// When `file` is `Some(path)`, reads and parses the file as a YAML array of
/// `{the, of, is, op?}` objects. When `file` is `None`, reads JSON Lines from
/// stdin where each line is `{op, the, of, is}`.
///
/// All operations are committed atomically — if any operation fails, the
/// entire batch is aborted with no changes committed.
///
/// Error reporting differs by input mode:
/// - **YAML**: The file is parsed as a single unit. If parsing fails, the
///   function returns an error immediately with no per-operation results.
/// - **JSON Lines**: Each line is parsed independently. Parse errors are
///   collected as individual `BatchResult` entries alongside successful
///   operations, then the batch is aborted if any errors occurred.
pub async fn batch(file: Option<String>, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let mut session = open_session(&ctx).await?;

    let mut results: Vec<BatchResult> = Vec::new();
    let mut transaction = session.edit();

    // Parse batch operations from either a YAML file or stdin JSON Lines
    let batch_ops: Vec<BatchOp> = if let Some(ref path) = file {
        // --file path: read and parse as YAML array
        let content =
            std::fs::read_to_string(path).context(format!("Failed to read file: {}", path))?;
        serde_yaml::from_str(&content).context(format!("Failed to parse YAML from: {}", path))?
    } else {
        // stdin: read JSON Lines, collecting parse errors into results
        let stdin = std::io::stdin();
        let lines: Vec<String> = stdin.lock().lines().collect::<Result<Vec<_>, _>>()?;
        let mut ops = Vec::new();

        for line in &lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<BatchOp>(line) {
                Ok(op) => ops.push(op),
                Err(e) => {
                    results.push(BatchResult {
                        ok: false,
                        op: "unknown".to_string(),
                        the: String::new(),
                        of: String::new(),
                        is: serde_json::Value::Null,
                        error: Some(format!("Parse error: {}", e)),
                    });
                }
            }
        }

        ops
    };

    // Process each batch operation
    for batch_op in batch_ops {
        let entity = match resolve_entity(&batch_op.of, &ctx.operator) {
            Ok(e) => e,
            Err(e) => {
                results.push(BatchResult {
                    ok: false,
                    op: batch_op.op.clone(),
                    the: batch_op.the.clone(),
                    of: batch_op.of.clone(),
                    is: value_to_json(&batch_op.is),
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
                    is: value_to_json(&batch_op.is),
                    error: Some(format!("Attribute error: {}", e)),
                });
                continue;
            }
        };

        let value = &batch_op.is;
        let relation = Relation::new(attribute, entity.clone(), value.clone());

        match batch_op.op.as_str() {
            "assert" => {
                relation.assert(&mut transaction);
                results.push(BatchResult {
                    ok: true,
                    op: "assert".to_string(),
                    the: batch_op.the,
                    of: entity.to_string(),
                    is: value_to_json(value),
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
                    is: value_to_json(value),
                    error: None,
                });
            }
            other => {
                results.push(BatchResult {
                    ok: false,
                    op: other.to_string(),
                    the: batch_op.the,
                    of: batch_op.of,
                    is: value_to_json(&batch_op.is),
                    error: Some(format!(
                        "Unknown operation: {}. Use 'assert' or 'retract'",
                        other
                    )),
                });
            }
        }
    }

    let ok_count = results.iter().filter(|r| r.ok).count();
    let err_count = results.iter().filter(|r| !r.ok).count();

    // If any operations failed, abort the entire batch without committing.
    // The transaction is dropped, so no partial changes are persisted.
    if err_count > 0 {
        if json {
            println!("{}", serde_json::to_string(&results)?);
        } else {
            println!(
                "Batch aborted: {} succeeded, {} failed (no changes committed)",
                ok_count, err_count
            );
            for result in &results {
                if !result.ok {
                    println!(
                        "  ERROR: {} - {}",
                        result.the,
                        result.error.as_deref().unwrap_or("unknown")
                    );
                }
            }
        }
        anyhow::bail!(
            "Batch aborted due to {} error(s). No changes were committed.",
            err_count
        );
    }

    // All operations parsed successfully — commit atomically
    session.commit(transaction).await?;

    if json {
        println!("{}", serde_json::to_string(&results)?);
    } else {
        println!("Batch complete: {} operations committed", ok_count);
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
                    match serde_json::from_str::<serde_json::Value>(&s) {
                        Ok(json) => serde_json::to_string_pretty(&json).unwrap_or(s),
                        Err(_) => format!("<{} bytes, invalid JSON>", bytes.len()),
                    }
                }
                Err(_) => format!("<{} bytes, invalid UTF-8>", bytes.len()),
            }
        }
        ByteFormat::Cbor => {
            match serde_ipld_dagcbor::from_slice::<serde_json::Value>(bytes) {
                Ok(value) => {
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{:?}", value))
                }
                Err(_) => {
                    format!("0x{}", hex::encode(bytes))
                }
            }
        }
        ByteFormat::Ucan => {
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
