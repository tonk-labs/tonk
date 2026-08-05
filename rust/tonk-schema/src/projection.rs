//! Stored event projections, typed source descriptors, and branch resolution.

use crate::concept::{ConceptLookupError, QueryEnv, lookup_named_entity};
use crate::query_source::Source;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Statement, Update, Value};
use dialog_query::{Output as _, Term};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_core::claim::ValueMap;
use tonk_core::command::{CommandSchema, CommandValidationError, SourceInvocation};

/// Property read from a named form control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ControlProperty {
    /// The control's textual value.
    #[default]
    Value,
    /// The control's checked state.
    Checked,
}

/// Exact named-control lookup and scalar property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSource {
    /// Exact HTML form-control name.
    pub name: String,
    /// Scalar property to read.
    #[serde(default)]
    pub property: ControlProperty,
}

/// Supported scalar members on a browser event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventMember {
    /// Event type.
    Type,
    /// Keyboard key.
    Key,
    /// Keyboard code.
    Code,
    /// Keyboard-repeat flag.
    Repeat,
    /// Shift modifier.
    ShiftKey,
    /// Control modifier.
    CtrlKey,
    /// Alt modifier.
    AltKey,
    /// Meta modifier.
    MetaKey,
    /// Pointer button.
    Button,
    /// Pointer client X coordinate.
    ClientX,
    /// Pointer client Y coordinate.
    ClientY,
    /// Event timestamp.
    TimeStamp,
}

/// Supported scalar members on an event target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetMember {
    /// Target value.
    Value,
    /// Target checked state.
    Checked,
}

/// One typed source for a command argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectionSource {
    /// Named form control.
    Control(ControlSource),
    /// Exact bound-element `data-*` suffix.
    Data(String),
    /// Whitelisted event member.
    Event(EventMember),
    /// One `CustomEvent.detail` member.
    Detail(String),
    /// Whitelisted event-target member.
    Target(TargetMember),
    /// Scalar literal.
    Literal(Value),
}

/// Synchronous action executed after successful projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventAction {
    /// Call `preventDefault()`.
    PreventDefault,
    /// Call `stopPropagation()`.
    StopPropagation,
    /// Call `stopImmediatePropagation()`.
    StopImmediatePropagation,
}

/// Stored projection body independent of its projection entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionDescriptor {
    /// Target nominal command kind.
    pub command: Entity,
    /// Whether this is the command's default projection.
    #[serde(default)]
    pub default: bool,
    /// Command field to typed input source.
    #[serde(default)]
    pub arguments: IndexMap<String, ProjectionSource>,
    /// Ordered synchronous event actions.
    #[serde(default)]
    pub actions: Vec<EventAction>,
}

/// One source read supplied by a browser or headless fixture adapter.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceRead {
    /// The source exists, including false, zero, and empty text values.
    Present(Value),
    /// The source does not exist.
    Missing,
    /// The source existed but could not be read safely.
    ReadFailed(String),
}

/// Source-independent adapter used to project an event into an invocation.
pub trait ProjectionInput {
    /// Read an exact named form control property.
    fn control(&self, name: &str, property: ControlProperty) -> SourceRead;
    /// Read an exact `data-*` suffix from the bound element.
    fn data(&self, name: &str) -> SourceRead;
    /// Read a whitelisted event member.
    fn event(&self, member: EventMember) -> SourceRead;
    /// Read one exact custom-event detail member.
    fn detail(&self, member: &str) -> SourceRead;
    /// Read a whitelisted target member.
    fn target(&self, member: TargetMember) -> SourceRead;
}

/// Successful per-field projection evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionTrace {
    /// Command argument field.
    pub field: String,
    /// Exact declared input source.
    pub source: ProjectionSource,
    /// Raw source value, or `None` when an optional source was omitted.
    pub value: Option<Value>,
}

/// Complete, validated projection result. Actions remain declarative and are
/// never executed by this source-independent layer.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionResult {
    /// Validated wire invocation ready for submission.
    pub invocation: SourceInvocation,
    /// Per-declared-field source evidence.
    pub trace: Vec<ProjectionTrace>,
    /// Optional fields whose sources were absent.
    pub omitted_optional: Vec<String>,
    /// Ordered synchronous actions to execute after successful extraction.
    pub actions: Vec<EventAction>,
}

