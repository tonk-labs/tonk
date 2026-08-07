//! Stored nominal command definitions and branch-local resolution.

use crate::concept::{ConceptLookupError, QueryEnv, lookup_named_entity};
use crate::prelude::EntityExt;
use crate::query_source::Source;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Statement, Update, Value};
use dialog_query::{Output as _, Term};
use thiserror::Error;
use tonk_core::command::CommandSchema;

/// A stable command kind paired with its current content-derived schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDefinition {
    kind: Entity,
    schema_entity: Entity,
    schema: CommandSchema,
    source: String,
}

impl CommandDefinition {
    /// Build a command definition for assertion.
    pub fn asserting(kind: Entity, schema: CommandSchema) -> Self {
        let schema_entity = Entity::of(&schema);
        let source = encode_source(&schema);
        Self {
            kind,
            schema_entity,
            schema,
            source,
        }
    }

    /// Resolve a command by its stable kind entity.
    pub fn by_entity(kind: Entity) -> CommandByEntity {
        CommandByEntity { kind }
    }

    /// Resolve a command by an anchor name or published alias.
    pub fn by_name(name: impl Into<String>) -> CommandByName {
        CommandByName { name: name.into() }
    }

    /// Resolve the exact stored definition needed for retraction.
    pub fn retracting(kind: Entity) -> CommandByEntity {
        Self::by_entity(kind)
    }

    /// Stable nominal command kind.
    pub fn kind(&self) -> &Entity {
        &self.kind
    }

    /// Content-derived entity of the current schema.
    pub fn schema_entity(&self) -> &Entity {
        &self.schema_entity
    }

    /// Current typed schema.
    pub fn schema(&self) -> &CommandSchema {
        &self.schema
    }

    /// Exact canonical DAG-JSON bytes stored as text.
    pub fn source(&self) -> &str {
        &self.source
    }
}

impl Statement for CommandDefinition {
    fn assert(self, update: &mut impl Update) {
        update.associate_unique(
            meta_attr("dialog.meta", "command"),
            self.kind.clone(),
            Value::Entity(command_marker_entity()),
        );
        update.associate_unique(
            meta_attr("dialog.command", "schema"),
            self.kind,
            Value::Entity(self.schema_entity.clone()),
        );
        update.associate_unique(
            meta_attr("dialog.command", "source"),
            self.schema_entity,
            Value::String(self.source),
        );
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(
            meta_attr("dialog.meta", "command"),
            self.kind.clone(),
            Value::Entity(command_marker_entity()),
        );
        update.dissociate(
            meta_attr("dialog.command", "schema"),
            self.kind,
            Value::Entity(self.schema_entity.clone()),
        );
        update.dissociate(
            meta_attr("dialog.command", "source"),
            self.schema_entity,
            Value::String(self.source),
        );
    }
}

/// A command reference used by analyzer and runtime resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandReference {
    /// Published anchor or alias without the `id:` prefix.
    Name(String),
    /// Stable command kind entity.
    Entity(Entity),
}

impl CommandReference {
    /// Resolve this reference against branch data.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<CommandDefinition>, CommandResolveError> {
        match self {
            Self::Name(name) => CommandDefinition::by_name(name).resolve(source, env).await,
            Self::Entity(entity) => {
                CommandDefinition::by_entity(entity)
                    .resolve(source, env)
                    .await
            }
        }
    }
}

/// Builder for resolving a command by stable kind.
#[derive(Debug, Clone)]
pub struct CommandByEntity {
    kind: Entity,
}

impl CommandByEntity {
    /// Resolve the current schema and retain its exact stored source.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<CommandDefinition>, CommandResolveError> {
        let schema_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(command_attr("schema"))
                    .of(Term::<Entity>::from(self.kind.clone()))
                    .is(Term::<Entity>::var("__command_schema")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|error| {
                CommandResolveError::Query(format!("schema lookup failed: {error:?}"))
            })?;
        let Some(schema_claim) = schema_claims.into_iter().next() else {
            return Ok(None);
        };
        let Value::Entity(schema_entity) = schema_claim.is else {
            return Err(CommandResolveError::Storage(
                "dialog.command/schema claim was not an entity".into(),
            ));
        };

        let source_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(command_attr("source"))
                    .of(Term::<Entity>::from(schema_entity.clone()))
                    .is(Term::<String>::var("__command_source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|error| {
                CommandResolveError::Query(format!("source lookup failed: {error:?}"))
            })?;
        let Some(source_claim) = source_claims.into_iter().next() else {
            return Err(CommandResolveError::Storage(
                "missing dialog.command/source claim".into(),
            ));
        };
        let Value::String(source_string) = source_claim.is else {
            return Err(CommandResolveError::Storage(
                "dialog.command/source claim was not text".into(),
            ));
        };
        let schema: CommandSchema = serde_ipld_dagjson::from_slice(source_string.as_bytes())
            .map_err(|error| {
                CommandResolveError::Storage(format!("invalid command schema source: {error}"))
            })?;
        if Entity::of(&schema) != schema_entity {
            return Err(CommandResolveError::Storage(
                "command schema source does not match its content-derived entity".into(),
            ));
        }
        Ok(Some(CommandDefinition {
            kind: self.kind,
            schema_entity,
            schema,
            source: source_string,
        }))
    }
}

