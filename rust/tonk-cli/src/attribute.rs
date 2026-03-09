//! Attribute introspection: list and inspect attribute metadata.
//!
//! Attributes are the fields of a concept schema. Each attribute has a qualified
//! name (e.g. `recipe/title`) and optional metadata — description, type,
//! cardinality, and optional flag — written by `tonk import`.
//!
//! This module reads that metadata back for schema discoverability.

use crate::schema::*;
use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Metadata for a single attribute, fetched from EAV triples.
struct AttrMeta {
    short_name: String,
    qualified_name: String,
    description: Option<String>,
    type_str: Option<String>,
    cardinality: Option<String>,
    optional: bool,
}

/// Format a type string for human display.
///
/// If the stored value is a JSON array (e.g. `["tsp","mls"]`), display it as
/// `tsp | mls`. Otherwise return the string as-is (e.g. `Text`, `Integer`).
fn format_type_display(type_str: &str) -> String {
    if let Ok(items) = serde_json::from_str::<Vec<String>>(type_str) {
        items.join(" | ")
    } else {
        type_str.to_string()
    }
}

/// Build a compact annotation string for human output.
///
/// Combines cardinality, type, and optional into a parenthetical like
/// `(many, Text, optional)` or just `(Text)`.
fn format_annotation(meta: &AttrMeta) -> String {
    let mut parts = Vec::new();

    if meta.cardinality.as_deref() == Some("many") {
        parts.push("many".to_string());
    }

    if let Some(t) = &meta.type_str {
        parts.push(format_type_display(t));
    }

    if meta.optional {
        parts.push("optional".to_string());
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Convert an AttrMeta to a JSON value.
fn meta_to_json(meta: &AttrMeta) -> serde_json::Value {
    serde_json::json!({
        "name": meta.short_name,
        "qualified_name": meta.qualified_name,
        "description": meta.description,
        "type": meta.type_str,
        "cardinality": meta.cardinality,
        "optional": meta.optional,
    })
}

/// Collected info about a single concept and its attributes.
struct ConceptAttrs {
    name: String,
    namespace: Option<String>,
    attrs: Vec<AttrMeta>,
}

/// Fetch attribute metadata for a single qualified attribute name.
async fn fetch_attr_meta<S: dialog_artifacts::ArtifactStore>(
    store: &S,
    space_did: &str,
    concept_name: &ConceptName,
    qualified_name: &str,
) -> Result<AttrMeta> {
    let meta_entity = attribute_meta_entity(space_did, concept_name, qualified_name)?;

    let description = fetch_string(store, &meta_entity, ATTR_ATTRIBUTE_DESCRIPTION).await?;
    let type_str = fetch_string(store, &meta_entity, ATTR_ATTRIBUTE_TYPE).await?;
    let cardinality = fetch_string(store, &meta_entity, ATTR_ATTRIBUTE_CARDINALITY).await?;
    let optional_str = fetch_string(store, &meta_entity, ATTR_ATTRIBUTE_OPTIONAL).await?;
    let optional = optional_str.as_deref() == Some("true");

    Ok(AttrMeta {
        short_name: short_attribute(concept_name, qualified_name),
        qualified_name: qualified_name.to_string(),
        description,
        type_str,
        cardinality,
        optional,
    })
}

/// Load all concepts and their attribute metadata from the space.
async fn load_all_concepts<S: dialog_artifacts::ArtifactStore>(
    store: &S,
    space_did: &str,
) -> Result<Vec<ConceptAttrs>> {
    let registry = registry_entity(space_did)?;
    let concept_entities = fetch_entity_values(store, &registry, ATTR_REGISTRY_CONCEPT).await?;

    let mut concepts = Vec::new();

    for entity in &concept_entities {
        let name = fetch_string(store, entity, ATTR_CONCEPT_NAME)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Concept entity '{}' is missing its 'concept/name' attribute",
                    entity
                )
            })?;
        let concept_name = ConceptName::from_stored(name.clone());
        let namespace = fetch_string(store, entity, ATTR_CONCEPT_NAMESPACE).await?;
        let qualified_attrs = fetch_string_values(store, entity, ATTR_CONCEPT_ATTRIBUTE).await?;

        let mut attrs = Vec::new();
        for qname in &qualified_attrs {
            let meta = fetch_attr_meta(store, space_did, &concept_name, qname).await?;
            attrs.push(meta);
        }

        concepts.push(ConceptAttrs {
            name,
            namespace,
            attrs,
        });
    }

    Ok(concepts)
}

