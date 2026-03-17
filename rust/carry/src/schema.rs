//! Shared helpers for the concepts & facts system.
//!
//! Provides deterministic entity derivation, attribute name prefixing,
//! and common storage access patterns.

use anyhow::{Context, Result};
use base64::Engine as _;
use dialog_artifacts::{ArtifactSelector, ArtifactStore};
pub use dialog_query::claim::Attribute as ClaimAttribute;
use dialog_query::concept::Concept as _;
use dialog_query::{Cardinality, Entity, Value};
use futures_util::TryStreamExt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Typed meta-schema for registered concepts
// ---------------------------------------------------------------------------

/// Meta-schema attributes for registered concepts.
///
/// These model "the concept of a concept" as a typed schema using
/// dialog-query's `#[derive(Attribute)]` system. The module name
/// `concept` becomes the attribute namespace, producing selectors
/// like `concept/name`, `concept/attribute`, etc.
pub mod concept {
    /// The human-readable name of the concept.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    pub struct Name(pub String);

    /// Description of the concept.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    pub struct Description(pub String);

    /// A fully-qualified attribute belonging to the concept (multi-valued).
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[cardinality(many)]
    pub struct Attribute(pub String);

    /// The namespace the concept belongs to.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    pub struct Namespace(pub String);

    /// Entity ID of the prior concept (for schema evolution tracking).
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    pub struct Prior(pub String);

    /// Rationale for updating a concept.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    pub struct UpdateRationale(pub String);
}

/// A registered concept in the space, modeled as a typed concept.
///
/// Because the `attribute` field has `Cardinality::Many`, querying
/// `Match::<RegisteredConcept>` returns one row per attribute value
/// (join semantics). Callers should deduplicate by entity.
#[derive(dialog_query::Concept, Debug, Clone)]
#[allow(dead_code)]
pub struct RegisteredConcept {
    pub this: Entity,
    pub name: concept::Name,
    pub description: concept::Description,
    pub namespace: concept::Namespace,
    pub attribute: concept::Attribute,
}

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
/// Concept names are labels for concepts — they are decoupled from attribute
/// namespaces. This type guarantees the name contains only safe characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConceptName(String);

impl ConceptName {
    /// Create a new `ConceptName`, validating that it contains only safe characters.
    ///
    /// The name is normalized to lowercase for consistent storage and
    /// exact-match lookups. Use [`ConceptName::from_stored`] to load
    /// names read back from the database without re-normalizing.
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_safe_name(&s, "Concept")?;
        Ok(Self(s.to_lowercase()))
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
// Well-known attribute selectors
// ---------------------------------------------------------------------------

// Concept meta-schema attributes are defined as typed newtypes in the
// `concept` module above. These selector functions and string constants
// provide access for low-level Instruction building and fetch_string
// queries respectively.

/// Get the artifact-level selector for `concept/name`.
pub fn concept_name_selector() -> ClaimAttribute {
    <concept::Name as dialog_query::Attribute>::selector()
}

/// Get the artifact-level selector for `concept/description`.
pub fn concept_description_selector() -> ClaimAttribute {
    <concept::Description as dialog_query::Attribute>::selector()
}

/// Get the artifact-level selector for `concept/attribute`.
pub fn concept_attribute_selector() -> ClaimAttribute {
    <concept::Attribute as dialog_query::Attribute>::selector()
}

/// Get the artifact-level selector for `concept/namespace`.
pub fn concept_namespace_selector() -> ClaimAttribute {
    <concept::Namespace as dialog_query::Attribute>::selector()
}

/// Get the artifact-level selector for `concept/prior`.
pub fn concept_prior_selector() -> ClaimAttribute {
    <concept::Prior as dialog_query::Attribute>::selector()
}

/// Get the artifact-level selector for `concept/update-rationale`.
pub fn concept_update_rationale_selector() -> ClaimAttribute {
    <concept::UpdateRationale as dialog_query::Attribute>::selector()
}

/// Attribute metadata: human-readable description of the attribute.
pub const ATTR_ATTRIBUTE_DESCRIPTION: &str = "attribute/description";

/// Attribute metadata: type constraint (e.g. "Text", "Integer", "RecipeStep", or JSON array for enums).
pub const ATTR_ATTRIBUTE_TYPE: &str = "attribute/type";

/// Attribute metadata: cardinality ("many" for multi-valued, absent for single).
pub const ATTR_ATTRIBUTE_CARDINALITY: &str = "attribute/cardinality";

/// Attribute metadata: whether the attribute is optional.
pub const ATTR_ATTRIBUTE_OPTIONAL: &str = "attribute/optional";

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

/// Derive a concept entity from its attribute set using dialog-db's
/// structural identity.
///
/// Builds a dynamic `Concept` from the attribute list, computes its
/// `operator()` URI (a blake3 hash of the CBOR-encoded attribute set),
/// then derives a deterministic `did:key` entity from that URI.
///
/// Two concepts with the same attributes (same namespace/name/cardinality/type
/// per attribute) produce the same entity ID, regardless of concept name.
pub fn concept_entity_from_attrs(
    attributes: &[String],
    cardinalities: &std::collections::HashMap<String, Cardinality>,
) -> Result<Entity> {
    let concept = build_dynamic_concept(attributes, cardinalities)?;
    let operator_uri = concept.operator();
    derive_entity(&operator_uri)
}

/// Look up a concept entity by name.
///
/// Queries the AEV index using the typed `concept::Name` selector,
/// with case-insensitive matching.
pub async fn lookup_concept_by_name<S: ArtifactStore>(
    store: &S,
    name: &ConceptName,
) -> Result<Option<Entity>> {
    let concept_entities = find_entities_by_attribute(store, concept_name_selector()).await?;

    for entity in concept_entities {
        let stored_names = fetch_string_values(store, &entity, concept_name_selector()).await?;
        if stored_names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Ok(Some(entity));
        }
    }
    Ok(None)
}