/// Loud projection failure with enough context for browser and CLI diagnostics.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProjectionError {
    /// A declared source could not be read.
    #[error("projection {projection} for {command} could not read field {field:?}: {message}")]
    ReadFailed {
        /// Projection entity.
        projection: Entity,
        /// Command kind.
        command: Entity,
        /// Argument field.
        field: String,
        /// Exact declared source.
        input: ProjectionSource,
        /// Adapter-provided diagnostic.
        message: String,
    },
    /// A required source was absent.
    #[error("projection {projection} for {command} is missing required field {field:?}")]
    MissingRequired {
        /// Projection entity.
        projection: Entity,
        /// Command kind.
        command: Entity,
        /// Required field.
        field: String,
        /// Exact declared source, when the projection mapped the field.
        input: Option<ProjectionSource>,
    },
    /// Extracted arguments failed command-schema validation/coercion.
    #[error("projection {projection} for {command} produced an invalid invocation: {error}")]
    InvalidInvocation {
        /// Projection entity.
        projection: Entity,
        /// Command kind.
        command: Entity,
        /// Source responsible for the field, when one was declared.
        input: Option<ProjectionSource>,
        /// Authoritative command validation failure.
        error: CommandValidationError,
    },
}

/// A projection entity paired with its exact stored descriptor source.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionDefinition {
    this: Entity,
    descriptor: ProjectionDescriptor,
    source: String,
}

impl ProjectionDefinition {
    /// Build a projection definition for assertion at a stable entity.
    pub fn asserting(this: Entity, descriptor: ProjectionDescriptor) -> Self {
        let source = encode_source(&descriptor);
        Self {
            this,
            descriptor,
            source,
        }
    }

    /// Resolve a projection by entity.
    pub fn by_entity(this: Entity) -> ProjectionByEntity {
        ProjectionByEntity { this }
    }

    /// Resolve a projection by anchor or published alias.
    pub fn by_name(name: impl Into<String>) -> ProjectionByName {
        ProjectionByName { name: name.into() }
    }

    /// Resolve every projection for a command and validate default uniqueness.
    pub fn for_command(command: Entity) -> ProjectionsForCommand {
        ProjectionsForCommand { command }
    }

    /// Resolve the exact stored definition needed for retraction.
    pub fn retracting(this: Entity) -> ProjectionByEntity {
        Self::by_entity(this)
    }

    /// Projection entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }

    /// Parsed projection descriptor.
    pub fn descriptor(&self) -> &ProjectionDescriptor {
        &self.descriptor
    }

