//! Wire-neutral nominal command schemas, validated invocations, and
//! transaction-local occurrence encoding.

use crate::claim::{TransactError, ValueMap, coerce_value};
use dialog_artifacts::{Attribute, Changes, Entity, Update, Value, ValueDataType};
use dialog_query::AttributeDescriptor;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Private relation that binds a transaction-local occurrence to its
/// nominal command kind.
pub const COMMAND_KIND_RELATION: &str = "dialog.command/kind";

/// Private relation prefix used to bind one command argument.
pub const COMMAND_ARGUMENT_RELATION_PREFIX: &str = "dialog.command.argument/";

/// The current typed argument contract for one stable command kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommandSchema {
    /// Arguments that every invocation must carry.
    #[serde(default)]
    pub required: IndexMap<String, AttributeDescriptor>,
    /// Arguments an invocation may omit.
    #[serde(default)]
    pub optional: IndexMap<String, AttributeDescriptor>,
}

impl CommandSchema {
    /// Validate and losslessly coerce one source invocation against
    /// this schema.
    pub fn validate(
        &self,
        source: SourceInvocation,
    ) -> Result<ValidatedInvocation, CommandValidationError> {
        let SourceInvocation { command, arguments } = source;
        let mut validated = ValueMap::new();

        for (field, value) in arguments {
            if field == "this" {
                return Err(CommandValidationError::ReservedArgument { field });
            }
            let descriptor = self
                .required
                .get(&field)
                .or_else(|| self.optional.get(&field))
                .ok_or_else(|| CommandValidationError::UnknownArgument {
                    field: field.clone(),
                })?;
            let value = coerce_value(&field, descriptor.content_type(), value).map_err(
                |error| match error {
                    TransactError::TypeMismatch {
                        field,
                        expected,
                        found,
                    } => CommandValidationError::TypeMismatch {
                        field,
                        expected,
                        found,
                    },
                    TransactError::UnknownField { .. }
                    | TransactError::InvocationRequiresResolution { .. } => {
                        unreachable!("coerce_value only returns type mismatches")
                    }
                },
            )?;
            validated.insert(field, value);
        }

        for field in self.required.keys() {
            if !validated.contains_key(field) {
                return Err(CommandValidationError::MissingRequiredArgument {
                    field: field.clone(),
                });
            }
        }

        Ok(ValidatedInvocation {
            command,
            arguments: validated,
        })
    }
}

/// An invocation as supplied on the wire, before branch-authoritative
/// schema resolution and validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInvocation {
    /// Stable nominal command kind.
    pub command: Entity,
    /// Argument values keyed by the command schema's field names.
    #[serde(default)]
    pub arguments: ValueMap,
}

/// An invocation whose arguments conform to the command's current
/// schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedInvocation {
    command: Entity,
    arguments: ValueMap,
}

impl ValidatedInvocation {
    /// Stable nominal command kind.
    pub fn command(&self) -> &Entity {
        &self.command
    }

    /// Validated and losslessly coerced arguments.
    pub fn arguments(&self) -> &ValueMap {
        &self.arguments
    }

    /// Consume the invocation into its kind and argument map.
    pub fn into_parts(self) -> (Entity, ValueMap) {
        (self.command, self.arguments)
    }
}

/// A command argument validation failure.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CommandValidationError {
    /// An argument was supplied that the command schema does not declare.
    #[error("argument {field:?} is not declared by this command")]
    UnknownArgument {
        /// Undeclared argument name.
        field: String,
    },
    /// A required argument was absent.
    #[error("required command argument {field:?} is missing")]
    MissingRequiredArgument {
        /// Missing argument name.
        field: String,
    },
    /// The occurrence identifier was incorrectly supplied as a domain
    /// argument.
    #[error("command argument {field:?} is reserved")]
    ReservedArgument {
        /// Reserved argument name; currently always `this`.
        field: String,
    },
    /// An argument could not be losslessly coerced to its declared type.
    #[error("argument {field:?} expects {expected} but got an incompatible {found} value")]
    TypeMismatch {
        /// Argument name.
        field: String,
        /// Declared argument type.
        expected: ValueDataType,
        /// Supplied value type.
        found: ValueDataType,
    },
}

