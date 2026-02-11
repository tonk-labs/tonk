//! Shared helpers for the concepts & facts system.
//!
//! Provides deterministic entity derivation, attribute name prefixing,
//! and common storage access patterns used by both `concept` and `instance` modules.

use crate::authority;
use crate::crypto::Operator;
use crate::keystore::Keystore;
use crate::state;
use anyhow::{Context, Result};
use base64::Engine as _;
use dialog_artifacts::repository::{BranchId, Credentials, Repository};
use dialog_artifacts::{ArtifactSelector, ArtifactStore};
use dialog_query::claim::Attribute;
use dialog_query::{Entity, Session, Value};
use futures_util::TryStreamExt;
use std::path::PathBuf;
use std::str::FromStr;
use tonk_space::FsBackend;

// ---------------------------------------------------------------------------
// Name validation
// ---------------------------------------------------------------------------

/// Validate that a name contains only safe characters: letters, digits,
/// hyphens, and underscores. Used for both concept and rule names.
pub fn validate_safe_name(name: &str, kind: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("{} name cannot be empty", kind);
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Invalid {} name '{}'. Names may only contain letters, digits, hyphens, and underscores.",
            kind,
            name
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ConceptName newtype
// ---------------------------------------------------------------------------

/// A validated concept name (alphanumeric, hyphens, underscores only).
///
/// Concept names are used as attribute namespace prefixes (e.g. `"task/title"`),
/// so they must only contain safe characters. This type guarantees that
/// invariant at construction time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConceptName(String);

impl ConceptName {
    /// Create a new `ConceptName`, validating that it contains only safe characters.
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_safe_name(&s, "Concept")?;
        Ok(Self(s))
    }

    /// Create from a name already stored in the database (skips validation).
    ///
    /// Use this only for names read back from storage that were validated
    /// at write time.
    pub fn from_stored(s: String) -> Self {
        Self(s)
    }

    /// The lowercase form used for entity derivation and attribute namespacing.
    pub fn to_lowercase(&self) -> String {
        self.0.to_lowercase()
    }

    /// The original name as stored.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConceptName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for ConceptName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ConceptName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Well-known attribute constants
// ---------------------------------------------------------------------------

/// Registry attribute: points from registry entity to concept entities.
pub const ATTR_REGISTRY_CONCEPT: &str = "registry/concept";

/// Concept attribute: the human-readable name of the concept.
pub const ATTR_CONCEPT_NAME: &str = "concept/name";

/// Concept attribute: optional description.
pub const ATTR_CONCEPT_DESCRIPTION: &str = "concept/description";

/// Concept attribute: one per attribute the concept has (multi-valued).
pub const ATTR_CONCEPT_ATTRIBUTE: &str = "concept/attribute";

/// Concept attribute: back-reference to instances (multi-valued).
pub const ATTR_CONCEPT_INSTANCE: &str = "concept/instance";

/// Instance attribute: type reference back to concept entity.
pub const ATTR_INSTANCE_TYPE: &str = "instance/type";

/// Instance attribute: creation timestamp.
pub const ATTR_INSTANCE_CREATED: &str = "instance/created";

/// Attribute metadata: human-readable description of the attribute.
pub const ATTR_ATTRIBUTE_DESCRIPTION: &str = "attribute/description";

/// Attribute metadata: type constraint (e.g. "Text", "Integer", "RecipeStep", or JSON array for enums).
pub const ATTR_ATTRIBUTE_TYPE: &str = "attribute/type";

/// Attribute metadata: cardinality ("many" for multi-valued, absent for single).
pub const ATTR_ATTRIBUTE_CARDINALITY: &str = "attribute/cardinality";

/// Attribute metadata: whether the attribute is optional.
pub const ATTR_ATTRIBUTE_OPTIONAL: &str = "attribute/optional";

/// Concept attribute: the namespace the concept was imported from (e.g. "diy.cook").
pub const ATTR_CONCEPT_NAMESPACE: &str = "concept/namespace";

/// Registry attribute: points from registry entity to rule entities.
pub const ATTR_REGISTRY_RULE: &str = "registry/rule";

/// Rule attribute: the human-readable name of the rule.
pub const ATTR_RULE_NAME: &str = "rule/name";

/// Rule attribute: optional description.
pub const ATTR_RULE_DESCRIPTION: &str = "rule/description";

/// Rule attribute: name of the conclusion concept.
pub const ATTR_RULE_CONCLUSION: &str = "rule/conclusion";

/// Rule attribute: JSON-serialized rule definition.
pub const ATTR_RULE_DEFINITION: &str = "rule/definition";

// ---------------------------------------------------------------------------
// Deterministic entity derivation
// ---------------------------------------------------------------------------

/// Derive the concept registry entity for a space.
///
/// `registry_entity = did:key:z{base58(blake3(space_did + "\0concept-registry"))}`
pub fn registry_entity(space_did: &str) -> Result<Entity> {
    derive_entity(&format!("{}\0concept-registry", space_did))
}

/// Derive the concept entity for a given concept name within a space.
///
/// `concept_entity = did:key:z{base58(blake3(space_did + "\0concept\0" + lowercase_name))}`
pub fn concept_entity(space_did: &str, concept_name: &ConceptName) -> Result<Entity> {
    derive_entity(&format!(
        "{}\0concept\0{}",
        space_did,
        concept_name.to_lowercase()
    ))
}

/// Derive the rule entity for a given rule name within a space.
///
/// `rule_entity = did:key:z{base58(blake3(space_did + "\0rule\0" + lowercase_name))}`
pub fn rule_entity(space_did: &str, rule_name: &str) -> Result<Entity> {
    derive_entity(&format!(
        "{}\0rule\0{}",
        space_did,
        rule_name.to_lowercase()
    ))
}

/// Derive an attribute metadata entity for a concept + attribute pair.
///
/// Reserved for future use (attribute descriptions, types, cardinality).
pub fn attribute_meta_entity(
    space_did: &str,
    concept_name: &ConceptName,
    attr_name: &str,
) -> Result<Entity> {
    derive_entity(&format!(
        "{}\0attr-meta\0{}\0{}",
        space_did,
        concept_name.to_lowercase(),
        attr_name
    ))
}

/// Low-level: hash input to produce a deterministic `did:key` entity.
fn derive_entity(input: &str) -> Result<Entity> {
    let hash = blake3::hash(input.as_bytes());
    let b58 = bs58::encode(hash.as_bytes()).into_string();
    let uri = format!("did:key:z{}", b58);
    Entity::from_str(&uri).context("Failed to derive entity")
}

// ---------------------------------------------------------------------------
// Attribute name helpers
// ---------------------------------------------------------------------------

/// Given a concept name and user-supplied attribute key, produce the fully
/// qualified attribute name `{concept_lower}/{key}`.
///
/// If the key already contains a `/` and the prefix matches the concept
/// namespace, it is returned as-is. If the prefix doesn't match, an error
/// is returned.
pub fn qualify_attribute(concept_name: &ConceptName, key: &str) -> Result<String> {
    let prefix = concept_name.to_lowercase();
    if let Some((_ns, _name)) = key.split_once('/') {
        let ns = _ns.to_lowercase();
        if ns == prefix {
            Ok(key.to_string())
        } else {
            anyhow::bail!(
                "Attribute '{}' has namespace '{}' but concept expects '{}'",
                key,
                ns,
                prefix
            );
        }
    } else {
        Ok(format!("{}/{}", prefix, key))
    }
}

/// Strip the concept namespace prefix from an attribute name, returning
/// just the short key (e.g. `"task/title"` -> `"title"`).
pub fn short_attribute(concept_name: &ConceptName, attr: &str) -> String {
    let prefix = format!("{}/", concept_name.to_lowercase());
    if let Some(short) = attr.strip_prefix(&prefix) {
        short.to_string()
    } else {
        attr.to_string()
    }
}

// ---------------------------------------------------------------------------
// Storage access helpers
// ---------------------------------------------------------------------------

/// Information about the active space needed to open storage.
pub struct SpaceContext {
    pub storage_path: PathBuf,
    pub space_did: String,
    pub operator: Operator,
}

/// Get the storage path, space DID, and operator for the active space.
pub fn get_space_context() -> Result<SpaceContext> {
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
    let storage_path = home
        .join(".tonk")
        .join("operator")
        .join(&operator_did)
        .join("session")
        .join(&authority.did)
        .join("space")
        .join(&space_did)
        .join("facts");

    Ok(SpaceContext {
        storage_path,
        space_did,
        operator,
    })
}

/// Open a `Session` for the active space's dialog-db.
pub async fn open_session(
    ctx: &SpaceContext,
) -> Result<Session<dialog_artifacts::repository::Branch<FsBackend>>> {
    let backend = FsBackend::new(&ctx.storage_path).await?;
    let credentials = Credentials::from(&ctx.operator);
    let space_did_parsed: dialog_varsig::Did = ctx
        .space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let replica = Repository::open(credentials, space_did_parsed, backend)?;
    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;
    Ok(Session::open(branch))
}

/// Open a raw Branch for the active space's dialog-db (for lower-level operations).
pub async fn open_branch(
    ctx: &SpaceContext,
) -> Result<dialog_artifacts::repository::Branch<FsBackend>> {
    let backend = FsBackend::new(&ctx.storage_path).await?;
    let credentials = Credentials::from(&ctx.operator);
    let space_did_parsed: dialog_varsig::Did = ctx
        .space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let replica = Repository::open(credentials, space_did_parsed, backend)?;
    let branch_id = BranchId::new("main".to_string());
    let branch = replica.branches.open(&branch_id).await?;
    Ok(branch)
}

// ---------------------------------------------------------------------------
// Common query helpers
// ---------------------------------------------------------------------------

/// Fetch all string values for a multi-valued attribute on an entity.
pub async fn fetch_string_values<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr_name: &str,
) -> Result<Vec<String>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr))
        .try_collect()
        .await?;

    Ok(results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// Fetch a single string value for an attribute on an entity.
pub async fn fetch_string<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr_name: &str,
) -> Result<Option<String>> {
    let values = fetch_string_values(store, entity, attr_name).await?;
    Ok(values.into_iter().next())
}