/// Builder for resolving an anchored command or published alias.
#[derive(Debug, Clone)]
pub struct CommandByName {
    name: String,
}

impl CommandByName {
    /// Resolve `id:<name>` directly first, then follow a published alias.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<CommandDefinition>, CommandResolveError> {
        if let Ok(direct) = format!("id:{}", self.name).parse::<Entity>()
            && let Some(command) = CommandDefinition::by_entity(direct)
                .resolve(source, env)
                .await?
        {
            return Ok(Some(command));
        }
        let Some(target) = lookup_named_entity(&self.name, source, env)
            .await
            .map_err(CommandResolveError::Name)?
        else {
            return Ok(None);
        };
        CommandDefinition::by_entity(target)
            .resolve(source, env)
            .await
    }
}

/// Failures while resolving persisted command definitions.
#[derive(Debug, Error)]
pub enum CommandResolveError {
    /// Underlying branch query failed.
    #[error("command resolve query failed: {0}")]
    Query(String),
    /// Stored facts were incomplete or inconsistent.
    #[error("command storage shape: {0}")]
    Storage(String),
    /// Published-name lookup failed.
    #[error(transparent)]
    Name(#[from] ConceptLookupError),
}

fn encode_source(schema: &CommandSchema) -> String {
    String::from_utf8(
        serde_ipld_dagjson::to_vec(schema).expect("CommandSchema always serializes to DAG-JSON"),
    )
    .expect("DAG-JSON is UTF-8")
}

fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("command storage attribute is valid")
}

fn command_attr(name: &str) -> dialog_query::attribute::The {
    format!("dialog.command/{name}")
        .parse()
        .expect("command query attribute is valid")
}

fn command_marker_entity() -> Entity {
    "db:command"
        .parse()
        .expect("command marker entity is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_source::Source;
    use dialog_artifacts::{Entity, Value};
    use dialog_query::{AttributeDescriptor, Cardinality, Term, Type};
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};
    use tonk_core::command::CommandSchema;

    fn field(the: &str, content_type: Type) -> AttributeDescriptor {
        AttributeDescriptor::new(
            the.parse().unwrap(),
            "",
            Cardinality::One,
            Some(content_type),
        )
    }

    fn schema(optional_note: bool) -> CommandSchema {
        CommandSchema {
            required: [("title".into(), field("xyz.tonk.todo/title", Type::String))]
                .into_iter()
                .collect(),
            optional: optional_note
                .then(|| ("note".into(), field("xyz.tonk.todo/note", Type::String)))
                .into_iter()
                .collect(),
        }
    }

    #[dialog_common::test]
    async fn command_schema_replacement_preserves_kind() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let kind: Entity = "id:todo/add".parse()?;

        let first = CommandDefinition::asserting(kind.clone(), schema(false));
        let first_schema = first.schema_entity().clone();
        branch
            .transaction()
            .assert(first)
            .commit()
            .perform(&operator)
            .await?;

        let second = CommandDefinition::asserting(kind.clone(), schema(true));
        let second_schema = second.schema_entity().clone();
        assert_ne!(first_schema, second_schema);
        branch
            .transaction()
            .assert(second)
            .commit()
            .perform(&operator)
            .await?;

        let resolved = CommandDefinition::by_entity(kind.clone())
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("command resolves");
        assert_eq!(resolved.kind(), &kind);
        assert_eq!(resolved.schema_entity(), &second_schema);
        assert!(resolved.schema().optional.contains_key("note"));

        let links: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(command_attr("schema"))
                    .of(Term::<Entity>::from(kind))
                    .is(Term::<Entity>::var("schema")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].is, Value::Entity(second_schema));
        Ok(())
    }

    #[dialog_common::test]
    async fn command_retraction_uses_exact_stored_source() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let kind: Entity = "id:todo/add".parse()?;
        let definition = CommandDefinition::asserting(kind.clone(), schema(false));
        let source = definition.source().to_owned();
        let schema_entity = definition.schema_entity().clone();
        branch
            .transaction()
            .assert(definition)
            .commit()
            .perform(&operator)
            .await?;

        let retracting = CommandDefinition::retracting(kind.clone())
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("stored command resolves for retraction");
        assert_eq!(retracting.source(), source);
        branch
            .transaction()
            .retract(retracting)
            .commit()
            .perform(&operator)
            .await?;

        assert!(
            CommandDefinition::by_entity(kind)
                .resolve(&Source::from(&branch), &operator)
                .await?
                .is_none()
        );
        let sources: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(command_attr("source"))
                    .of(Term::<Entity>::from(schema_entity))
                    .is(Term::<String>::var("source")),
            ))
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(sources.is_empty());
        Ok(())
    }
}