/// Find all named concept entities in the store.
///
/// Discovers concepts by querying the AEV index for entities with the
/// typed `concept::Name` attribute. Returns `(entity, name)` pairs.
pub async fn find_all_concepts<S: ArtifactStore>(store: &S) -> Result<Vec<(Entity, String)>> {
    let entities = find_entities_by_attribute(store, concept_name_selector()).await?;
    let mut result = Vec::new();
    for entity in entities {
        let names = fetch_string_values(store, &entity, concept_name_selector()).await?;
        for name in names {
            result.push((entity.clone(), name));
        }
    }
    Ok(result)
}

/// Find all rule entities in the store (both named and unnamed).
///
/// Discovers rules by querying the AEV index for entities with a
/// `rule/conclusion` attribute (which all rules have). Returns
/// `(entity, Option<name>)` pairs.
pub async fn find_all_rules<S: ArtifactStore>(store: &S) -> Result<Vec<(Entity, Option<String>)>> {
    let conclusion_attr = parse_claim_attribute(ATTR_RULE_CONCLUSION)?;
    let rule_name_attr = parse_claim_attribute(ATTR_RULE_NAME)?;
    let entities = find_entities_by_attribute(store, conclusion_attr).await?;
    let mut result = Vec::new();
    for entity in entities {
        let name = fetch_string(store, &entity, rule_name_attr.clone()).await?;
        result.push((entity, name));
    }
    Ok(result)
}

/// Look up a rule entity by name.
///
/// Discovers rules structurally by querying the AEV index for all
/// entities with a `rule/name` attribute, then matches by name
/// (case-insensitive).
pub async fn lookup_rule_by_name<S: ArtifactStore>(
    store: &S,
    name: &str,
) -> Result<Option<Entity>> {
    let rule_name_attr = parse_claim_attribute(ATTR_RULE_NAME)?;
    let rule_entities = find_entities_by_attribute(store, rule_name_attr.clone()).await?;
    for entity in rule_entities {
        if let Some(stored_name) = fetch_string(store, &entity, rule_name_attr.clone()).await?
            && stored_name.to_lowercase() == name.to_lowercase()
        {
            return Ok(Some(entity));
        }
    }
    Ok(None)
}

/// Derive the rule entity for a given rule name within a space.
///
/// Deterministically derived as an Ed25519 `did:key` from the space DID and rule name.
pub fn rule_entity(space_did: &str, rule_name: &str) -> Result<Entity> {
    derive_entity(&format!(
        "{}\0rule\0{}",
        space_did,
        rule_name.to_lowercase()
    ))
}

