//! `carry query` — query entities by domain or concept.
//!
//! Supports both domain queries (`carry query io.gozala.person name age`)
//! and concept queries (`carry query person name="Alice"`).

use crate::schema;
use crate::site::SiteContext;
use crate::target::{Field, Target};
use anyhow::Result;
use dialog_query::Value;
use std::collections::BTreeMap;

/// Execute `carry query <TARGET> [FIELD[=VALUE]...]`.
pub async fn execute(
    ctx: &SiteContext,
    target: Target,
    fields: Vec<Field>,
    format: &str,
) -> Result<()> {
    match target {
        Target::Domain(ref domain) => domain_query(ctx, domain, &fields, format).await,
        Target::Concept(ref concept) => concept_query(ctx, concept, &fields, format).await,
    }
}

/// Domain query: open-ended search over a domain namespace.
async fn domain_query(
    ctx: &SiteContext,
    domain: &str,
    fields: &[Field],
    format: &str,
) -> Result<()> {
    if fields.is_empty() {
        anyhow::bail!("Domain query requires at least one field");
    }

    let session = ctx.open_session().await?;

    // Build qualified attribute names
    let qualified_fields: Vec<(String, Option<String>)> = fields
        .iter()
        .map(|f| (f.qualified_name(domain), f.value.clone()))
        .collect();

    // Separate filters from projections
    let filter_attrs: Vec<(&str, &str)> = qualified_fields
        .iter()
        .filter_map(|(name, val)| val.as_deref().map(|v| (name.as_str(), v)))
        .collect();
    let all_attr_names: Vec<&str> = qualified_fields.iter().map(|f| f.0.as_str()).collect();

    // Find entities that have ANY of the requested attributes
    // (we start with the first attribute and filter down)
    let first_attr = schema::parse_claim_attribute(all_attr_names[0])?;
    let candidate_entities = schema::find_entities_by_attribute(&session, first_attr).await?;

    // For each candidate, check filters and collect values
    let mut results: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();

    for entity in &candidate_entities {
        let mut entity_values: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut matches_filters = true;

        for attr_name in &all_attr_names {
            let attr = schema::parse_claim_attribute(attr_name)?;
            let values = schema::fetch_values(&session, entity, attr).await?;

            if values.is_empty() {
                continue;
            }

            // Check filter constraints
            for (filter_attr, filter_val) in &filter_attrs {
                if *filter_attr == *attr_name {
                    let expected = schema::parse_value(filter_val);
                    if !values.contains(&expected) {
                        matches_filters = false;
                        break;
                    }
                }
            }

            if !matches_filters {
                break;
            }

            entity_values.insert(attr_name.to_string(), values);
        }

        if matches_filters && !entity_values.is_empty() {
            results.insert(entity.to_string(), entity_values);
        }
    }

    output_results(&results, domain, format)
}

/// Concept query: resolve a named concept and match entities.
async fn concept_query(
    ctx: &SiteContext,
    concept_name: &str,
    fields: &[Field],
    format: &str,
) -> Result<()> {
    let session = ctx.open_session().await?;

    // Look up the concept by name
    let cname = schema::ConceptName::new(concept_name)?;
    let concept_entity = schema::lookup_concept_by_name(&session, &cname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Concept '{}' not found", concept_name))?;

    // Get schema attributes for this concept
    let schema_attrs = schema::fetch_string_values(
        &session,
        &concept_entity,
        schema::concept_attribute_selector(),
    )
    .await?;

    if schema_attrs.is_empty() {
        anyhow::bail!("Concept '{}' has no attributes", concept_name);
    }

    // Find entities belonging to this concept
    let entities = schema::find_entities_by_concept(&session, &schema_attrs).await?;

    // Determine which attributes to show
    let namespace = schema::fetch_string_values(
        &session,
        &concept_entity,
        schema::concept_namespace_selector(),
    )
    .await?
    .into_iter()
    .next()
    .unwrap_or_else(|| concept_name.to_string());

    // If fields are specified, use them as filters/projections
    // Otherwise show all concept attributes
    let (show_attrs, filter_pairs): (Vec<String>, Vec<(String, String)>) = if fields.is_empty() {
        (schema_attrs.clone(), Vec::new())
    } else {
        let mut show = Vec::new();
        let mut filters = Vec::new();
        for f in fields {
            let qname = f.qualified_name(&namespace);
            show.push(qname.clone());
            if let Some(ref v) = f.value {
                filters.push((qname, v.clone()));
            }
        }
        (show, filters)
    };

    let mut results: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();

    for entity in &entities {
        let mut entity_values: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut matches_filters = true;

        for attr_name in &show_attrs {
            let attr = schema::parse_claim_attribute(attr_name)?;
            let values = schema::fetch_values(&session, entity, attr).await?;

            if values.is_empty() {
                continue;
            }

            // Check filters
            for (filter_attr, filter_val) in &filter_pairs {
                if filter_attr == attr_name {
                    let expected = schema::parse_value(filter_val);
                    if !values.contains(&expected) {
                        matches_filters = false;
                        break;
                    }
                }
            }

            if !matches_filters {
                break;
            }

            entity_values.insert(attr_name.clone(), values);
        }

        if matches_filters && !entity_values.is_empty() {
            results.insert(entity.to_string(), entity_values);
        }
    }

    output_results(&results, &namespace, format)
}

/// Format and print query results.
fn output_results(
    results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    namespace: &str,
    format: &str,
) -> Result<()> {
    if results.is_empty() {
        return Ok(());
    }

    match format {
        "json" => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|(entity_id, attrs)| {
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "id".to_string(),
                        serde_json::Value::String(entity_id.clone()),
                    );
                    for (attr, values) in attrs {
                        let short = schema::short_attribute(namespace, attr);
                        if values.len() == 1 {
                            obj.insert(short, schema::value_to_json(&values[0]));
                        } else {
                            obj.insert(
                                short,
                                serde_json::Value::Array(
                                    values.iter().map(schema::value_to_json).collect(),
                                ),
                            );
                        }
                    }
                    serde_json::Value::Object(obj)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json_results)?);
        }
        _ => {
            // YAML output (default)
            for (entity_id, attrs) in results {
                println!("{}:", entity_id);
                println!("  {}:", namespace);
                for (attr, values) in attrs {
                    let short = schema::short_attribute(namespace, attr);
                    if values.len() == 1 {
                        println!("    {}: {}", short, schema::format_value(&values[0]));
                    } else {
                        println!("    {}:", short);
                        for v in values {
                            println!("      - {}", schema::format_value(v));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