/// Transport metadata attached after payload validation. It is never
/// accepted as part of [`SourceInvocation::arguments`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationMetadata {
    occurrence: Entity,
    correlation: String,
}

impl InvocationMetadata {
    /// Attach an already-assigned occurrence entity and diagnostic
    /// correlation identifier.
    pub fn new(occurrence: Entity, correlation: impl Into<String>) -> Self {
        Self {
            occurrence,
            correlation: correlation.into(),
        }
    }

    /// Transaction-local occurrence entity.
    pub fn occurrence(&self) -> &Entity {
        &self.occurrence
    }

    /// Diagnostic correlation identifier.
    pub fn correlation(&self) -> &str {
        &self.correlation
    }
}

/// One independently identifiable occurrence of a validated command.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOccurrence {
    invocation: ValidatedInvocation,
    metadata: InvocationMetadata,
}

impl CommandOccurrence {
    /// Combine a validated payload with separately assigned metadata.
    pub fn new(invocation: ValidatedInvocation, metadata: InvocationMetadata) -> Self {
        Self {
            invocation,
            metadata,
        }
    }

    /// Transaction-local occurrence entity.
    pub fn occurrence(&self) -> &Entity {
        self.metadata.occurrence()
    }

    /// Stable nominal command kind.
    pub fn command(&self) -> &Entity {
        self.invocation.command()
    }

    /// Validated command arguments.
    pub fn arguments(&self) -> &ValueMap {
        self.invocation.arguments()
    }

    /// Diagnostic correlation identifier.
    pub fn correlation(&self) -> &str {
        self.metadata.correlation()
    }

    /// Consume the occurrence into its validated payload and metadata.
    pub fn into_parts(self) -> (ValidatedInvocation, InvocationMetadata) {
        (self.invocation, self.metadata)
    }
}

/// Command occurrences supplied to one evaluator round.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommandBatch {
    occurrences: Vec<CommandOccurrence>,
}

impl CommandBatch {
    /// Build a batch in source order.
    pub fn new(occurrences: Vec<CommandOccurrence>) -> Self {
        Self { occurrences }
    }

    /// Borrow the occurrences in source order.
    pub fn occurrences(&self) -> &[CommandOccurrence] {
        &self.occurrences
    }

    /// Consume the batch into its occurrences.
    pub fn into_occurrences(self) -> Vec<CommandOccurrence> {
        self.occurrences
    }

    /// Return whether the batch contains no occurrences.
    pub fn is_empty(&self) -> bool {
        self.occurrences.is_empty()
    }

    /// Encode the batch as private, transaction-local query overlay
    /// relations. Semantic command attributes are deliberately absent.
    pub fn encode(&self) -> Changes {
        let mut changes = Changes::new();
        let kind_attribute = command_kind_attribute();
        for occurrence in &self.occurrences {
            changes.associate(
                kind_attribute.clone(),
                occurrence.occurrence().clone(),
                Value::Entity(occurrence.command().clone()),
            );
            for (field, value) in occurrence.arguments() {
                changes.associate(
                    command_argument_attribute(field),
                    occurrence.occurrence().clone(),
                    value.clone(),
                );
            }
        }
        changes
    }
}

/// Construct the reserved command-kind relation.
pub fn command_kind_attribute() -> Attribute {
    COMMAND_KIND_RELATION
        .parse()
        .expect("reserved command kind relation is valid")
}