/// Fetch all entity values for a multi-valued attribute.
pub async fn fetch_entity_values<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr_name: &str,
) -> Result<Vec<Entity>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr))
        .try_collect()
        .await?;

    Ok(results
        .into_iter()
        .filter_map(|a| match a.is {
            Value::Entity(e) => Some(e),
            _ => None,
        })
        .collect())
}

/// Fetch a single Value for an attribute on an entity.
pub async fn fetch_value<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr_name: &str,
) -> Result<Option<Value>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr))
        .try_collect()
        .await?;

    Ok(results.into_iter().next().map(|a| a.is))
}

/// Fetch all Values for a multi-valued attribute on an entity.
pub async fn fetch_values<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr_name: &str,
) -> Result<Vec<Value>> {
    let attr = Attribute::from_str(attr_name).context("Invalid attribute")?;
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr))
        .try_collect()
        .await?;

    Ok(results.into_iter().map(|a| a.is).collect())
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

/// Parse a string input into a Value, trying integer -> float -> string.
pub fn parse_value(input: &str) -> Value {
    // Entity references (DID URIs)
    if input.starts_with("did:")
        && let Ok(entity) = dialog_query::Entity::from_str(input)
    {
        return Value::Entity(entity);
    }
    if let Ok(n) = input.parse::<i128>() {
        if n >= 0 {
            return Value::UnsignedInt(n as u128);
        } else {
            return Value::SignedInt(n);
        }
    }
    if let Ok(f) = input.parse::<f64>() {
        return Value::Float(f);
    }
    // Handle booleans
    match input.to_lowercase().as_str() {
        "true" => return Value::Boolean(true),
        "false" => return Value::Boolean(false),
        _ => {}
    }
    Value::String(input.to_string())
}

