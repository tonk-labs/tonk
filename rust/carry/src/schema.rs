//! Shared helpers for the concepts & facts system.
//!
//! Provides typed meta-schema definitions using dialog-query derive macros,
//! deterministic entity derivation, and common storage access patterns.
//!
//! ## Meta-schema domains (RFC)
//!
//! | Domain                  | Purpose                                          |
//! |-------------------------|--------------------------------------------------|
//! | `dialog.attribute`      | Attribute identity fields (id, type, cardinality) |
//! | `dialog.concept.with`   | Required concept membership by field name         |
//! | `dialog.concept.maybe`  | Optional concept membership by field name          |
//! | `dialog.meta`           | Universal metadata: names and descriptions        |

use anyhow::{Context, Result};
use base64::Engine as _;
use dialog_artifacts::{ArtifactSelector, ArtifactStore};
use dialog_query::Attribute as _;
pub use dialog_query::claim::Attribute as ClaimAttribute;
use dialog_query::concept::Concept as _;
use dialog_query::{Entity, Value};
use futures_util::TryStreamExt;
use std::collections::BTreeMap;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Typed primitive attributes (RFC §Appendix: Primitive domains)
// ---------------------------------------------------------------------------

/// Universal metadata attributes: names and descriptions for any entity.
pub mod dialog_meta {
    /// Human-readable name for any entity.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[namespace("dialog.meta")]
    pub struct Name(pub String);

    /// Human-readable description for any entity.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[namespace("dialog.meta")]
    pub struct Description(pub String);
}

/// Attribute identity fields.
pub mod dialog_attribute {
    /// Nominal identifier (selector) of the attribute, e.g. `io.gozala.person/name`.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[namespace("dialog.attribute")]
    pub struct Id(pub String);

    /// Value type of the attribute (e.g. Text, UnsignedInteger, Symbol).
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[namespace("dialog.attribute")]
    pub struct Type(pub String);

    /// Cardinality: `one` or `many`.
    #[derive(dialog_query::Attribute, Clone, PartialEq)]
    #[namespace("dialog.attribute")]
    pub struct Cardinality(pub String);
}

// ---------------------------------------------------------------------------
// Typed builtin concept structs
// ---------------------------------------------------------------------------

/// The `attribute` concept: models a typed relation.
///
/// CLI field mapping:
///   `the`         → `dialog.attribute/id`
///   `as`          → `dialog.attribute/type`
///   `cardinality` → `dialog.attribute/cardinality`
///   `description` → `dialog.meta/description`
#[derive(dialog_query::Concept, Debug, Clone)]
pub struct AttributeDef {
    pub this: Entity,
    pub description: dialog_meta::Description,
    pub the: dialog_attribute::Id,
    // TODO: use `#[dialog(rename = "as")]` when available
    pub as_type: dialog_attribute::Type,
    pub cardinality: dialog_attribute::Cardinality,
}

/// The `bookmark` concept: maps a name to any entity.
///
/// CLI field mapping:
///   `name` → `dialog.meta/name`
#[derive(dialog_query::Concept, Debug, Clone)]
pub struct BookmarkDef {
    pub this: Entity,
    pub name: dialog_meta::Name,
}

// Note: The `concept` concept cannot be fully expressed with #[derive(Concept)]
// because its `with` and `maybe` fields are variable-keyed (with.{?name}).
// The fixed `description` field is captured here; variable-keyed fields are
// handled dynamically.

/// The `concept` concept (partial): captures the fixed `description` field.
///
/// Variable-keyed fields (`with.{name}`, `maybe.{name}`) are handled
/// dynamically in assertion/query code.
#[derive(dialog_query::Concept, Debug, Clone)]
pub struct ConceptDef {
    pub this: Entity,
    pub description: dialog_meta::Description,
}

// ---------------------------------------------------------------------------
// Builtin concept field mapping (CLI field name → attribute selector)
// ---------------------------------------------------------------------------

/// A field within a pre-registered concept's schema.
#[derive(Debug, Clone)]
pub struct BuiltinField {
    /// The field name as exposed to the CLI (e.g. "the", "as", "name").
    pub cli_name: &'static str,
    /// The fully-qualified relation identifier this field maps to.
    pub relation: &'static str,
    /// The expected value type (for documentation/validation).
    pub value_type: &'static str,
    /// Default cardinality.
    pub cardinality: &'static str,
    /// Whether this is a variable-keyed field (e.g. `with` on the concept concept).
    pub variable_keyed: bool,
}