/// Derive the rule entity from its definition content (for unnamed rules).
///
/// Deterministically derived as an Ed25519 `did:key` from the space DID and
/// the blake3 hash of the canonical definition JSON. Same definition in the
/// same space always produces the same entity, making unnamed defines idempotent.
pub fn rule_entity_from_definition(space_did: &str, definition_json: &str) -> Result<Entity> {
    let def_hash = blake3::hash(definition_json.as_bytes());
    derive_entity(&format!("{}\0rule\0def:{}", space_did, def_hash.to_hex()))
}

/// Derive an attribute metadata entity from its structural identity.
///
/// Uses dialog-db's `AttributeSchema::to_uri()` to derive a deterministic
/// entity ID from the attribute's namespace/name. This is concept-independent —
/// the same attribute used in multiple concepts shares one metadata entity.
pub fn attribute_meta_entity(attr_name: &str) -> Result<Entity> {
    let (ns, name) = attr_name.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "Malformed attribute '{}': expected 'namespace/name' format",
            attr_name
        )
    })?;
    let schema = dialog_query::AttributeSchema::<Value>::new(
        leak_str(ns),
        leak_str(name),
        leak_str(""),
        dialog_query::Type::String,
    );
    let uri = schema.to_uri();
    derive_entity(&uri)
}

/// Convert 32 bytes of hash output into a proper Ed25519 `did:key` entity.
///
/// Treats the hash as an Ed25519 signing key seed, then formats the
/// resulting verifying (public) key as a standards-compliant `did:key`
/// with the Ed25519 multicodec prefix `[0xed, 0x01]`.
///
/// This is the canonical implementation — `derive_entity()` and
/// `derive_entity_from_fields()` both delegate here, as does `fact.rs`.
pub fn derive_entity_from_hash(hash: &blake3::Hash) -> Result<Entity> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(hash.as_bytes());
    let verifying_key = signing_key.verifying_key();
    const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];
    let mut multicodec_key = [0u8; 34];
    multicodec_key[..2].copy_from_slice(&ED25519_MULTICODEC);
    multicodec_key[2..].copy_from_slice(verifying_key.as_bytes());
    let encoded = bs58::encode(&multicodec_key).into_string();
    let uri = format!("did:key:z{}", encoded);
    Entity::from_str(&uri).context("Failed to derive did:key entity")
}

/// Low-level: hash input to produce a deterministic `did:key` entity.
///
/// Uses blake3 to hash the input, then delegates to [`derive_entity_from_hash`].
pub fn derive_entity(input: &str) -> Result<Entity> {
    derive_entity_from_hash(&blake3::hash(input.as_bytes()))
}

/// Derive an entity ID deterministically from field content.
///
/// Sorts fields by attribute name, hashes the concatenated key/value pairs
/// with blake3, then delegates to [`derive_entity_from_hash`] for proper Ed25519
/// `did:key` formatting.
pub fn derive_entity_from_fields(fields: &[(String, String)]) -> Result<Entity> {
    let mut sorted = fields.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (attr, value) in &sorted {
        hasher.update(attr.as_bytes());
        hasher.update(b"\0");
        hasher.update(value.as_bytes());
        hasher.update(b"\0");
    }

    derive_entity_from_hash(&hasher.finalize())
}

// ---------------------------------------------------------------------------
// Attribute name helpers
// ---------------------------------------------------------------------------

/// Given a namespace and user-supplied attribute key, produce the fully
/// qualified attribute name `{namespace}/{key}`.
///
/// If the key already contains a `/`, it is returned as-is (the user
/// provided a fully qualified attribute name).
pub fn qualify_attribute(namespace: &str, key: &str) -> Result<String> {
    if key.contains('/') {
        Ok(key.to_string())
    } else {
        Ok(format!("{}/{}", namespace, key))
    }
}

/// Strip the namespace prefix from an attribute name, returning just the
/// short key (e.g. `"my-space/title"` -> `"title"`).
///
/// If the attribute doesn't match the provided namespace, the full
/// attribute name is returned as-is.
pub fn short_attribute(namespace: &str, attr: &str) -> String {
    let prefix = format!("{}/", namespace);
    if let Some(short) = attr.strip_prefix(&prefix) {
        short.to_string()
    } else {
        // Fall back to stripping any namespace prefix
        if let Some((_ns, name)) = attr.split_once('/') {
            name.to_string()
        } else {
            attr.to_string()
        }
    }
}