/// Convert a Value to a serde_json::Value.
pub fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::UnsignedInt(n) => serde_json::json!(*n),
        Value::SignedInt(n) => serde_json::json!(*n),
        Value::Float(f) => serde_json::json!(*f),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Entity(e) => serde_json::Value::String(e.to_string()),
        Value::Symbol(s) => serde_json::json!({"symbol": s.to_string()}),
        Value::Bytes(b) => {
            serde_json::json!({"bytes": base64::engine::general_purpose::STANDARD.encode(b)})
        }
        Value::Record(r) => {
            serde_json::json!({"record": base64::engine::general_purpose::STANDARD.encode(r)})
        }
    }
}

/// Format a Value for human-readable display.
pub fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::UnsignedInt(n) => n.to_string(),
        Value::SignedInt(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Entity(e) => e.to_string(),
        Value::Symbol(s) => format!(":{}", s),
        Value::Bytes(b) => format!("<{} bytes>", b.len()),
        Value::Record(r) => format!("<{} bytes record>", r.len()),
    }
}

// ---------------------------------------------------------------------------
// Dynamic concept construction (for rule compilation)
// ---------------------------------------------------------------------------

/// Leak a runtime string to get a `&'static str`.
///
/// This is safe for CLI tools that run once and exit. The leaked memory
/// lives for the process lifetime, which is exactly what `AttributeSchema`
/// needs for its `&'static str` fields.
///
/// # Safety note
///
/// Intentionally leaks memory for CLI single-run usage. Each call leaks
/// one small `String` allocation that lives until process exit.
pub fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Build a `dialog_query::predicate::Concept` dynamically from a list of
/// fully-qualified attribute names (e.g. `["task/title", "task/status"]`).
///
/// Each attribute is split on `/` to extract namespace and name, then
/// wrapped in an `AttributeSchema<Value>` with `Type::String` (the
/// type doesn't affect rule evaluation, only schema validation).
pub fn build_dynamic_concept(attributes: &[String]) -> Result<dialog_query::predicate::Concept> {
    use dialog_query::{AttributeSchema, Type};

    let attr_schemas: Vec<(&str, AttributeSchema<Value>)> = attributes
        .iter()
        .map(|attr| {
            let (ns, name) = attr.split_once('/').ok_or_else(|| {
                anyhow::anyhow!(
                    "Malformed attribute '{}': expected 'namespace/name' format",
                    attr
                )
            })?;
            let short_name = leak_str(name);
            let schema = AttributeSchema::<Value>::new(
                leak_str(ns),
                short_name,
                leak_str(""), // description
                Type::String, // default type; rule eval doesn't enforce types
            );
            Ok((short_name, schema))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(dialog_query::predicate::Concept::new(attr_schemas.into()))
}