/// Schema for a pre-registered concept.
#[derive(Debug, Clone)]
pub struct BuiltinConceptSchema {
    /// The concept name (also its bookmark).
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Required fields (`with`).
    pub with_fields: &'static [BuiltinField],
    /// Optional fields (`maybe`).
    pub maybe_fields: &'static [BuiltinField],
}

/// The `attribute` concept schema.
pub static BUILTIN_ATTRIBUTE: BuiltinConceptSchema = BuiltinConceptSchema {
    name: "attribute",
    description: "Built-in concept for modeling attributes",
    with_fields: &[
        BuiltinField {
            cli_name: "description",
            relation: "dialog.meta/description",
            value_type: "Text",
            cardinality: "one",
            variable_keyed: false,
        },
        BuiltinField {
            cli_name: "the",
            relation: "dialog.attribute/id",
            value_type: "Symbol",
            cardinality: "one",
            variable_keyed: false,
        },
        BuiltinField {
            cli_name: "as",
            relation: "dialog.attribute/type",
            value_type: "Symbol",
            cardinality: "one",
            variable_keyed: false,
        },
        BuiltinField {
            cli_name: "cardinality",
            relation: "dialog.attribute/cardinality",
            value_type: "Symbol",
            cardinality: "one",
            variable_keyed: false,
        },
    ],
    maybe_fields: &[],
};

/// The `concept` concept schema.
pub static BUILTIN_CONCEPT: BuiltinConceptSchema = BuiltinConceptSchema {
    name: "concept",
    description: "Built-in concept for composing attributes into a shape",
    with_fields: &[
        BuiltinField {
            cli_name: "description",
            relation: "dialog.meta/description",
            value_type: "Text",
            cardinality: "one",
            variable_keyed: false,
        },
        BuiltinField {
            cli_name: "with",
            relation: "dialog.concept.with/",
            value_type: "attribute",
            cardinality: "one",
            variable_keyed: true,
        },
    ],
    maybe_fields: &[BuiltinField {
        cli_name: "maybe",
        relation: "dialog.concept.maybe/",
        value_type: "attribute",
        cardinality: "one",
        variable_keyed: true,
    }],
};

/// The `bookmark` concept schema.
pub static BUILTIN_BOOKMARK: BuiltinConceptSchema = BuiltinConceptSchema {
    name: "bookmark",
    description: "Naming mechanism mapping a local name to any entity",
    with_fields: &[BuiltinField {
        cli_name: "name",
        relation: "dialog.meta/name",
        value_type: "Text",
        cardinality: "one",
        variable_keyed: false,
    }],
    maybe_fields: &[],
};

/// Look up a pre-registered (builtin) concept schema by name.
pub fn lookup_builtin(name: &str) -> Option<&'static BuiltinConceptSchema> {
    match name.to_lowercase().as_str() {
        "attribute" => Some(&BUILTIN_ATTRIBUTE),
        "concept" => Some(&BUILTIN_CONCEPT),
        "bookmark" => Some(&BUILTIN_BOOKMARK),
        _ => None,
    }
}