    /// Exact canonical DAG-JSON stored on the branch.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Evaluate one stored projection against a browser- or fixture-backed input.
/// Extraction is all-or-nothing: errors return no actions and no invocation.
pub fn project(
    projection: &ProjectionDefinition,
    schema: &CommandSchema,
    input: &impl ProjectionInput,
) -> Result<ProjectionResult, ProjectionError> {
    let descriptor = projection.descriptor();
    let mut arguments = ValueMap::new();
    let mut trace = Vec::with_capacity(descriptor.arguments.len());
    let mut omitted_optional = Vec::new();

    for (field, source) in &descriptor.arguments {
        let read = read_source(source, input);
        match read {
            SourceRead::Present(value) => {
                trace.push(ProjectionTrace {
                    field: field.clone(),
                    source: source.clone(),
                    value: Some(value.clone()),
                });
                arguments.insert(field.clone(), value);
            }
            SourceRead::Missing if schema.optional.contains_key(field) => {
                trace.push(ProjectionTrace {
                    field: field.clone(),
                    source: source.clone(),
                    value: None,
                });
                omitted_optional.push(field.clone());
            }
            SourceRead::Missing if schema.required.contains_key(field) => {
                return Err(ProjectionError::MissingRequired {
                    projection: projection.this().clone(),
                    command: descriptor.command.clone(),
                    field: field.clone(),
                    input: Some(source.clone()),
                });
            }
            SourceRead::Missing => {
                return Err(invalid_invocation(
                    projection,
                    Some(source.clone()),
                    CommandValidationError::UnknownArgument {
                        field: field.clone(),
                    },
                ));
            }
            SourceRead::ReadFailed(message) => {
                return Err(ProjectionError::ReadFailed {
                    projection: projection.this().clone(),
                    command: descriptor.command.clone(),
                    field: field.clone(),
                    input: source.clone(),
                    message,
                });
            }
        }
    }

    let invocation = schema
        .validate(SourceInvocation {
            command: descriptor.command.clone(),
            arguments,
        })
        .map_err(|error| {
            let field = validation_field(&error);
            let input = field
                .and_then(|field| descriptor.arguments.get(field))
                .cloned();
            match error {
                CommandValidationError::MissingRequiredArgument { field } => {
                    ProjectionError::MissingRequired {
                        projection: projection.this().clone(),
                        command: descriptor.command.clone(),
                        field,
                        input,
                    }
                }
                error => invalid_invocation(projection, input, error),
            }
        })?;
    let (command, arguments) = invocation.into_parts();

    Ok(ProjectionResult {
        invocation: SourceInvocation { command, arguments },
        trace,
        omitted_optional,
        actions: descriptor.actions.clone(),
    })
}

fn read_source(source: &ProjectionSource, input: &impl ProjectionInput) -> SourceRead {
    match source {
        ProjectionSource::Control(control) => input.control(&control.name, control.property),
        ProjectionSource::Data(name) => input.data(name),
        ProjectionSource::Event(member) => input.event(*member),
        ProjectionSource::Detail(member) => input.detail(member),
        ProjectionSource::Target(member) => input.target(*member),
        ProjectionSource::Literal(value) => SourceRead::Present(value.clone()),
    }
}

fn invalid_invocation(
    projection: &ProjectionDefinition,
    input: Option<ProjectionSource>,
    error: CommandValidationError,
) -> ProjectionError {
    ProjectionError::InvalidInvocation {
        projection: projection.this().clone(),
        command: projection.descriptor().command.clone(),
        input,
        error,
    }
}

fn validation_field(error: &CommandValidationError) -> Option<&str> {
    match error {
        CommandValidationError::UnknownArgument { field }
        | CommandValidationError::MissingRequiredArgument { field }
        | CommandValidationError::ReservedArgument { field }
        | CommandValidationError::TypeMismatch { field, .. } => Some(field),
    }
}

impl Statement for ProjectionDefinition {
    fn assert(self, update: &mut impl Update) {
        update.associate_unique(
            meta_attr("dialog.meta", "projection"),
            self.this.clone(),
            Value::Entity(projection_marker_entity()),
        );
        update.associate_unique(
            meta_attr("dialog.projection", "command"),
            self.this.clone(),
            Value::Entity(self.descriptor.command.clone()),
        );
        update.associate_unique(
            meta_attr("dialog.projection", "default"),
            self.this.clone(),
            Value::Boolean(self.descriptor.default),
        );
        update.associate_unique(
            meta_attr("dialog.projection", "source"),
            self.this,
            Value::String(self.source),
        );
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(
            meta_attr("dialog.meta", "projection"),
            self.this.clone(),
            Value::Entity(projection_marker_entity()),
        );
        update.dissociate(
            meta_attr("dialog.projection", "command"),
            self.this.clone(),
            Value::Entity(self.descriptor.command.clone()),
        );
        update.dissociate(
            meta_attr("dialog.projection", "default"),
            self.this.clone(),
            Value::Boolean(self.descriptor.default),
        );
        update.dissociate(
            meta_attr("dialog.projection", "source"),
            self.this,
            Value::String(self.source),
        );
    }
}

/// Builder for resolving a projection by entity.
#[derive(Debug, Clone)]
pub struct ProjectionByEntity {
    this: Entity,
}

impl ProjectionByEntity {
    /// Resolve and validate indexes against the exact stored source.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<ProjectionDefinition>, ProjectionResolveError> {
        let sources = query_string_claims(
            source,
            env,
            projection_attr("source"),
            self.this.clone(),
            "__projection_source",
        )
        .await?;
        let Some(source_claim) = sources.into_iter().next() else {
            return Ok(None);
        };
        let Value::String(source_string) = source_claim.is else {
            return Err(ProjectionResolveError::Storage(
                "dialog.projection/source claim was not text".into(),
            ));
        };
        let descriptor: ProjectionDescriptor =
            serde_ipld_dagjson::from_slice(source_string.as_bytes()).map_err(|error| {
                ProjectionResolveError::Storage(format!(
                    "invalid projection descriptor source: {error}"
                ))
            })?;