/// Extract the namespace from a fully-qualified attribute name.
pub fn attribute_namespace(attr: &str) -> &str {
    attr.split_once('/').map(|(ns, _)| ns).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Common query helpers
// ---------------------------------------------------------------------------

/// Fetch all string values for a multi-valued attribute on an entity.
pub async fn fetch_string_values<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr: ClaimAttribute,
) -> Result<Vec<String>> {
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
    attr: ClaimAttribute,
) -> Result<Option<String>> {
    let values = fetch_string_values(store, entity, attr).await?;
    Ok(values.into_iter().next())
}

/// Fetch all entity values for a multi-valued attribute.
pub async fn fetch_entity_values<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
    attr: ClaimAttribute,
) -> Result<Vec<Entity>> {
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
    attr: ClaimAttribute,
) -> Result<Option<Value>> {
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
    attr: ClaimAttribute,
) -> Result<Vec<Value>> {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()).the(attr))
        .try_collect()
        .await?;

    Ok(results.into_iter().map(|a| a.is).collect())
}

// ---------------------------------------------------------------------------
// Entity discovery helpers
// ---------------------------------------------------------------------------

/// Find all entities that have a given attribute (using the AEV index).
///
/// This replaces the old `concept/entity` back-reference pattern.
/// Dialog-db identifies concept membership structurally: an entity belongs
/// to a concept if it has facts for that concept's attributes.
pub async fn find_entities_by_attribute<S: ArtifactStore>(
    store: &S,
    attr: ClaimAttribute,
) -> Result<Vec<Entity>> {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().the(attr))
        .try_collect()
        .await?;

    // Deduplicate entities (multiple values for the same entity+attribute
    // would otherwise produce duplicates).
    let mut seen = std::collections::HashSet::new();
    let mut entities = Vec::new();
    for artifact in results {
        if seen.insert(artifact.of.to_string()) {
            entities.push(artifact.of);
        }
    }
    Ok(entities)
}

/// Find all entities belonging to a concept by checking ALL schema attributes.
///
/// Queries the AEV index for each schema attribute and intersects the
/// entity sets. An entity must have facts for every attribute to be
/// included — matching dialog-db's structural inner-join semantics.
pub async fn find_entities_by_concept<S: ArtifactStore>(
    store: &S,
    schema_attrs: &[String],
) -> Result<Vec<Entity>> {
    if schema_attrs.is_empty() {
        return Ok(Vec::new());
    }

    // Start with entities from the first attribute
    let first_attr = parse_claim_attribute(&schema_attrs[0])?;
    let mut result_set: std::collections::HashSet<String> =
        find_entities_by_attribute(store, first_attr)
            .await?
            .iter()
            .map(|e| e.to_string())
            .collect();

    // Intersect with entities from each subsequent attribute
    for attr in &schema_attrs[1..] {
        let claim_attr = parse_claim_attribute(attr)?;
        let attr_entities: std::collections::HashSet<String> =
            find_entities_by_attribute(store, claim_attr)
                .await?
                .iter()
                .map(|e| e.to_string())
                .collect();
        result_set = result_set.intersection(&attr_entities).cloned().collect();
        if result_set.is_empty() {
            return Ok(Vec::new());
        }
    }

    // Convert back to Entity values (preserving original Entity objects)
    let first_attr = parse_claim_attribute(&schema_attrs[0])?;
    let all_entities = find_entities_by_attribute(store, first_attr).await?;
    Ok(all_entities
        .into_iter()
        .filter(|e| result_set.contains(&e.to_string()))
        .collect())
}

/// Parse a string attribute name into a `ClaimAttribute`.
pub fn parse_claim_attribute(attr_name: &str) -> Result<ClaimAttribute> {
    ClaimAttribute::from_str(attr_name).context(format!("Invalid attribute: {}", attr_name))
}

