//! Read-only nominal/legacy command inventory for migration planning.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow};
use dialog_artifacts::{Entity, Value};
use dialog_query::{AttributeQuery, Output as _, Term, attribute};
use serde::Serialize;
use tonk_schema::command_definition::CommandDefinition;
use tonk_schema::concept::Concept;
use tonk_schema::projection::{ProjectionDefinition, ProjectionDescriptor};
use tonk_schema::query_source::Source;

use crate::site::TonkSite;

/// Complete branch-local command inventory used to prepare migrations.
#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    /// Observed branch tree. Consumers must treat the inventory as stale once
    /// this revision changes.
    pub revision: String,
    /// Persisted nominal command declarations.
    pub nominal: Vec<NominalCommand>,
    /// Structural transient declarations retained for compatibility.
    pub legacy: Vec<LegacyCommand>,
    /// Nominal rule reverse-index rows, including the exact stored source.
    pub effects: Vec<CommandEffect>,
    /// Durable string claims that appear to contain event bindings.
    pub event_bindings: Vec<EventBinding>,
}

/// One nominal declaration and its resolved projections.
#[derive(Debug, Clone, Serialize)]
pub struct NominalCommand {
    /// Published name when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Stable nominal kind.
    pub kind: String,
    /// Content-derived current schema entity.
    pub schema_entity: String,
    /// Exact canonical schema source stored on the branch.
    pub source: String,
    /// Projection declarations selecting this kind.
    pub projections: Vec<ProjectionInventory>,
}

/// One stored event projection.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionInventory {
    /// Published name when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Stable projection entity.
    pub entity: String,
    /// Parsed descriptor for machine-readable audits.
    pub descriptor: ProjectionDescriptor,
    /// Exact canonical descriptor source stored on the branch.
    pub source: String,
}

/// One legacy structural transient declaration.
#[derive(Debug, Clone, Serialize)]
pub struct LegacyCommand {
    /// Published name when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Structural concept entity.
    pub entity: String,
    /// Reconstructed concept descriptor JSON.
    pub source: String,
}

/// One nominal command-consuming rule index.
#[derive(Debug, Clone, Serialize)]
pub struct CommandEffect {
    /// Stable command kind selected by the rule.
    pub command: String,
    /// Durable effect entity.
    pub effect: String,
    /// Exact stored `dialog.effect/source` bytes.
    pub source: String,
}

/// A likely view/template event-binding carrier.
#[derive(Debug, Clone, Serialize)]
pub struct EventBinding {
    /// Entity carrying the source string.
    pub entity: String,
    /// Attribute carrying the source string.
    pub attribute: String,
    /// Binding references conservatively extracted from markup.
    pub references: Vec<String>,
}

/// Inspect the selected branch without mutating it.
pub async fn inventory(site: &TonkSite) -> Result<Inventory> {
    let session = site.branch().await?;
    let branch = session.handle();
    let source = Source::from(branch);
    let names = names_by_referent(branch, &site.operator).await?;

    let command_entities =
        marker_subjects(branch, &site.operator, "dialog.meta/command", "db:command").await?;
    let mut nominal = Vec::with_capacity(command_entities.len());
    for kind in command_entities {
        let definition = CommandDefinition::by_entity(kind.clone())
            .resolve(&source, &site.operator)
            .await?
            .ok_or_else(|| anyhow!("command marker {kind} has no resolvable definition"))?;
        let mut projections = ProjectionDefinition::for_command(kind.clone())
            .resolve(&source, &site.operator)
            .await?
            .into_iter()
            .map(|projection| ProjectionInventory {
                name: names.get(projection.this()).cloned(),
                entity: projection.this().to_string(),
                descriptor: projection.descriptor().clone(),
                source: projection.source().to_owned(),
            })
            .collect::<Vec<_>>();
        projections.sort_by(|left, right| left.entity.cmp(&right.entity));
        nominal.push(NominalCommand {
            name: names.get(&kind).cloned(),
            kind: kind.to_string(),
            schema_entity: definition.schema_entity().to_string(),
            source: definition.source().to_owned(),
            projections,
        });
    }
    nominal.sort_by(|left, right| left.kind.cmp(&right.kind));

    let nominal_entities = nominal
        .iter()
        .map(|command| command.kind.as_str())
        .collect::<BTreeSet<_>>();
    let transient_entities = marker_subjects(
        branch,
        &site.operator,
        "dialog.concept/transient",
        "db:transient",
    )
    .await?;
    let mut legacy = Vec::new();
    for entity in transient_entities {
        if nominal_entities.contains(entity.to_string().as_str()) {
            continue;
        }
        let concept = Concept::by_entity(entity.clone())
            .resolve(&source, &site.operator)
            .await?
            .ok_or_else(|| anyhow!("transient marker {entity} has no concept descriptor"))?;
        legacy.push(LegacyCommand {
            name: names.get(&entity).cloned(),
            entity: entity.to_string(),
            source: serde_json::to_string(&concept.descriptor)
                .context("could not serialize legacy command descriptor")?,
        });
    }
    legacy.sort_by(|left, right| left.entity.cmp(&right.entity));

    let effects = command_effects(branch, &site.operator).await?;
    let event_bindings = event_bindings(branch, &site.operator).await?;
    Ok(Inventory {
        revision: branch
            .revision()
            .map(|revision| revision.tree.to_string())
            .unwrap_or_else(|| "unborn".to_owned()),
        nominal,
        legacy,
        effects,
        event_bindings,
    })
}