        let command_claims = query_entity_claims(
            source,
            env,
            projection_attr("command"),
            self.this.clone(),
            "__projection_command",
        )
        .await?;
        let Some(command_claim) = command_claims.into_iter().next() else {
            return Err(ProjectionResolveError::Storage(
                "missing dialog.projection/command claim".into(),
            ));
        };
        if command_claim.is != Value::Entity(descriptor.command.clone()) {
            return Err(ProjectionResolveError::Storage(
                "projection command index disagrees with source".into(),
            ));
        }

        let default_claims = query_bool_claims(
            source,
            env,
            projection_attr("default"),
            self.this.clone(),
            "__projection_default",
        )
        .await?;
        let Some(default_claim) = default_claims.into_iter().next() else {
            return Err(ProjectionResolveError::Storage(
                "missing dialog.projection/default claim".into(),
            ));
        };
        if default_claim.is != Value::Boolean(descriptor.default) {
            return Err(ProjectionResolveError::Storage(
                "projection default index disagrees with source".into(),
            ));
        }

        Ok(Some(ProjectionDefinition {
            this: self.this,
            descriptor,
            source: source_string,
        }))
    }
}

/// Builder for resolving a projection by anchor or alias.
#[derive(Debug, Clone)]
pub struct ProjectionByName {
    name: String,
}

impl ProjectionByName {
    /// Resolve `id:<name>` directly first, then follow a published alias.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<ProjectionDefinition>, ProjectionResolveError> {
        if let Ok(direct) = format!("id:{}", self.name).parse::<Entity>()
            && let Some(projection) = ProjectionDefinition::by_entity(direct)
                .resolve(source, env)
                .await?
        {
            return Ok(Some(projection));
        }
        let Some(target) = lookup_named_entity(&self.name, source, env)
            .await
            .map_err(ProjectionResolveError::Name)?
        else {
            return Ok(None);
        };
        ProjectionDefinition::by_entity(target)
            .resolve(source, env)
            .await
    }
}

/// Builder for resolving every projection indexed by one command.
#[derive(Debug, Clone)]
pub struct ProjectionsForCommand {
    command: Entity,
}

impl ProjectionsForCommand {
    /// Resolve projections and reject multiple defaults.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Vec<ProjectionDefinition>, ProjectionResolveError> {
        let claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(projection_attr("command"))
                    .of(Term::<Entity>::var("__projection"))
                    .is(Term::<Entity>::from(self.command.clone())),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|error| {
                ProjectionResolveError::Query(format!(
                    "projections-for-command lookup failed: {error:?}"
                ))
            })?;
        let mut projections = Vec::with_capacity(claims.len());
        for claim in claims {
            if let Some(projection) = ProjectionDefinition::by_entity(claim.of)
                .resolve(source, env)
                .await?
            {
                projections.push(projection);
            }
        }
        let defaults = projections
            .iter()
            .filter(|projection| projection.descriptor.default)
            .map(|projection| projection.this.clone())
            .collect::<Vec<_>>();
        if defaults.len() > 1 {
            return Err(ProjectionResolveError::AmbiguousDefault {
                command: self.command,
                projections: defaults,
            });
        }
        Ok(projections)
    }
}

/// Failures while resolving projections.
#[derive(Debug, Error)]
pub enum ProjectionResolveError {
    /// Underlying branch query failed.
    #[error("projection resolve query failed: {0}")]
    Query(String),
    /// Stored facts were incomplete or inconsistent.
    #[error("projection storage shape: {0}")]
    Storage(String),
    /// Published-name lookup failed.
    #[error(transparent)]
    Name(#[from] ConceptLookupError),
    /// More than one projection is marked default for a command.
    #[error("command {command} has multiple default projections")]
    AmbiguousDefault {
        /// Command whose defaults conflict.
        command: Entity,
        /// Conflicting projection entities.
        projections: Vec<Entity>,
    },
}

async fn query_string_claims<Env: QueryEnv>(
    source: &Source<'_>,
    env: &Env,
    attribute: dialog_query::attribute::The,
    entity: Entity,
    variable: &str,
) -> Result<Vec<dialog_query::Claim>, ProjectionResolveError> {
    source
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(attribute)
                .of(Term::<Entity>::from(entity))
                .is(Term::<String>::var(variable)),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            ProjectionResolveError::Query(format!("attribute lookup failed: {error:?}"))
        })
}

