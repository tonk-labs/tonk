//! Headless event-projection verification shared with the mounted runtime.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use dialog_artifacts::Value;
use serde::{Deserialize, Serialize};
use tonk_core::command::{CommandBatch, CommandOccurrence, InvocationMetadata};
use tonk_evaluator::effect_query::effects_by_command;
use tonk_schema::claim::{SourceClaim, TransactRequest};
use tonk_schema::command_definition::{CommandDefinition, CommandReference};
use tonk_schema::projection::{
    ControlProperty, EventAction, EventMember, ProjectionDefinition, ProjectionInput,
    ProjectionSource, SourceRead, TargetMember, project,
};
use tonk_schema::query_source::Source;

use crate::site::{BRANCH_NAME, REPO_NAME, TonkSite};

/// One named control in a headless fixture.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FixtureControl {
    /// Control `value` property.
    pub value: Option<serde_yaml::Value>,
    /// Control `checked` property.
    pub checked: Option<serde_yaml::Value>,
}

/// YAML fixture accepted by `tonk project`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FixtureInput {
    /// Exact control names to value/checked properties.
    #[serde(default)]
    pub controls: BTreeMap<String, FixtureControl>,
    /// Exact `data-*` suffixes.
    #[serde(default)]
    pub data: BTreeMap<String, serde_yaml::Value>,
    /// Whitelisted event member names.
    #[serde(default)]
    pub event: BTreeMap<String, serde_yaml::Value>,
    /// Exact custom-event detail member names.
    #[serde(default)]
    pub detail: BTreeMap<String, serde_yaml::Value>,
    /// Whitelisted target member names.
    #[serde(default)]
    pub target: BTreeMap<String, serde_yaml::Value>,
}

impl FixtureInput {
    /// Load a fixture YAML document from disk.
    pub fn read(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read projection fixture {}", path.display()))?;
        serde_yaml::from_str(&source)
            .with_context(|| format!("invalid projection fixture {}", path.display()))
    }
}

impl ProjectionInput for FixtureInput {
    fn control(&self, name: &str, property: ControlProperty) -> SourceRead {
        let Some(control) = self.controls.get(name) else {
            return SourceRead::Missing;
        };
        let value = match property {
            ControlProperty::Value => control.value.as_ref(),
            ControlProperty::Checked => control.checked.as_ref(),
        };
        fixture_read(value)
    }

    fn data(&self, name: &str) -> SourceRead {
        fixture_read(self.data.get(name))
    }

    fn event(&self, member: EventMember) -> SourceRead {
        fixture_read(self.event.get(enum_name(member)))
    }

    fn detail(&self, member: &str) -> SourceRead {
        fixture_read(self.detail.get(member))
    }

    fn target(&self, member: TargetMember) -> SourceRead {
        fixture_read(self.target.get(enum_name(member)))
    }
}

/// One successful source trace in CLI-friendly serializable form.
#[derive(Debug, Clone, Serialize)]
pub struct Trace {
    /// Command field.
    pub field: String,
    /// Exact declared projection source.
    pub source: ProjectionSource,
    /// Raw fixture value, absent for omitted optional fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// `tonk project` output.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectReport {
    /// Resolved projection entity.
    pub projection: String,
    /// Resolved stable command kind.
    pub command: String,
    /// Field-by-field source evidence.
    pub trace: Vec<Trace>,
    /// Optional fields omitted because their source was missing.
    pub omitted: Vec<String>,
    /// Planned synchronous actions, never executed headlessly.
    pub actions: Vec<EventAction>,
    /// Exact request the mounted runtime would submit.
    pub request: TransactRequest,
    /// Durable revision after explicit `--transact`, otherwise absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_after: Option<String>,
}

impl ProjectReport {
    /// Replace argument/trace values while preserving field and source names.
    pub fn redact(mut self) -> Self {
        for trace in &mut self.trace {
            if trace.value.is_some() {
                trace.value = Some(Value::String("<redacted>".into()));
            }
        }
        for claim in &mut self.request.claims {
            if let SourceClaim::Invoke(invocation) = claim {
                for value in invocation.arguments.values_mut() {
                    *value = Value::String("<redacted>".into());
                }
            }
        }
        self
    }
}