async fn marker_subjects<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
    relation: &str,
    marker: &str,
) -> Result<Vec<Entity>>
where
    Env: tonk_schema::concept::QueryEnv,
{
    let the: attribute::The = relation.parse()?;
    let marker: Entity = marker.parse()?;
    let claims: Vec<dialog_query::Claim> = branch
        .query()
        .select(AttributeQuery::from(
            Term::from(the)
                .of(Term::<Entity>::var("subject"))
                .is(Term::from(marker)),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| anyhow!("{relation} inventory query failed: {error:?}"))?;
    let mut entities = claims.into_iter().map(|claim| claim.of).collect::<Vec<_>>();
    entities.sort();
    entities.dedup();
    Ok(entities)
}

async fn names_by_referent<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
) -> Result<BTreeMap<Entity, String>>
where
    Env: tonk_schema::concept::QueryEnv,
{
    let the: attribute::The = "dialog.name/referent".parse()?;
    let claims: Vec<dialog_query::Claim> = branch
        .query()
        .select(AttributeQuery::new(
            Term::from(the),
            Term::<Entity>::var("name"),
            Term::<dialog_query::Any>::var("referent"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| anyhow!("name inventory query failed: {error:?}"))?;
    Ok(claims
        .into_iter()
        .filter_map(|claim| {
            let name = claim.of.to_string().strip_prefix("id:")?.to_owned();
            let Value::Entity(referent) = claim.is else {
                return None;
            };
            Some((referent, name))
        })
        .collect())
}

async fn command_effects<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
) -> Result<Vec<CommandEffect>>
where
    Env: tonk_schema::concept::QueryEnv,
{
    let the: attribute::The = "dialog.effect/command".parse()?;
    let claims: Vec<dialog_query::Claim> = branch
        .query()
        .select(AttributeQuery::new(
            Term::from(the),
            Term::<Entity>::var("effect"),
            Term::<dialog_query::Any>::var("command"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| anyhow!("command effect inventory query failed: {error:?}"))?;
    let source_the: attribute::The = "dialog.effect/source".parse()?;
    let mut out = Vec::new();
    for claim in claims {
        let Value::Entity(command) = claim.is else {
            continue;
        };
        let sources: Vec<dialog_query::Claim> = branch
            .query()
            .select(AttributeQuery::from(
                Term::from(source_the.clone())
                    .of(Term::from(claim.of.clone()))
                    .is(Term::<String>::var("source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|error| anyhow!("effect source inventory query failed: {error:?}"))?;
        for source in sources {
            if let Value::String(source) = source.is {
                out.push(CommandEffect {
                    command: command.to_string(),
                    effect: claim.of.to_string(),
                    source,
                });
            }
        }
    }
    out.sort_by(|left, right| (&left.command, &left.effect).cmp(&(&right.command, &right.effect)));
    out.dedup_by(|left, right| left.command == right.command && left.effect == right.effect);
    Ok(out)
}

async fn event_bindings<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
) -> Result<Vec<EventBinding>>
where
    Env: tonk_schema::concept::QueryEnv,
{
    let id_the: attribute::The = "dialog.attribute/id".parse()?;
    let definitions: Vec<dialog_query::Claim> = branch
        .query()
        .select(AttributeQuery::from(
            Term::from(id_the)
                .of(Term::<Entity>::var("attribute"))
                .is(Term::<String>::var("id")),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| anyhow!("attribute inventory query failed: {error:?}"))?;
    let mut out = Vec::new();
    for definition in definitions {
        let Value::String(identifier) = definition.is else {
            continue;
        };
        let Ok(the) = identifier.parse::<attribute::The>() else {
            continue;
        };
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(AttributeQuery::from(
                Term::from(the)
                    .of(Term::<Entity>::var("of"))
                    .is(Term::<String>::var("source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|error| anyhow!("event binding lookup for {identifier} failed: {error:?}"))?;
        for claim in claims {
            let Value::String(value) = claim.is else {
                continue;
            };
            let references = extract_event_references(&value);
            if !references.is_empty() {
                out.push(EventBinding {
                    entity: claim.of.to_string(),
                    attribute: identifier.clone(),
                    references,
                });
            }
        }
    }
    out.sort_by(|left, right| {
        (&left.entity, &left.attribute).cmp(&(&right.entity, &right.attribute))
    });
    out.dedup_by(|left, right| left.entity == right.entity && left.attribute == right.attribute);
    Ok(out)
}

fn extract_event_references(source: &str) -> Vec<String> {
    let mut references = BTreeSet::new();
    for marker in ["data-on", "onclick=", "onsubmit=", "onchange=", "oninput="] {
        let mut rest = source;
        while let Some(offset) = rest.find(marker) {
            rest = &rest[offset + marker.len()..];
            let rest = rest.trim_start_matches(|character: char| {
                character.is_ascii_alphanumeric() || character == '-'
            });
            let rest = rest.trim_start_matches(['=', ' ', '\'', '"', '{']);
            let reference = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '/' | ':' | '.')
                })
                .collect::<String>();
            if !reference.is_empty() {
                references.insert(reference);
            }
            if rest.is_empty() {
                break;
            }
        }
    }
    references.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::extract_event_references;

    #[test]
    fn event_reference_extraction_is_stable_and_deduplicated() {
        assert_eq!(
            extract_event_references(
                r#"<form onsubmit="todo/add"><button data-onclick="todo/add-form">"#
            ),
            vec!["todo/add", "todo/add-form"]
        );
    }
}
