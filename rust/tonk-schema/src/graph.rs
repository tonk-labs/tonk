//! Persisted typed edges and graph-schema resolution.
//!
//! Targets are bookmarks, so resolving a cycle never recursively constructs or
//! hashes a concept descriptor. Ordinary concepts have no edge metadata.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dialog_artifacts::{Entity, Value};
use dialog_query::graph::{GraphEdge, GraphQuery, GraphSchema, GraphShape};
use dialog_query::{AttributeQuery, ConceptQuery, Output as _, Term};

use crate::concept::{ConceptLookupError, QueryEnv};
use crate::query_source::Source;
use crate::resolution::{ConceptReference, NamedReference};

/// Explicit query-side equalities, separated by the stage they affect.
#[derive(Debug, Clone, Default)]
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

/// Resolve graph metadata for a normal query. `None` leaves an ordinary query
/// on its existing path. Every target must resolve; unresolved bookmarks are an
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
    let edges = read_edges(&root_definition.entity, source, env).await?;
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
                let target_edges = read_edges(&target.entity, source, env).await?;
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

async fn read_edges<Env: QueryEnv>(
    entity: &Entity,
    source: &Source<'_>,
    env: &Env,
) -> Result<BTreeMap<String, GraphEdge>, ConceptLookupError> {
    let claims = source
        .select(AttributeQuery::new(
            Term::var("relation"),
            Term::from(entity.clone()),
            Term::var("target"),
            Term::blank(),
            Some(dialog_query::Cardinality::One),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| ConceptLookupError::query(error.to_string()))?;
    let mut edges = BTreeMap::new();
    for claim in claims {
        let relation: dialog_artifacts::Relation = claim.the.into();
        let Some(field) = relation.as_str().strip_prefix("db.concept.edge/") else {
            continue;
        };
        let Value::String(target) = claim.is else {
            continue;
        };
        if edges
            .insert(
                field.into(),
                GraphEdge {
                    target,
                    matching: BTreeMap::new(),
                    shaping: BTreeMap::new(),
                },
            )
            .is_some()
        {
            return Err(ConceptLookupError::query(format!(
                "conflicting graph targets for {entity}.{field}"
            )));
        }
    }
    Ok(edges)
}