async fn query_entity_claims<Env: QueryEnv>(
    source: &Source<'_>,
    env: &Env,
    attribute: dialog_query::attribute::The,
    entity: Entity,
    variable: &str,
) -> Result<Vec<dialog_query::Claim>, ProjectionResolveError> {
    source
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(attribute)
                .of(Term::<Entity>::from(entity))
                .is(Term::<Entity>::var(variable)),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            ProjectionResolveError::Query(format!("attribute lookup failed: {error:?}"))
        })
}

async fn query_bool_claims<Env: QueryEnv>(
    source: &Source<'_>,
    env: &Env,
    attribute: dialog_query::attribute::The,
    entity: Entity,
    variable: &str,
) -> Result<Vec<dialog_query::Claim>, ProjectionResolveError> {
    source
        .select(dialog_query::AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(attribute)
                .of(Term::<Entity>::from(entity))
                .is(Term::<bool>::var(variable)),
        ))
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| {
            ProjectionResolveError::Query(format!("attribute lookup failed: {error:?}"))
        })
}

fn encode_source(descriptor: &ProjectionDescriptor) -> String {
    String::from_utf8(
        serde_ipld_dagjson::to_vec(descriptor)
            .expect("ProjectionDescriptor always serializes to DAG-JSON"),
    )
    .expect("DAG-JSON is UTF-8")
}

fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("projection storage attribute is valid")
}

fn projection_attr(name: &str) -> dialog_query::attribute::The {
    format!("dialog.projection/{name}")
        .parse()
        .expect("projection query attribute is valid")
}

