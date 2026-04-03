//! `carry query` -- query entities by domain or concept.
//!
//! Supports both domain queries (`carry query io.gozala.person name age`)
//! and concept queries (`carry query person name="Alice"`).
//!
//! Concept queries resolve the concept via `dialog.meta/name`, fetch its
//! `dialog.concept.with/*` attributes, and return all matching entities
//! with output grouped under the concept's bookmark name.

use crate::schema;
use crate::site::Site;
use crate::target::{Field, Target};
use anyhow::Result;
use dialog_query::Value;
use std::collections::BTreeMap;

/// A concept field name mapped to its attribute selector (for display).
struct FieldMapping {
    field_name: String,
    selector: String,
}

/// An attribute selector with a required value (for filtering).
struct FilterConstraint {
    selector: String,
    value: String,
}

/// Query results: entity DID -> attribute name -> values.
pub type QueryResults = BTreeMap<String, BTreeMap<String, Vec<Value>>>;

/// Execute `carry query <TARGET> [FIELD[=VALUE]...]` -- prints results to stdout.
pub async fn execute(site: &Site, target: Target, fields: Vec<Field>, format: &str) -> Result<()> {
    let (results, namespace) = query(site, target.clone(), fields).await?;
    match target {
        Target::Domain(_) => output_results(&results, &namespace, format),
        Target::Concept(ref concept) => {
            if format == "triples" {
                let resolved =
                    schema::resolve_concept(&site.branch, &site.operator, concept).await?;
                let remapped = remap_to_selectors(&results, &resolved);
                output_triples(&remapped)
            } else {
                output_concept_results(&results, concept, format)
            }
        }
    }
}

/// Query without printing -- returns the result map and the namespace/concept name.
pub async fn query(
    site: &Site,
    target: Target,
    fields: Vec<Field>,
) -> Result<(QueryResults, String)> {
    match target {
        Target::Domain(ref domain) => {
            let results = domain_query_results(site, domain, &fields).await?;
            Ok((results, domain.clone()))
        }
        Target::Concept(ref concept) => {
            let results = concept_query_results(site, concept, &fields).await?;
            Ok((results, concept.clone()))
        }
    }
}