// ---------------------------------------------------------------------------
// List all attributes
// ---------------------------------------------------------------------------

/// List all attributes in the active space, grouped by concept.
pub async fn list(json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    let concepts = load_all_concepts(&branch, &ctx.space_did).await?;

    if concepts.is_empty() {
        if json {
            println!("[]");
        } else {
            println!(
                "No concepts defined. Use 'tonk concept define <name>' or 'tonk import <file>' to create one."
            );
        }
        return Ok(());
    }

    if json {
        let items: Vec<serde_json::Value> = concepts
            .iter()
            .map(|c| {
                let attrs_json: Vec<serde_json::Value> = c.attrs.iter().map(meta_to_json).collect();
                serde_json::json!({
                    "concept": c.name,
                    "namespace": c.namespace,
                    "attributes": attrs_json,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&items)?);
    } else {
        let total_concepts = concepts.len();
        let total_attrs: usize = concepts.iter().map(|c| c.attrs.len()).sum();

        println!("Attributes:\n");
        for c in &concepts {
            let ns_str = c
                .namespace
                .as_ref()
                .map(|ns| format!(" ({})", ns))
                .unwrap_or_default();
            println!("  {}{}:", c.name, ns_str);

            if c.attrs.is_empty() {
                println!("    (no attributes)");
            } else {
                // Calculate padding for aligned descriptions
                let max_name_len = c
                    .attrs
                    .iter()
                    .map(|a| a.short_name.len())
                    .max()
                    .unwrap_or(0);

                for attr in &c.attrs {
                    let padding = " ".repeat(max_name_len - attr.short_name.len());
                    let desc_str = attr
                        .description
                        .as_ref()
                        .map(|d| format!(" - {}", d))
                        .unwrap_or_default();
                    let annotation = format_annotation(attr);
                    println!(
                        "    {}{}{}{}",
                        attr.short_name, padding, desc_str, annotation
                    );
                }
            }

            println!();
        }

        let concept_word = if total_concepts == 1 {
            "concept"
        } else {
            "concepts"
        };
        let attr_word = if total_attrs == 1 {
            "attribute"
        } else {
            "attributes"
        };
        println!(
            "{} {}, {} {}",
            total_concepts, concept_word, total_attrs, attr_word
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Show a single attribute
// ---------------------------------------------------------------------------

/// Show details of a specific attribute.
///
/// The attribute can be specified as a qualified name (e.g. `recipe/title`) or
/// as a short name with `--concept` (e.g. `title --concept Recipe`).
pub async fn show(name: String, concept: Option<String>, json: bool) -> Result<()> {
    let ctx = get_space_context()?;
    let branch = open_branch(&ctx).await?;

    // Resolve the qualified name and owning concept
    let (concept_name, namespace, qualified_name) = if name.contains('/') {
        // Qualified name: split to find the concept prefix, then walk concepts
        // to find the match.
        resolve_qualified(&branch, &ctx.space_did, &name).await?
    } else if let Some(c) = concept {
        // Short name + --concept flag
        resolve_short(&branch, &ctx.space_did, &name, &c).await?
    } else {
        // Try to find by short name across all concepts
        resolve_unqualified(&branch, &ctx.space_did, &name).await?
    };

    let meta = fetch_attr_meta(&branch, &ctx.space_did, &concept_name, &qualified_name).await?;

    if json {
        let mut obj = meta_to_json(&meta);
        obj.as_object_mut().unwrap().insert(
            "concept".to_string(),
            serde_json::json!(concept_name.as_str()),
        );
        if let Some(ns) = &namespace {
            obj.as_object_mut()
                .unwrap()
                .insert("namespace".to_string(), serde_json::json!(ns));
        }
        println!("{}", serde_json::to_string(&obj)?);
    } else {
        println!("Attribute: {}", qualified_name);
        println!("  Concept:     {}", concept_name);
        if let Some(ns) = &namespace {
            println!("  Namespace:   {}", ns);
        }
        if let Some(desc) = &meta.description {
            println!("  Description: {}", desc);
        }
        let type_display = meta
            .type_str
            .as_ref()
            .map(|t| format_type_display(t))
            .unwrap_or_else(|| "(untyped)".to_string());
        println!("  Type:        {}", type_display);
        let card = if meta.cardinality.as_deref() == Some("many") {
            "many"
        } else {
            "single"
        };
        println!("  Cardinality: {}", card);
        println!(
            "  Optional:    {}",
            if meta.optional { "yes" } else { "no" }
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Name resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a qualified attribute name (e.g. `recipe/title`) to its concept.
///
/// Walks all concepts in the registry to find one whose `concept/attribute`
/// list contains the given qualified name.
async fn resolve_qualified<S: dialog_artifacts::ArtifactStore>(
    store: &S,
    space_did: &str,
    qualified_name: &str,
) -> Result<(ConceptName, Option<String>, String)> {
    let normalized = if let Some((prefix, attr)) = qualified_name.split_once('/') {
        format!("{}/{}", prefix.to_lowercase(), attr)
    } else {
        qualified_name.to_string()
    };
    let registry = registry_entity(space_did)?;
    let concept_entities = fetch_entity_values(store, &registry, ATTR_REGISTRY_CONCEPT).await?;

    for entity in &concept_entities {
        let attrs = fetch_string_values(store, entity, ATTR_CONCEPT_ATTRIBUTE).await?;
        if attrs.iter().any(|a| a == &normalized) {
            let name = fetch_string(store, entity, ATTR_CONCEPT_NAME)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Concept entity missing concept/name"))?;
            let namespace = fetch_string(store, entity, ATTR_CONCEPT_NAMESPACE).await?;
            return Ok((ConceptName::from_stored(name), namespace, normalized));
        }
    }

    anyhow::bail!(
        "Attribute '{}' not found. Run 'tonk attribute' to list available attributes.",
        qualified_name
    );
}

/// Resolve a short attribute name given a concept name.
async fn resolve_short<S: dialog_artifacts::ArtifactStore>(
    store: &S,
    space_did: &str,
    short_name: &str,
    concept_str: &str,
) -> Result<(ConceptName, Option<String>, String)> {
    let concept_name = ConceptName::new(concept_str)?;
    let concept = concept_entity(space_did, &concept_name)?;

    // Verify concept exists
    let stored_name = fetch_string(store, &concept, ATTR_CONCEPT_NAME)
        .await?
        .context(format!("Concept '{}' not found", concept_name))?;
    let stored_name = ConceptName::from_stored(stored_name);

    let namespace = fetch_string(store, &concept, ATTR_CONCEPT_NAMESPACE).await?;
    let qualified = qualify_attribute(&stored_name, short_name)?;

    // Verify the attribute actually exists on this concept
    let attrs = fetch_string_values(store, &concept, ATTR_CONCEPT_ATTRIBUTE).await?;
    if !attrs.contains(&qualified) {
        anyhow::bail!(
            "Attribute '{}' not found on concept '{}'. Available attributes: {}",
            short_name,
            stored_name,
            attrs
                .iter()
                .map(|a| short_attribute(&stored_name, a))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok((stored_name, namespace, qualified))
}

/// Resolve a short attribute name without a concept — search all concepts.
///
/// If exactly one concept has an attribute with this short name, return it.
/// If multiple concepts have it, bail with a disambiguation message.
async fn resolve_unqualified<S: dialog_artifacts::ArtifactStore>(
    store: &S,
    space_did: &str,
    short_name: &str,
) -> Result<(ConceptName, Option<String>, String)> {
    let registry = registry_entity(space_did)?;
    let concept_entities = fetch_entity_values(store, &registry, ATTR_REGISTRY_CONCEPT).await?;

    let mut matches: Vec<(ConceptName, Option<String>, String)> = Vec::new();

    for entity in &concept_entities {
        let name = fetch_string(store, entity, ATTR_CONCEPT_NAME)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Concept entity missing concept/name"))?;
        let concept_name = ConceptName::from_stored(name);
        let namespace = fetch_string(store, entity, ATTR_CONCEPT_NAMESPACE).await?;
        let attrs = fetch_string_values(store, entity, ATTR_CONCEPT_ATTRIBUTE).await?;

        for qname in &attrs {
            if short_attribute(&concept_name, qname) == short_name {
                matches.push((concept_name.clone(), namespace.clone(), qname.clone()));
                break;
            }
        }
    }

    match matches.len() {
        0 => {
            anyhow::bail!(
                "Attribute '{}' not found in any concept. Run 'tonk attribute' to list available attributes.",
                short_name
            );
        }
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            let concept_list: Vec<String> = matches
                .iter()
                .map(|(c, _, q)| format!("  {} ({})", c, q))
                .collect();
            anyhow::bail!(
                "Attribute '{}' exists in multiple concepts. Use --concept to disambiguate:\n{}",
                short_name,
                concept_list.join("\n")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- format_type_display -------------------------------------------------

    #[test]
    fn format_type_display_simple_string() {
        assert_eq!(format_type_display("Text"), "Text");
        assert_eq!(format_type_display("Integer"), "Integer");
        assert_eq!(format_type_display("RecipeStep"), "RecipeStep");
    }

    #[test]
    fn format_type_display_json_array_enum() {
        assert_eq!(format_type_display(r#"["tsp","mls"]"#), "tsp | mls");
    }

    #[test]
    fn format_type_display_single_element_array() {
        assert_eq!(format_type_display(r#"["only"]"#), "only");
    }

    #[test]
    fn format_type_display_empty_array() {
        assert_eq!(format_type_display("[]"), "");
    }

    #[test]
    fn format_type_display_invalid_json_passthrough() {
        // Malformed JSON should be returned as-is
        assert_eq!(format_type_display("[not json"), "[not json");
    }

    // -- format_annotation ---------------------------------------------------

    fn make_meta(cardinality: Option<&str>, type_str: Option<&str>, optional: bool) -> AttrMeta {
        AttrMeta {
            short_name: "test".to_string(),
            qualified_name: "concept/test".to_string(),
            description: None,
            type_str: type_str.map(|s| s.to_string()),
            cardinality: cardinality.map(|s| s.to_string()),
            optional,
        }
    }

    #[test]
    fn annotation_empty_when_no_metadata() {
        let meta = make_meta(None, None, false);
        assert_eq!(format_annotation(&meta), "");
    }

    #[test]
    fn annotation_type_only() {
        let meta = make_meta(None, Some("Text"), false);
        assert_eq!(format_annotation(&meta), " (Text)");
    }

    #[test]
    fn annotation_many_and_type() {
        let meta = make_meta(Some("many"), Some("RecipeStep"), false);
        assert_eq!(format_annotation(&meta), " (many, RecipeStep)");
    }

    #[test]
    fn annotation_many_only() {
        let meta = make_meta(Some("many"), None, false);
        assert_eq!(format_annotation(&meta), " (many)");
    }

    #[test]
    fn annotation_optional_only() {
        let meta = make_meta(None, None, true);
        assert_eq!(format_annotation(&meta), " (optional)");
    }

    #[test]
    fn annotation_all_three() {
        let meta = make_meta(Some("many"), Some("Text"), true);
        assert_eq!(format_annotation(&meta), " (many, Text, optional)");
    }

    #[test]
    fn annotation_enum_type() {
        let meta = make_meta(None, Some(r#"["tsp","mls"]"#), false);
        assert_eq!(format_annotation(&meta), " (tsp | mls)");
    }

    #[test]
    fn annotation_single_cardinality_ignored() {
        // Only "many" gets shown; other cardinality values are ignored
        let meta = make_meta(Some("single"), Some("Text"), false);
        assert_eq!(format_annotation(&meta), " (Text)");
    }

    // -- meta_to_json --------------------------------------------------------

    #[test]
    fn meta_to_json_with_all_fields() {
        let meta = AttrMeta {
            short_name: "title".to_string(),
            qualified_name: "recipe/title".to_string(),
            description: Some("The name of this recipe".to_string()),
            type_str: Some("Text".to_string()),
            cardinality: None,
            optional: false,
        };
        let json = meta_to_json(&meta);
        assert_eq!(json["name"], "title");
        assert_eq!(json["qualified_name"], "recipe/title");
        assert_eq!(json["description"], "The name of this recipe");
        assert_eq!(json["type"], "Text");
        assert!(json["cardinality"].is_null());
        assert_eq!(json["optional"], false);
    }

    #[test]
    fn meta_to_json_minimal() {
        let meta = AttrMeta {
            short_name: "tag".to_string(),
            qualified_name: "task/tag".to_string(),
            description: None,
            type_str: None,
            cardinality: None,
            optional: false,
        };
        let json = meta_to_json(&meta);
        assert_eq!(json["name"], "tag");
        assert!(json["description"].is_null());
        assert!(json["type"].is_null());
        assert_eq!(json["optional"], false);
    }

    #[test]
    fn meta_to_json_optional_many() {
        let meta = AttrMeta {
            short_name: "step".to_string(),
            qualified_name: "recipe/step".to_string(),
            description: Some("A cooking step".to_string()),
            type_str: Some("RecipeStep".to_string()),
            cardinality: Some("many".to_string()),
            optional: true,
        };
        let json = meta_to_json(&meta);
        assert_eq!(json["cardinality"], "many");
        assert_eq!(json["optional"], true);
        assert_eq!(json["type"], "RecipeStep");
    }
}
