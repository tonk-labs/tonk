//! Persisted typed edges and graph-schema resolution.
//!
//! Targets are bookmarks, so resolving a cycle never recursively constructs or
//! hashes a concept descriptor. Ordinary concepts have no edge metadata.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dialog_artifacts::{Entity, Value};
use dialog_query::ConceptQuery;
use dialog_query::graph::{GraphEdge, GraphQuery, GraphSchema, GraphShape};

use crate::concept::{ConceptLookupError, QueryEnv};
use crate::query_source::Source;
use crate::resolution::{ConceptReference, NamedReference};

/// Explicit query-side equalities, separated by the stage they affect.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphConstraints {
    /// Root-edge target equalities: these may remove roots.
    pub matching: BTreeMap<String, BTreeMap<String, Value>>,
    /// Edge target equalities: these never remove roots.
    pub shaping: BTreeMap<String, BTreeMap<String, Value>>,
}

impl GraphConstraints {
    /// Bind constants into the graph scans, validating all edge and field names.
    pub fn apply(&self, graph: &mut GraphQuery) -> Result<(), ConceptLookupError> {
        for (stage, constraints) in [("matching", &self.matching), ("shaping", &self.shaping)] {
            for (field, equalities) in constraints {
                let edge = graph.schema.shapes[&graph.shape]
                    .edges
                    .get(field)
                    .ok_or_else(|| {
                        ConceptLookupError::query(format!("unknown graph edge {field:?}"))
                    })?;
                let target = &graph.schema.shapes[&edge.target].concept;
                let mut bound = BTreeMap::new();
                for (name, value) in equalities {
                    let descriptor = target
                        .with()
                        .iter()
                        .find(|(field, _)| field == name)
                        .map(|(_, descriptor)| descriptor)
                        .ok_or_else(|| {
                            ConceptLookupError::query(format!("unknown target field {name:?}"))
                        })?;
                    let value = match descriptor.content_type() {
                        Some(kind) => kind
                            .holds(value)
                            .then(|| value.clone().viewed_as(kind))
                            .flatten()
                            .ok_or_else(|| {
                                ConceptLookupError::query(format!(
                                    "graph equality {field}.{name} does not fit its type"
                                ))
                            })?,
                        None => value.clone(),
                    };
                    bound.insert(name.clone(), value);
                }
                let edge = graph
                    .schema
                    .shapes
                    .get_mut(&graph.shape)
                    .expect("resolved root")
                    .edges
                    .get_mut(field)
                    .expect("validated edge");
                if stage == "matching" {
                    edge.matching.extend(bound);
                } else {
                    edge.shaping.extend(bound);
                }
            }
        }
        graph
            .validate()
            .map_err(|error| ConceptLookupError::query(error.to_string()))
    }
}

/// The stored relation naming a field's graph target.
pub const EDGE_DOMAIN: &str = "db.concept.edge";

/// Resolve persisted graph metadata for an explicit HTTP query. `None`
/// indicates that the selected concept has no typed edges. Every target must resolve; unresolved bookmarks are an
/// error, rather than silently returning an incomplete graph.
pub async fn resolve_graph<Env: QueryEnv>(
    root: ConceptQuery,
    name: &str,
    source: &Source<'_>,
    env: &Env,
) -> Result<Option<GraphQuery>, ConceptLookupError> {
    let root_definition = reference(name).resolve(source).perform(env).await?;
    let Some(root_definition) = root_definition else {
        return Ok(None);
    };
    let edges = definition_edges(&root_definition);
    if edges.is_empty() {
        return Ok(None);
    }
    let shape = root_definition.entity.to_string();
    let mut schema = GraphSchema::default();
    let mut pending = VecDeque::from([(root_definition, edges)]);
    let mut scheduled = BTreeSet::from([shape.clone()]);
    while let Some((definition, mut edges)) = pending.pop_front() {
        for edge in edges.values_mut() {
            let target = reference(&edge.target)
                .resolve(source)
                .perform(env)
                .await?
                .ok_or_else(|| {
                    ConceptLookupError::query(format!(
                        "graph target {:?} is not a concept",
                        edge.target
                    ))
                })?;
            edge.target = target.entity.to_string();
            if scheduled.insert(edge.target.clone()) {
                let target_edges = definition_edges(&target);
                pending.push_back((target, target_edges));
            }
        }
        schema.shapes.insert(
            definition.entity.to_string(),
            GraphShape {
                concept: definition.descriptor.concept().clone(),
                edges,
            },
        );
    }
    let graph = GraphQuery {
        root,
        shape,
        schema,
    };
    graph
        .validate()
        .map_err(|error| ConceptLookupError::query(error.to_string()))?;
    Ok(Some(graph))
}

fn reference(name: &str) -> ConceptReference {
    match name.parse::<Entity>() {
        Ok(entity) => ConceptReference::from(entity),
        Err(_) => ConceptReference::from(NamedReference(name.into())),
    }
}

fn definition_edges(
    definition: &crate::resolution::ConceptDefinition,
) -> BTreeMap<String, GraphEdge> {
    definition
        .edges
        .iter()
        .map(|(field, target)| {
            (
                field.clone(),
                GraphEdge {
                    target: target.clone(),
                    matching: BTreeMap::new(),
                    shaping: BTreeMap::new(),
                },
            )
        })
        .collect()
}