/// Find the relation for a CLI field name within a builtin concept schema.
///
/// For variable-keyed fields, `cli_field` should be the dotted form (e.g. "with.name").
/// Returns `(relation, is_variable_keyed)`.
pub fn resolve_builtin_field(
    schema: &BuiltinConceptSchema,
    cli_field: &str,
) -> Option<(String, bool)> {
    // Check fixed fields first
    for f in schema.with_fields.iter().chain(schema.maybe_fields.iter()) {
        if !f.variable_keyed && f.cli_name == cli_field {
            return Some((f.relation.to_string(), false));
        }
    }
    // Check variable-keyed fields (e.g. "with.name" → prefix "with")
    for f in schema.with_fields.iter().chain(schema.maybe_fields.iter()) {
        if f.variable_keyed
            && let Some(key) = cli_field
                .strip_prefix(f.cli_name)
                .and_then(|s| s.strip_prefix('.'))
            && !key.is_empty()
        {
            return Some((format!("{}{}", f.relation, key), true));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Name validation
// ---------------------------------------------------------------------------

/// Validate that a name contains only safe characters: letters, digits,
/// hyphens, and underscores.
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConceptName(String);

impl ConceptName {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_safe_name(&s, "Concept")?;
        Ok(Self(s.to_lowercase()))
    }

    pub fn from_stored(s: String) -> Self {
        Self(s)
    }

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
// Entity derivation
// ---------------------------------------------------------------------------

/// Derive an attribute entity from its identity fields.
///
/// Per the RFC: `entity = hash(the, type, cardinality)`.
/// Description and name do NOT participate in identity.
pub fn derive_attribute_entity(
    selector: &str,
    value_type: &str,
    cardinality: &str,
) -> Result<Entity> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"attribute\0");
    hasher.update(selector.as_bytes());
    hasher.update(b"\0");
    hasher.update(value_type.as_bytes());
    hasher.update(b"\0");
    hasher.update(cardinality.as_bytes());
    derive_entity_from_hash(&hasher.finalize())
}

/// Derive a concept entity from its field→attribute mappings.
///
/// Per the RFC: identity = hash(sorted (field_name, attribute_entity) pairs).
/// Both field names and attribute entities participate. `maybe` fields do NOT.
pub fn derive_concept_entity(with_fields: &BTreeMap<String, Entity>) -> Result<Entity> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"concept\0");
    for (field_name, attr_entity) in with_fields {
        hasher.update(field_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(attr_entity.to_string().as_bytes());
        hasher.update(b"\0");
    }
    derive_entity_from_hash(&hasher.finalize())
}

/// Convert 32 bytes of hash output into a proper Ed25519 `did:key` entity.
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

/// Hash input to produce a deterministic `did:key` entity.
pub fn derive_entity(input: &str) -> Result<Entity> {
    derive_entity_from_hash(&blake3::hash(input.as_bytes()))
}

/// Derive an entity ID deterministically from field content.
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
// Concept resolution (runtime lookup via dialog.meta/name)
// ---------------------------------------------------------------------------

/// Look up any entity by its `dialog.meta/name` value.
pub async fn lookup_entity_by_name<S: ArtifactStore>(
    store: &S,
    name: &str,
) -> Result<Option<Entity>> {
    let name_attr = dialog_meta::Name::selector();
    let entities = find_entities_by_attribute(store, name_attr.clone()).await?;

    for entity in entities {
        let stored_names = fetch_string_values(store, &entity, name_attr.clone()).await?;
        if stored_names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Ok(Some(entity));
        }
    }
    Ok(None)
}

/// A concept resolved from the database at runtime.
#[derive(Debug, Clone)]
pub struct ResolvedConcept {
    /// The concept entity.
    pub entity: Entity,
    /// The concept's name.
    pub name: String,
    /// Required fields: field_name → (attribute_entity, attribute_selector).
    pub with_fields: BTreeMap<String, (Entity, String)>,
    /// Optional fields: same structure.
    pub maybe_fields: BTreeMap<String, (Entity, String)>,
}

