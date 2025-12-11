use crate::authority;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use dialog_artifacts::replica::{BranchId, Issuer, Replica};
use dialog_query::claim::{Attribute, Claim, Relation};
use dialog_query::{Entity, Session, Value};
use dialog_storage::FileSystemStorageBackend;
use futures_util::TryStreamExt;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::Keypair;

/// Type alias for the filesystem-backed storage
type FsBackend = FileSystemStorageBackend<Vec<u8>, Vec<u8>>;

/// Resolve an entity identifier to an Entity.
///
/// Rules:
/// - If starts with `~/` - derive by signing path with operator key, then blake3 hash, format as did:key
/// - If parses as a valid URI (Entity) - use as-is
/// - Otherwise - blake3 hash the input and format as did:key
fn resolve_entity(input: &str, operator: &Keypair) -> Result<Entity> {
    if input.starts_with("~/") {
        // Sign the path with operator key, then hash
        let path_bytes = input.as_bytes();
        let signature = operator.sign(path_bytes);
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

/// Get the storage path for the active space's facts database
fn get_active_space_storage_path() -> Result<PathBuf> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

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

    Ok(path)
}

/// Assert a fact into the active space
pub async fn assert(the: String, of: String, is: String) -> Result<()> {
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
    let storage_path = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let issuer = Issuer::from_secret(&operator.to_bytes());
    let replica = Replica::open(issuer, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let mut session = Session::open(branch);

    // Create and commit the fact
    let mut transaction = session.edit();
    let relation = Relation::new(attribute.clone(), entity.clone(), value.clone());
    relation.assert(&mut transaction);
    session.commit(transaction).await?;

    println!("✓ Asserted fact:");
    println!("  the: {}", the);
    println!("  of:  {} ({})", entity, of);
    println!("  is:  {:?}", value);

    Ok(())
}

/// Retract a fact from the active space
pub async fn retract(the: String, of: String, is: String) -> Result<()> {
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
    let storage_path = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let issuer = Issuer::from_secret(&operator.to_bytes());
    let replica = Replica::open(issuer, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;

    let mut session = Session::open(branch);

    // Create and commit the retraction
    let mut transaction = session.edit();
    let relation = Relation::new(attribute.clone(), entity.clone(), value.clone());
    relation.retract(&mut transaction);
    session.commit(transaction).await?;

    println!("✓ Retracted fact:");
    println!("  the: {}", the);
    println!("  of:  {} ({})", entity, of);
    println!("  is:  {:?}", value);

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
    let application = fact
        .compile()
        .context("Failed to compile query. At least one of --the, --of, or --is must be provided")?;

    // Get storage path and create session
    let storage_path = get_active_space_storage_path()?;
    let backend = FsBackend::new(&storage_path).await?;
    let issuer = Issuer::from_secret(&operator.to_bytes());
    let replica = Replica::open(issuer, backend)?;

    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.load(&branch_id).await?;

    let session = Session::open(branch);

    // Execute the query
    let results: Vec<dialog_query::Fact<Value>> =
        application.query(&session).try_collect().await?;

    if results.is_empty() {
        println!("No facts found matching criteria.");
        return Ok(());
    }

    println!("Found {} fact(s):\n", results.len());

    for result in results {
        // Extract fact fields based on variant
        let (the_val, of_val, is_val) = match &result {
            dialog_query::Fact::Assertion { the, of, is, .. } => {
                (the.to_string(), of.to_string(), format_value(is, byte_format))
            }
            dialog_query::Fact::Retraction { the, of, is, .. } => {
                (format!("!{}", the), of.to_string(), format_value(is, byte_format))
            }
        };

        println!("  the: {}", the_val);
        println!("  of:  {}", of_val);
        println!("  is:  {}", is_val);
        println!();
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
        ByteFormat::Text => {
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => format!("\"{}\"", s),
                Err(_) => format!("<{} bytes, invalid UTF-8>", bytes.len()),
            }
        }
        ByteFormat::Json => {
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => {
                    // Try to parse and pretty-print JSON
                    match serde_json::from_str::<serde_json::Value>(&s) {
                        Ok(json) => {
                            serde_json::to_string_pretty(&json)
                                .unwrap_or_else(|_| s)
                        }
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
                    serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|_| format!("{:?}", value))
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
                        ucan::delegation::subject::DelegatedSubject::Specific(did) => did.to_string(),
                        ucan::delegation::subject::DelegatedSubject::Any => "*".to_string(),
                    };
                    let exp = delegation.expiration()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "never".to_string());
                    let cmd = delegation.command().join("/");
                    let cmd_display = if cmd.is_empty() { "/".to_string() } else { format!("/{}", cmd) };
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