/// Construct the reserved relation for one validated command field.
pub fn command_argument_attribute(field: &str) -> Attribute {
    format!("{COMMAND_ARGUMENT_RELATION_PREFIX}{field}")
        .parse()
        .expect("validated command argument relation is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ValueMap;
    use dialog_artifacts::{Change, Entity, Value};
    use dialog_query::{AttributeDescriptor, Cardinality, Type};

    fn field(the: &str, content_type: Type) -> AttributeDescriptor {
        AttributeDescriptor::new(
            the.parse().expect("valid attribute"),
            "",
            Cardinality::One,
            Some(content_type),
        )
    }

    fn schema() -> CommandSchema {
        CommandSchema {
            required: [("title".into(), field("xyz.tonk.todo/title", Type::String))]
                .into_iter()
                .collect(),
            optional: [("done".into(), field("xyz.tonk.todo/done", Type::Boolean))]
                .into_iter()
                .collect(),
        }
    }

    fn invocation(arguments: ValueMap) -> SourceInvocation {
        SourceInvocation {
            command: "id:todo/add".parse().expect("valid command kind"),
            arguments,
        }
    }

    #[dialog_common::test]
    fn command_batch_encodes_reserved_relations_without_semantic_attributes() {
        let arguments = [("title".into(), Value::String("Buy milk".into()))]
            .into_iter()
            .collect();
        let validated = schema().validate(invocation(arguments)).unwrap();
        let first = CommandOccurrence::new(
            validated.clone(),
            InvocationMetadata::new(Entity::new().unwrap(), "invoke:first"),
        );
        let second = CommandOccurrence::new(
            validated,
            InvocationMetadata::new(Entity::new().unwrap(), "invoke:second"),
        );
        assert_ne!(first.occurrence(), second.occurrence());

        let changes = CommandBatch::new(vec![first.clone(), second.clone()]).encode();
        let triples = changes.iter().collect::<Vec<_>>();
        assert_eq!(
            triples
                .iter()
                .filter(|(_, attribute, change)| {
                    attribute.to_string() == "dialog.command/kind"
                        && matches!(change, Change::Assert(Value::Entity(kind)) if kind == first.command())
                })
                .count(),
            2
        );
        assert!(triples.iter().all(|(_, attribute, _)| {
            let attribute = attribute.to_string();
            attribute == "dialog.command/kind" || attribute == "dialog.command.argument/title"
        }));
        assert!(
            triples
                .iter()
                .all(|(_, attribute, _)| attribute.to_string() != "xyz.tonk.todo/title")
        );
    }

    #[dialog_common::test]
    fn command_validation_rejects_unknown_argument() {
        let arguments = [
            ("title".into(), Value::String("Buy milk".into())),
            ("extra".into(), Value::Boolean(true)),
        ]
        .into_iter()
        .collect();
        assert!(matches!(
            schema().validate(invocation(arguments)),
            Err(CommandValidationError::UnknownArgument { ref field }) if field == "extra"
        ));
    }

    #[dialog_common::test]
    fn command_validation_rejects_missing_required_argument() {
        assert!(matches!(
            schema().validate(invocation(ValueMap::new())),
            Err(CommandValidationError::MissingRequiredArgument { ref field }) if field == "title"
        ));
    }

    #[dialog_common::test]
    fn command_validation_omits_missing_optional_argument() {
        let arguments = [("title".into(), Value::String("Buy milk".into()))]
            .into_iter()
            .collect();
        let validated = schema().validate(invocation(arguments)).unwrap();
        assert_eq!(validated.arguments().get("done"), None);
    }

    #[dialog_common::test]
    fn command_validation_rejects_reserved_this_argument() {
        let arguments = [
            ("title".into(), Value::String("Buy milk".into())),
            (
                "this".into(),
                Value::Entity("id:todo/one".parse().expect("valid entity")),
            ),
        ]
        .into_iter()
        .collect();
        assert!(matches!(
            schema().validate(invocation(arguments)),
            Err(CommandValidationError::ReservedArgument { ref field }) if field == "this"
        ));
    }

    #[dialog_common::test]
    fn command_validation_rejects_type_mismatch() {
        let arguments = [("title".into(), Value::Boolean(true))]
            .into_iter()
            .collect();
        assert!(matches!(
            schema().validate(invocation(arguments)),
            Err(CommandValidationError::TypeMismatch { ref field, .. }) if field == "title"
        ));
    }

    #[dialog_common::test]
    fn command_validation_preserves_present_empty_text() {
        let arguments = [("title".into(), Value::String(String::new()))]
            .into_iter()
            .collect();
        let validated = schema().validate(invocation(arguments)).unwrap();
        assert_eq!(
            validated.arguments().get("title"),
            Some(&Value::String(String::new()))
        );
    }
}