/// Domain query: return matching entities and their attribute values.
async fn domain_query_results(site: &Site, domain: &str, fields: &[Field]) -> Result<QueryResults> {
    if fields.is_empty() {
        anyhow::bail!("Domain query requires at least one field");
    }

    let branch = &site.branch;
    let operator = &site.operator;

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
    let first_attr = schema::parse_claim_attribute(all_attr_names[0])?;
    let candidate_entities =
        schema::find_entities_by_attribute(branch, operator, first_attr).await?;

    // For each candidate, check filters and collect values
    let mut results: QueryResults = BTreeMap::new();

    for entity in &candidate_entities {
        let mut entity_values: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut matches_filters = true;

        for attr_name in &all_attr_names {
            let attr = schema::parse_claim_attribute(attr_name)?;
            let values = schema::fetch_values(branch, operator, entity, attr).await?;

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

    Ok(results)
}

/// Concept query: return matching entities and their attribute values.
///
/// Keys in the returned map use the concept's short field names (not qualified selectors).
async fn concept_query_results(
    site: &Site,
    concept_name: &str,
    fields: &[Field],
) -> Result<QueryResults> {
    let branch = &site.branch;
    let operator = &site.operator;

    // Resolve the concept from the database
    let concept = schema::resolve_concept(branch, operator, concept_name).await?;

    // Get the attribute selectors for structural membership matching
    let schema_attrs = schema::concept_attribute_selectors(&concept);

    if schema_attrs.is_empty() {
        anyhow::bail!("Concept '{}' has no attributes", concept_name);
    }

    // Find entities belonging to this concept (structural matching)
    let entities = schema::find_entities_by_concept(branch, operator, &schema_attrs).await?;

    // Determine which attributes to show and which to filter by
    let (show_attrs, filter_pairs): (Vec<FieldMapping>, Vec<FilterConstraint>) =
        if fields.is_empty() {
            let show: Vec<FieldMapping> = concept
                .with_fields
                .iter()
                .chain(concept.maybe_fields.iter())
                .map(|(field_name, (_, selector))| FieldMapping {
                    field_name: field_name.clone(),
                    selector: selector.clone(),
                })
                .collect();
            (show, Vec::new())
        } else {
            let mut show = Vec::new();
            let mut filters = Vec::new();
            for f in fields {
                let selector = schema::resolve_field_selector(&concept, &f.name)?;
                show.push(FieldMapping {
                    field_name: f.name.clone(),
                    selector: selector.clone(),
                });
                if let Some(ref v) = f.value {
                    filters.push(FilterConstraint {
                        selector,
                        value: v.clone(),
                    });
                }
            }
            (show, filters)
        };

    let mut results: QueryResults = BTreeMap::new();

    for entity in &entities {
        let mut entity_values: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut matches_filters = true;

        for mapping in &show_attrs {
            let attr = schema::parse_claim_attribute(&mapping.selector)?;
            let values = schema::fetch_values(branch, operator, entity, attr).await?;

            if values.is_empty() {
                continue;
            }

            // Check filters
            for filter in &filter_pairs {
                if filter.selector == mapping.selector {
                    let expected = schema::parse_value(&filter.value);
                    if !values.contains(&expected) {
                        matches_filters = false;
                        break;
                    }
                }
            }

            if !matches_filters {
                break;
            }

            entity_values.insert(mapping.field_name.clone(), values);
        }

        if matches_filters && !entity_values.is_empty() {
            results.insert(entity.to_string(), entity_values);
        }
    }

    Ok(results)
}

/// Remap short field names back to qualified selectors for triples output.
fn remap_to_selectors(results: &QueryResults, concept: &schema::ResolvedConcept) -> QueryResults {
    let field_to_selector: BTreeMap<&str, &str> = concept
        .with_fields
        .iter()
        .chain(concept.maybe_fields.iter())
        .map(|(name, (_, sel))| (name.as_str(), sel.as_str()))
        .collect();

    results
        .iter()
        .map(|(entity, attrs)| {
            let remapped = attrs
                .iter()
                .map(|(field, values)| {
                    let key = field_to_selector
                        .get(field.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| field.clone());
                    (key, values.clone())
                })
                .collect();
            (entity.clone(), remapped)
        })
        .collect()
}

fn output_results(
    results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    namespace: &str,
    format: &str,
) -> Result<()> {
    if results.is_empty() {
        return Ok(());
    }

    match format {
        "triples" => output_triples(results),
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
            Ok(())
        }
        _ => {
            let yaml = format_asserted_yaml(results, namespace);
            print!("{}", yaml);
            Ok(())
        }
    }
}

/// Format and print concept query results (grouped under concept name).
fn output_concept_results(
    results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    concept_name: &str,
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
                    for (field_name, values) in attrs {
                        if values.len() == 1 {
                            obj.insert(field_name.clone(), schema::value_to_json(&values[0]));
                        } else {
                            obj.insert(
                                field_name.clone(),
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
            for (entity_id, attrs) in results {
                println!("{}:", entity_id);
                println!("  {}:", concept_name);
                for (field_name, values) in attrs {
                    if values.len() == 1 {
                        println!("    {}: {}", field_name, schema::format_value(&values[0]));
                    } else {
                        println!("    {}:", field_name);
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

/// Format and print results as EAV triples in YAML.
fn output_triples(results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>) -> Result<()> {
    let yaml = format_triples(results)?;
    if !yaml.is_empty() {
        print!("{}", yaml);
    }
    Ok(())
}

/// Format results as asserted notation YAML string (entity-grouped).
pub fn format_asserted_yaml(
    results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    namespace: &str,
) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for (entity_id, attrs) in results {
        writeln!(output, "{}:", entity_id).unwrap();
        writeln!(output, "  {}:", namespace).unwrap();
        for (attr, values) in attrs {
            let short = schema::short_attribute(namespace, attr);
            if values.len() == 1 {
                writeln!(
                    output,
                    "    {}: {}",
                    short,
                    schema::format_value(&values[0])
                )
                .unwrap();
            } else {
                writeln!(output, "    {}:", short).unwrap();
                for v in values {
                    writeln!(output, "      - {}", schema::format_value(v)).unwrap();
                }
            }
        }
    }
    output
}

/// Format results as EAV triple YAML string.
pub fn format_triples(results: &BTreeMap<String, BTreeMap<String, Vec<Value>>>) -> Result<String> {
    if results.is_empty() {
        return Ok(String::new());
    }

    let mut triples: Vec<serde_yaml::Value> = Vec::new();

    for (entity_id, attrs) in results {
        for (attr_name, values) in attrs {
            for value in values {
                let mut map = serde_yaml::Mapping::new();
                map.insert(
                    serde_yaml::Value::String("the".to_string()),
                    serde_yaml::Value::String(attr_name.clone()),
                );
                map.insert(
                    serde_yaml::Value::String("of".to_string()),
                    serde_yaml::Value::String(entity_id.clone()),
                );
                map.insert(
                    serde_yaml::Value::String("is".to_string()),
                    value_to_yaml(value),
                );
                triples.push(serde_yaml::Value::Mapping(map));
            }
        }
    }

    Ok(serde_yaml::to_string(&triples)?)
}

/// Convert a dialog_query::Value to a serde_yaml::Value.
fn value_to_yaml(value: &Value) -> serde_yaml::Value {
    match value {
        Value::String(s) => serde_yaml::Value::String(s.clone()),
        Value::UnsignedInt(n) => {
            if *n <= u64::MAX as u128 {
                serde_yaml::Value::Number(serde_yaml::Number::from(*n as u64))
            } else {
                serde_yaml::Value::String(n.to_string())
            }
        }
        Value::SignedInt(n) => {
            if *n >= i64::MIN as i128 && *n <= i64::MAX as i128 {
                serde_yaml::Value::Number(serde_yaml::Number::from(*n as i64))
            } else {
                serde_yaml::Value::String(n.to_string())
            }
        }
        Value::Float(f) => {
            serde_yaml::to_value(f).unwrap_or_else(|_| serde_yaml::Value::String(f.to_string()))
        }
        Value::Boolean(b) => serde_yaml::Value::Bool(*b),
        Value::Entity(e) => serde_yaml::Value::String(e.to_string()),
        Value::Symbol(s) => serde_yaml::Value::String(format!(":{}", s)),
        Value::Bytes(b) => serde_yaml::Value::String(format!("<{} bytes>", b.len())),
        Value::Record(r) => serde_yaml::Value::String(format!("<{} bytes record>", r.len())),
    }
}