/// Resolve a concept by name: look up the entity, fetch its
/// `dialog.concept.with/*` and `dialog.concept.maybe/*` claims,
/// and for each attribute entity fetch its `dialog.attribute/id`.
pub async fn resolve_concept<S: ArtifactStore>(
    store: &S,
    concept_name: &str,
) -> Result<ResolvedConcept> {
    let concept_entity = lookup_entity_by_name(store, concept_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Concept '{}' not found", concept_name))?;

    let with_fields = fetch_concept_fields(store, &concept_entity, "dialog.concept.with/").await?;
    let maybe_fields =
        fetch_concept_fields(store, &concept_entity, "dialog.concept.maybe/").await?;

    if with_fields.is_empty() {
        anyhow::bail!("Concept '{}' has no required fields", concept_name);
    }

    Ok(ResolvedConcept {
        entity: concept_entity,
        name: concept_name.to_string(),
        with_fields,
        maybe_fields,
    })
}

/// Fetch concept fields from `dialog.concept.with/*` or `dialog.concept.maybe/*` claims.
///
/// Returns a map of field_name → (attribute_entity, attribute_selector).
async fn fetch_concept_fields<S: ArtifactStore>(
    store: &S,
    concept_entity: &Entity,
    prefix: &str,
) -> Result<BTreeMap<String, (Entity, String)>> {
    let mut fields = BTreeMap::new();

    let results: Vec<_> = store
        .select(ArtifactSelector::new().of(concept_entity.clone()))
        .try_collect()
        .await?;

    let attr_id_selector = dialog_attribute::Id::selector();

    for artifact in &results {
        let attr_str = artifact.the.to_string();
        if let Some(field_name) = attr_str.strip_prefix(prefix) {
            if field_name.is_empty() {
                continue;
            }
            let attr_entity = match &artifact.is {
                Value::Entity(e) => e.clone(),
                _ => continue,
            };

            let selector = fetch_string(store, &attr_entity, attr_id_selector.clone())
                .await?
                .unwrap_or_default();

            fields.insert(field_name.to_string(), (attr_entity, selector));
        }
    }

    Ok(fields)
}

/// Get all attribute selectors for a resolved concept's required fields.
pub fn concept_attribute_selectors(concept: &ResolvedConcept) -> Vec<String> {
    concept
        .with_fields
        .values()
        .map(|(_, selector)| selector.clone())
        .collect()
}

/// Resolve a concept field name to its attribute selector.
pub fn resolve_field_selector(concept: &ResolvedConcept, field_name: &str) -> Result<String> {
    if let Some((_, selector)) = concept.with_fields.get(field_name) {
        return Ok(selector.clone());
    }
    if let Some((_, selector)) = concept.maybe_fields.get(field_name) {
        return Ok(selector.clone());
    }
    anyhow::bail!(
        "Field '{}' not found in concept '{}'. Available fields: {}",
        field_name,
        concept.name,
        concept
            .with_fields
            .keys()
            .chain(concept.maybe_fields.keys())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ---------------------------------------------------------------------------
// Concept membership (structural matching)
// ---------------------------------------------------------------------------

/// Find all entities belonging to a concept by checking ALL required attribute selectors.
pub async fn find_entities_by_concept<S: ArtifactStore>(
    store: &S,
    schema_attrs: &[String],
) -> Result<Vec<Entity>> {
    if schema_attrs.is_empty() {
        return Ok(Vec::new());
    }

    let first_attr = parse_claim_attribute(&schema_attrs[0])?;
    let mut result_set: std::collections::HashSet<String> =
        find_entities_by_attribute(store, first_attr)
            .await?
            .iter()
            .map(|e| e.to_string())
            .collect();

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

    let first_attr = parse_claim_attribute(&schema_attrs[0])?;
    let all_entities = find_entities_by_attribute(store, first_attr).await?;
    Ok(all_entities
        .into_iter()
        .filter(|e| result_set.contains(&e.to_string()))
        .collect())
}

// ---------------------------------------------------------------------------
// Init bootstrapping
// ---------------------------------------------------------------------------

/// Assert all pre-registered concepts into the space.
///
/// Called during `carry init` to bootstrap the meta-schema.
/// Claims are deterministic (content-addressed), so re-running is idempotent.
pub async fn bootstrap_builtins<S: dialog_query::Store>(
    session: &mut dialog_query::Session<S>,
) -> Result<()> {
    use dialog_query::claim::{Claim, Relation};

    let builtins = [&BUILTIN_ATTRIBUTE, &BUILTIN_CONCEPT, &BUILTIN_BOOKMARK];

    // First pass: derive attribute entities for all fixed fields.
    let mut attr_entities: BTreeMap<String, Entity> = BTreeMap::new();

    for builtin in &builtins {
        for field in builtin
            .with_fields
            .iter()
            .chain(builtin.maybe_fields.iter())
        {
            if field.variable_keyed {
                continue;
            }
            let entity =
                derive_attribute_entity(field.relation, field.value_type, field.cardinality)?;
            attr_entities.insert(field.relation.to_string(), entity);
        }
    }

    let mut transaction = session.edit();

    let name_attr = dialog_meta::Name::selector();
    let desc_attr = dialog_meta::Description::selector();
    let attr_id = dialog_attribute::Id::selector();
    let attr_type = dialog_attribute::Type::selector();
    let attr_card = dialog_attribute::Cardinality::selector();

    // Assert attribute entity claims
    for builtin in &builtins {
        for field in builtin
            .with_fields
            .iter()
            .chain(builtin.maybe_fields.iter())
        {
            if field.variable_keyed {
                continue;
            }

            let entity = attr_entities[field.relation].clone();

            // dialog.attribute/id
            Relation::new(
                attr_id.clone(),
                entity.clone(),
                Value::String(field.relation.to_string()),
            )
            .assert(&mut transaction);

            // dialog.attribute/type
            Relation::new(
                attr_type.clone(),
                entity.clone(),
                Value::String(field.value_type.to_string()),
            )
            .assert(&mut transaction);

            // dialog.attribute/cardinality
            Relation::new(
                attr_card.clone(),
                entity.clone(),
                Value::String(field.cardinality.to_string()),
            )
            .assert(&mut transaction);

            // dialog.meta/name = qualified name (e.g. "attribute/the")
            Relation::new(
                name_attr.clone(),
                entity.clone(),
                Value::String(format!("{}/{}", builtin.name, field.cli_name)),
            )
            .assert(&mut transaction);
        }
    }

    // Assert concept entity claims
    for builtin in &builtins {
        let mut with_fields: BTreeMap<String, Entity> = BTreeMap::new();
        for field in builtin.with_fields {
            if field.variable_keyed {
                continue;
            }
            with_fields.insert(
                field.cli_name.to_string(),
                attr_entities[field.relation].clone(),
            );
        }

        let concept_entity = derive_concept_entity(&with_fields)?;

        // dialog.meta/name
        Relation::new(
            name_attr.clone(),
            concept_entity.clone(),
            Value::String(builtin.name.to_string()),
        )
        .assert(&mut transaction);

        // dialog.meta/description
        Relation::new(
            desc_attr.clone(),
            concept_entity.clone(),
            Value::String(builtin.description.to_string()),
        )
        .assert(&mut transaction);

        // dialog.concept.with/{field} = attribute_entity
        for field in builtin.with_fields {
            if field.variable_keyed {
                continue;
            }
            let rel = format!("dialog.concept.with/{}", field.cli_name);
            let rel_attr = parse_claim_attribute(&rel)?;
            Relation::new(
                rel_attr,
                concept_entity.clone(),
                Value::Entity(attr_entities[field.relation].clone()),
            )
            .assert(&mut transaction);
        }

        // dialog.concept.maybe/{field} = attribute_entity
        for field in builtin.maybe_fields {
            if field.variable_keyed {
                continue;
            }
            let rel = format!("dialog.concept.maybe/{}", field.cli_name);
            let rel_attr = parse_claim_attribute(&rel)?;
            Relation::new(
                rel_attr,
                concept_entity.clone(),
                Value::Entity(attr_entities[field.relation].clone()),
            )
            .assert(&mut transaction);
        }
    }

    session.commit(transaction).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Attribute name helpers
// ---------------------------------------------------------------------------

/// Qualify a field name within a namespace: `{namespace}/{key}`.
/// If the key already contains `/`, it is returned as-is.
pub fn qualify_attribute(namespace: &str, key: &str) -> Result<String> {
    if key.contains('/') {
        Ok(key.to_string())
    } else {
        Ok(format!("{}/{}", namespace, key))
    }
}

/// Strip the namespace prefix from an attribute name.
pub fn short_attribute(namespace: &str, attr: &str) -> String {
    let prefix = format!("{}/", namespace);
    if let Some(short) = attr.strip_prefix(&prefix) {
        short.to_string()
    } else if let Some((_ns, name)) = attr.split_once('/') {
        name.to_string()
    } else {
        attr.to_string()
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
pub async fn find_entities_by_attribute<S: ArtifactStore>(
    store: &S,
    attr: ClaimAttribute,
) -> Result<Vec<Entity>> {
    let results: Vec<_> = store
        .select(ArtifactSelector::new().the(attr))
        .try_collect()
        .await?;

    let mut seen = std::collections::HashSet::new();
    let mut entities = Vec::new();
    for artifact in results {
        if seen.insert(artifact.of.to_string()) {
            entities.push(artifact.of);
        }
    }
    Ok(entities)
}

/// Parse a string attribute name into a `ClaimAttribute`.
pub fn parse_claim_attribute(attr_name: &str) -> Result<ClaimAttribute> {
    ClaimAttribute::from_str(attr_name).context(format!("Invalid attribute: {}", attr_name))
}

/// Fetch all facts (as Artifacts) for an entity.
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
// Value helpers
// ---------------------------------------------------------------------------

/// Parse a string input into a Value, trying integer -> float -> string.
pub fn parse_value(input: &str) -> Value {
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

/// Leak a runtime string to get a `&'static str`.
pub fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