fn projection_marker_entity() -> Entity {
    "db:projection"
        .parse()
        .expect("projection marker entity is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_source::Source;
    use dialog_artifacts::{Entity, Value};
    use dialog_query::{AttributeDescriptor, Cardinality, Type};
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};
    use std::collections::HashMap;

    #[derive(Default)]
    struct FixtureInput {
        controls: HashMap<(String, ControlProperty), SourceRead>,
        data: HashMap<String, SourceRead>,
        event: HashMap<EventMember, SourceRead>,
        detail: HashMap<String, SourceRead>,
        target: HashMap<TargetMember, SourceRead>,
    }

    impl ProjectionInput for FixtureInput {
        fn control(&self, name: &str, property: ControlProperty) -> SourceRead {
            self.controls
                .get(&(name.to_string(), property))
                .cloned()
                .unwrap_or(SourceRead::Missing)
        }

        fn data(&self, name: &str) -> SourceRead {
            self.data.get(name).cloned().unwrap_or(SourceRead::Missing)
        }

        fn event(&self, member: EventMember) -> SourceRead {
            self.event
                .get(&member)
                .cloned()
                .unwrap_or(SourceRead::Missing)
        }

        fn detail(&self, member: &str) -> SourceRead {
            self.detail
                .get(member)
                .cloned()
                .unwrap_or(SourceRead::Missing)
        }

        fn target(&self, member: TargetMember) -> SourceRead {
            self.target
                .get(&member)
                .cloned()
                .unwrap_or(SourceRead::Missing)
        }
    }

    fn field(name: &str, content_type: Type) -> AttributeDescriptor {
        AttributeDescriptor::new(
            format!("xyz.tonk.project/{}", name.replace('_', "-"))
                .parse()
                .unwrap(),
            "",
            Cardinality::One,
            Some(content_type),
        )
    }

    fn schema(required: &[(&str, Type)], optional: &[(&str, Type)]) -> CommandSchema {
        CommandSchema {
            required: required
                .iter()
                .map(|(name, ty)| ((*name).to_string(), field(name, *ty)))
                .collect(),
            optional: optional
                .iter()
                .map(|(name, ty)| ((*name).to_string(), field(name, *ty)))
                .collect(),
        }
    }

    fn projected_definition(descriptor: ProjectionDescriptor) -> ProjectionDefinition {
        ProjectionDefinition::asserting("id:test/projection".parse().unwrap(), descriptor)
    }

    fn descriptor(command: Entity, default: bool) -> ProjectionDescriptor {
        ProjectionDescriptor {
            command,
            default,
            arguments: [
                (
                    "control_value".into(),
                    ProjectionSource::Control(ControlSource {
                        name: "note-body".into(),
                        property: ControlProperty::Value,
                    }),
                ),
                (
                    "control_checked".into(),
                    ProjectionSource::Control(ControlSource {
                        name: "done".into(),
                        property: ControlProperty::Checked,
                    }),
                ),
                ("data".into(), ProjectionSource::Data("note-id".into())),
                (
                    "event".into(),
                    ProjectionSource::Event(EventMember::TimeStamp),
                ),
                ("detail".into(), ProjectionSource::Detail("sheet".into())),
                (
                    "target".into(),
                    ProjectionSource::Target(TargetMember::Value),
                ),
                (
                    "literal".into(),
                    ProjectionSource::Literal(Value::String("next".into())),
                ),
            ]
            .into_iter()
            .collect(),
            actions: vec![
                EventAction::PreventDefault,
                EventAction::StopPropagation,
                EventAction::StopImmediatePropagation,
            ],
        }
    }

    #[dialog_common::test]
    fn projection_evaluator_reads_every_source_and_reuses_command_coercion() {
        let command: Entity = "id:todo/add".parse().unwrap();
        let projection = projected_definition(descriptor(command.clone(), true));
        let schema = schema(
            &[
                ("control_value", Type::String),
                ("control_checked", Type::Boolean),
                ("data", Type::Entity),
                ("event", Type::Float),
                ("detail", Type::SignedInt),
                ("target", Type::String),
                ("literal", Type::String),
            ],
            &[],
        );
        let mut input = FixtureInput::default();
        input.controls.insert(
            ("note-body".into(), ControlProperty::Value),
            SourceRead::Present(Value::String(String::new())),
        );
        input.controls.insert(
            ("done".into(), ControlProperty::Checked),
            SourceRead::Present(Value::Boolean(false)),
        );
        input.data.insert(
            "note-id".into(),
            SourceRead::Present(Value::String("did:key:zNote".into())),
        );
        input.event.insert(
            EventMember::TimeStamp,
            SourceRead::Present(Value::Float(0.0)),
        );
        input
            .detail
            .insert("sheet".into(), SourceRead::Present(Value::Float(0.0)));
        input.target.insert(
            TargetMember::Value,
            SourceRead::Present(Value::String("target".into())),
        );

        let result = project(&projection, &schema, &input).unwrap();
        assert_eq!(result.invocation.command, command);
        assert_eq!(
            result.invocation.arguments.get("control_value"),
            Some(&Value::String(String::new())),
            "present blank text must not collapse into missing"
        );
        assert_eq!(
            result.invocation.arguments.get("control_checked"),
            Some(&Value::Boolean(false))
        );
        assert!(matches!(
            result.invocation.arguments.get("data"),
            Some(Value::Entity(_))
        ));
        assert_eq!(
            result.invocation.arguments.get("detail"),
            Some(&Value::SignedInt(0))
        );
        assert_eq!(result.trace.len(), 7);
        assert_eq!(result.actions, projection.descriptor().actions);
    }

    #[dialog_common::test]
    fn projection_evaluator_uses_exact_hyphenated_names_and_omits_optional_missing() {
        let command: Entity = "id:todo/add".parse().unwrap();
        let projection = projected_definition(ProjectionDescriptor {
            command: command.clone(),
            default: true,
            arguments: [
                (
                    "title".into(),
                    ProjectionSource::Control(ControlSource {
                        name: "note-body".into(),
                        property: ControlProperty::Value,
                    }),
                ),
                ("note".into(), ProjectionSource::Data("data-note-id".into())),
            ]
            .into_iter()
            .collect(),
            actions: vec![EventAction::PreventDefault],
        });
        let schema = schema(&[("title", Type::String)], &[("note", Type::String)]);
        let mut input = FixtureInput::default();
        input.controls.insert(
            ("note-body".into(), ControlProperty::Value),
            SourceRead::Present(Value::String("hello".into())),
        );

        let result = project(&projection, &schema, &input).unwrap();
        assert_eq!(result.omitted_optional, vec!["note"]);
        assert!(!result.invocation.arguments.contains_key("note"));
        assert_eq!(result.trace[1].value, None);
    }

    #[dialog_common::test]
    fn projection_evaluator_rejects_missing_required_without_returning_actions() {
        let projection = projected_definition(ProjectionDescriptor {
            command: "id:todo/add".parse().unwrap(),
            default: true,
            arguments: [(
                "title".into(),
                ProjectionSource::Control(ControlSource {
                    name: "title".into(),
                    property: ControlProperty::Value,
                }),
            )]
            .into_iter()
            .collect(),
            actions: vec![EventAction::PreventDefault],
        });

        assert!(matches!(
            project(
                &projection,
                &schema(&[("title", Type::String)], &[]),
                &FixtureInput::default()
            ),
            Err(ProjectionError::MissingRequired { field, .. }) if field == "title"
        ));
    }

    #[dialog_common::test]
    fn projection_evaluator_distinguishes_read_failure_and_type_failure() {
        let projection = projected_definition(ProjectionDescriptor {
            command: "id:todo/add".parse().unwrap(),
            default: false,
            arguments: [(
                "count".into(),
                ProjectionSource::Target(TargetMember::Value),
            )]
            .into_iter()
            .collect(),
            actions: Vec::new(),
        });
        let schema = schema(&[("count", Type::SignedInt)], &[]);
        let mut failed = FixtureInput::default();
        failed.target.insert(
            TargetMember::Value,
            SourceRead::ReadFailed("detached target".into()),
        );
        assert!(matches!(
            project(&projection, &schema, &failed),
            Err(ProjectionError::ReadFailed { .. })
        ));

        let mut wrong_type = FixtureInput::default();
        wrong_type.target.insert(
            TargetMember::Value,
            SourceRead::Present(Value::String("not an integer".into())),
        );
        assert!(matches!(
            project(&projection, &schema, &wrong_type),
            Err(ProjectionError::InvalidInvocation {
                error: CommandValidationError::TypeMismatch { .. },
                ..
            })
        ));
    }

    #[dialog_common::test]
    fn projection_evaluator_preserves_zero_and_action_order() {
        let actions = vec![
            EventAction::StopPropagation,
            EventAction::PreventDefault,
            EventAction::StopImmediatePropagation,
        ];
        let projection = projected_definition(ProjectionDescriptor {
            command: "id:counter/set".parse().unwrap(),
            default: true,
            arguments: [("count".into(), ProjectionSource::Literal(Value::Float(0.0)))]
                .into_iter()
                .collect(),
            actions: actions.clone(),
        });

        let result = project(
            &projection,
            &schema(&[("count", Type::SignedInt)], &[]),
            &FixtureInput::default(),
        )
        .unwrap();
        assert_eq!(
            result.invocation.arguments.get("count"),
            Some(&Value::SignedInt(0))
        );
        assert_eq!(result.actions, actions);
    }

    #[dialog_common::test]
    async fn projection_round_trips_every_source_and_action() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command: Entity = "id:todo/add".parse()?;
        let projection: Entity = "id:todo/add-form".parse()?;
        let definition =
            ProjectionDefinition::asserting(projection.clone(), descriptor(command, true));
        let exact_source = definition.source().to_owned();
        branch
            .transaction()
            .assert(definition)
            .commit()
            .perform(&operator)
            .await?;

        let resolved = ProjectionDefinition::by_entity(projection.clone())
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("projection resolves");
        assert_eq!(resolved.this(), &projection);
        assert_eq!(resolved.source(), exact_source);
        assert_eq!(
            resolved.descriptor(),
            &descriptor("id:todo/add".parse()?, true)
        );

        let retracting = ProjectionDefinition::retracting(projection.clone())
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("projection resolves for retraction");
        branch
            .transaction()
            .retract(retracting)
            .commit()
            .perform(&operator)
            .await?;
        assert!(
            ProjectionDefinition::by_entity(projection)
                .resolve(&Source::from(&branch), &operator)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn projection_default_is_unique_per_command() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;
        let command: Entity = "id:todo/add".parse()?;
        for name in ["id:todo/add-form", "id:todo/add-button"] {
            branch
                .transaction()
                .assert(ProjectionDefinition::asserting(
                    name.parse()?,
                    descriptor(command.clone(), true),
                ))
                .commit()
                .perform(&operator)
                .await?;
        }

        assert!(matches!(
            ProjectionDefinition::for_command(command)
                .resolve(&Source::from(&branch), &operator)
                .await,
            Err(ProjectionResolveError::AmbiguousDefault { .. })
        ));
        Ok(())
    }
}