/// Infer the concept that an entity belongs to by examining its attributes.
///
/// Fetches all facts about the entity, then checks each registered concept
/// to see if the entity has facts for ALL of that concept's attributes.
/// Returns the best-matching concept (the one with the most attributes).
///
/// Returns `(concept_name, concept_entity, schema_attrs)` or an error if
/// the entity has no attributes or the concept cannot be resolved.
pub async fn infer_concept_from_entity<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
) -> Result<(ConceptName, Entity, Vec<String>)> {
    // Fetch all facts about this entity
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()))
        .try_collect()
        .await?;

    if results.is_empty() {
        anyhow::bail!("Entity '{}' not found (no facts)", entity);
    }

    // Collect the entity's attribute names
    let entity_attrs: std::collections::HashSet<String> =
        results.iter().map(|a| a.the.to_string()).collect();

    // Find all named concepts via structural discovery (AEV index)
    let concept_entities = find_entities_by_attribute(store, concept_name_selector()).await?;

    let mut best_match: Option<(ConceptName, Entity, Vec<String>, usize)> = None;

    for concept_ent in &concept_entities {
        let name = match fetch_string(store, concept_ent, concept_name_selector()).await? {
            Some(n) => ConceptName::from_stored(n),
            None => continue,
        };
        let schema_attrs =
            fetch_string_values(store, concept_ent, concept_attribute_selector()).await?;

        if schema_attrs.is_empty() {
            continue;
        }

        // Check if the entity has ALL of this concept's attributes
        let has_all = schema_attrs.iter().all(|a| entity_attrs.contains(a));
        if has_all {
            let score = schema_attrs.len();
            if best_match.as_ref().is_none_or(|(_, _, _, s)| score > *s) {
                best_match = Some((name, concept_ent.clone(), schema_attrs, score));
            }
        }
    }

    match best_match {
        Some((name, ent, attrs, _)) => Ok((name, ent, attrs)),
        None => anyhow::bail!(
            "Could not resolve concept for entity '{}' (no named concept matches its attributes)",
            entity
        ),
    }
}

/// Fetch all facts (as Artifacts) for an entity.
///
/// Used by retract operations to discover and remove all facts about an
/// entity without needing to know its concept schema in advance.
pub async fn fetch_all_entity_facts<S: ArtifactStore>(
    store: &S,
    entity: &Entity,
) -> Result<Vec<dialog_artifacts::Artifact>> {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(entity.clone()))
        .try_collect()
        .await?;
    Ok(results)
}

// ---------------------------------------------------------------------------
// Cardinality helpers
// ---------------------------------------------------------------------------

/// Fetch the cardinality for each attribute from stored metadata.
///
/// Returns a map from fully-qualified attribute name to `Cardinality`.
/// Attributes without stored cardinality metadata default to `Cardinality::One`.
pub async fn fetch_attribute_cardinalities<S: ArtifactStore>(
    store: &S,
    schema_attrs: &[String],
) -> Result<std::collections::HashMap<String, Cardinality>> {
    let cardinality_attr = parse_claim_attribute(ATTR_ATTRIBUTE_CARDINALITY)?;
    let mut cardinalities = std::collections::HashMap::new();
    for attr_name in schema_attrs {
        let meta_entity = attribute_meta_entity(attr_name)?;
        if let Some(val) = fetch_string(store, &meta_entity, cardinality_attr.clone()).await?
            && val.to_lowercase() == "many"
        {
            cardinalities.insert(attr_name.clone(), Cardinality::Many);
        }
    }
    Ok(cardinalities)
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
/// wrapped in an `AttributeSchema<Value>`.
///
/// ## Type::String default
///
/// `Type::String` is used for all attributes because dialog-db's type
/// checking at query time is a no-op (`AttributeSchema::check()` always
/// returns Ok). Type only affects the concept's identity hash (used for
/// rule resolution), but since both `build_dynamic_concept()` and
/// `compile_rule()` use the same `Type::String` default, concept URIs
/// are consistent and rules resolve correctly.
///
/// ## Cardinality
///
/// If a `cardinalities` map is provided, attributes with
/// `Cardinality::Many` get correct cost estimates from the query planner.
/// Attributes not in the map default to `Cardinality::One`.
pub fn build_dynamic_concept(
    attributes: &[String],
    cardinalities: &std::collections::HashMap<String, Cardinality>,
) -> Result<dialog_query::predicate::Concept> {
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
            let mut schema = AttributeSchema::<Value>::new(
                leak_str(ns),
                short_name,
                leak_str(""), // description
                Type::String, // see doc comment above
            );
            if let Some(&cardinality) = cardinalities.get(attr) {
                schema.cardinality = cardinality;
            }
            Ok((short_name, schema))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(dialog_query::predicate::Concept::new(attr_schemas.into()))
}