/// Resolve and evaluate a projection or command reference. Nothing is written
/// unless `transact` is true.
pub async fn run(
    site: &TonkSite,
    reference: &str,
    fixture: &FixtureInput,
    transact: bool,
) -> Result<ProjectReport> {
    let session = site.branch().await?;
    let source = Source::from(session.handle());
    let (projection, command) = resolve(reference, &source, &site.operator).await?;
    let result = project(&projection, command.schema(), fixture)?;
    let invocation = result.invocation.clone();
    let mut revision_after = None;

    if transact {
        let registered = effects_by_command(invocation.command.clone())
            .resolve(session.handle(), &site.operator)
            .await
            .map_err(|error| anyhow!("command consumer lookup failed: {error}"))?;
        if registered.is_empty() {
            bail!("command_unhandled: no declarative rule is registered for this command");
        }
        let validated = command.schema().validate(invocation.clone())?;
        let occurrence = CommandOccurrence::new(
            validated,
            InvocationMetadata::new(
                dialog_artifacts::Entity::new()?,
                format!("invoke:{}", hex::encode(rand::random::<[u8; 16]>())),
            ),
        );
        let report = site
            .reactor
            .repository(REPO_NAME)
            .branch(BRANCH_NAME)
            .transaction()
            .command_batch(CommandBatch::new(vec![occurrence]))
            .commit()
            .perform_report(&site.operator)
            .await?;
        revision_after = Some(report.revision.tree.to_string());
    }

    Ok(ProjectReport {
        projection: projection.this().to_string(),
        command: command.kind().to_string(),
        trace: result
            .trace
            .into_iter()
            .map(|trace| Trace {
                field: trace.field,
                source: trace.source,
                value: trace.value,
            })
            .collect(),
        omitted: result.omitted_optional,
        actions: result.actions,
        request: TransactRequest {
            claims: vec![SourceClaim::Invoke(invocation)],
        },
        revision_after,
    })
}

async fn resolve<Env: tonk_schema::concept::QueryEnv>(
    reference: &str,
    source: &Source<'_>,
    env: &Env,
) -> Result<(ProjectionDefinition, CommandDefinition)> {
    let projection = if reference.contains(':') {
        ProjectionDefinition::by_entity(reference.parse()?)
            .resolve(source, env)
            .await?
    } else {
        ProjectionDefinition::by_name(reference)
            .resolve(source, env)
            .await?
    };
    if let Some(projection) = projection {
        let command = CommandDefinition::by_entity(projection.descriptor().command.clone())
            .resolve(source, env)
            .await?
            .ok_or_else(|| anyhow!("projection references an unknown command"))?;
        return Ok((projection, command));
    }

    let command = if reference.contains(':') {
        CommandReference::Entity(reference.parse()?)
    } else {
        CommandReference::Name(reference.to_owned())
    }
    .resolve(source, env)
    .await?
    .ok_or_else(|| anyhow!("unknown projection or command {reference:?}"))?;
    let projections = ProjectionDefinition::for_command(command.kind().clone())
        .resolve(source, env)
        .await?;
    let projection = match projections.as_slice() {
        [projection] => projection.clone(),
        [] => bail!("command has no projection"),
        projections => projections
            .iter()
            .find(|projection| projection.descriptor().default)
            .cloned()
            .ok_or_else(|| anyhow!("command has multiple projections and no unique default"))?,
    };
    Ok((projection, command))
}

fn fixture_read(value: Option<&serde_yaml::Value>) -> SourceRead {
    let Some(value) = value else {
        return SourceRead::Missing;
    };
    match yaml_value(value) {
        Ok(Some(value)) => SourceRead::Present(value),
        Ok(None) => SourceRead::Missing,
        Err(error) => SourceRead::ReadFailed(error),
    }
}

fn yaml_value(value: &serde_yaml::Value) -> std::result::Result<Option<Value>, String> {
    Ok(match value {
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Bool(value) => Some(Value::Boolean(*value)),
        serde_yaml::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Some(Value::SignedInt(value.into()))
            } else if let Some(value) = value.as_u64() {
                Some(Value::UnsignedInt(value.into()))
            } else if let Some(value) = value.as_f64() {
                Some(Value::Float(value))
            } else {
                return Err("number is outside supported ranges".into());
            }
        }
        serde_yaml::Value::String(value) => Some(Value::String(value.clone())),
        _ => return Err("fixture values must be scalar".into()),
    })
}

fn enum_name<T: Serialize>(value: T) -> &'static str {
    // Both enums have a finite static spelling. Keep the map explicit so the
    // returned string can be borrowed without allocating on every read.
    let encoded = serde_json::to_string(&value).expect("enum serializes");
    match encoded.as_str() {
        "\"type\"" => "type",
        "\"key\"" => "key",
        "\"code\"" => "code",
        "\"repeat\"" => "repeat",
        "\"shiftKey\"" => "shiftKey",
        "\"ctrlKey\"" => "ctrlKey",
        "\"altKey\"" => "altKey",
        "\"metaKey\"" => "metaKey",
        "\"button\"" => "button",
        "\"clientX\"" => "clientX",
        "\"clientY\"" => "clientY",
        "\"timeStamp\"" => "timeStamp",
        "\"value\"" => "value",
        "\"checked\"" => "checked",
        _ => unreachable!("finite projection enum"),
    }
}
