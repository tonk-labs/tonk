//! Privacy-safe lifecycle events for product-owned interactions.
//!
//! The schema deliberately accepts only closed vocabulary. Product code can
//! correlate one user attempt across frame and worker boundaries without
//! sending labels, routes, URLs, content, or rendered error messages.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// Current product event schema.
pub const SCHEMA_VERSION: u8 = 1;
/// Longest duration retained in analytics (ten minutes).
pub const MAX_DURATION_MS: u64 = 600_000;

/// Create an opaque random token for correlating one interaction lifecycle.
pub fn attempt_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

macro_rules! closed_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
        #[allow(missing_docs)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }
    };
}

closed_enum! {
    /// Product journey used for dashboard grouping.
    Journey { Startup, Account, Space, Collaboration, Workspace, Sync, Handoff }
}

closed_enum! {
    /// Stable product-owned operation.
    ProductAction {
        WorkerReady, WelcomeReady, ProductReady,
        SaveDisplayName, LoadDeletionPlan, DeleteAccount, DeleteHostedSpace, SignOut,
        LoadProfiles, AddProfile, SwitchProfile, AddPasskey, LinkDevice,
        ResolveInvite, RetryJoin, CreateSpace, JoinSpace, MintInvite, CopyShareLink,
        PrepareAgentPrompt, CopyAgentPrompt, ConnectAgent, EnableSync, PauseSync,
        RenameSpace, RemoveSpace, UpdateSeed, ForgetInvitation,
        ActivateSheet, CreateSheet, CloseSheet, Edit, Evaluate, Query, Inspect,
        Import, Export, Blob, Push, Pull, Upgrade, Help, Identity, ConfigureSpace,
        Concept, View, Render, Remote, Migrate
    }
}

closed_enum! {
    /// Position reached by an interaction.
    Stage { Intent, Validation, Worker, Welcome, LocalCommit, RemoteCommit, Clipboard, Ready, Complete }
}

closed_enum! {
    /// Product surface that owns the interaction.
    Surface { Shell, Welcome, Settings, Hub, Workspace, Join, NativeCli }
}

closed_enum! {
    /// What initiated the interaction.
    Trigger { User, Automatic, Recovery }
}

closed_enum! {
    /// Terminal result of an interaction.
    ProductResult { Success, DegradedSuccess, Cancelled, Blocked, Noop, RetryableFailure, TerminalFailure, UnknownCommit }
}

closed_enum! {
    /// Reviewed reason for a non-success result.
    FailureKind {
        InvalidInput, Cancelled, Timeout, Unsupported, AccessDenied, NotFound,
        Conflict, Network, ServiceUnavailable, InvalidResponse, LocalState, Unknown
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Started,
    Checkpoint,
    Finished,
}

/// A closed, content-free product interaction event.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductEvent {
    schema_version: u8,
    journey: Journey,
    action: ProductAction,
    phase: Phase,
    stage: Stage,
    surface: Surface,
    trigger: Trigger,
    attempt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ProductResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_kind: Option<FailureKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

/// Why a product event was rejected.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// The event used a schema this build does not understand.
    #[error("unsupported product event schema")]
    InvalidSchema,
    /// Attempt identifier was empty, too long, or non-ASCII.
    #[error("attempt_id must be 1..=36 ASCII characters")]
    InvalidAttemptId,
    /// A non-terminal event carried terminal fields, or vice versa.
    #[error("phase and terminal properties are inconsistent")]
    InvalidPhase,
    /// Result and failure fields do not form a valid outcome.
    #[error("terminal outcome fields are inconsistent")]
    InvalidOutcome,
    /// Duration exceeded the analytics cap.
    #[error("duration_ms exceeds 600000")]
    DurationTooLong,
    /// The relayed value was not a product event object.
    #[error("invalid product event properties")]
    InvalidProperties,
}

impl ProductEvent {
    /// Begin one product-owned interaction.
    pub fn started(
        journey: Journey,
        action: ProductAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self::non_terminal(
            journey,
            action,
            Phase::Started,
            stage,
            surface,
            trigger,
            attempt_id,
        )
    }

    /// Record a meaningful intermediate receipt for the same attempt.
    pub fn checkpoint(
        journey: Journey,
        action: ProductAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self::non_terminal(
            journey,
            action,
            Phase::Checkpoint,
            stage,
            surface,
            trigger,
            attempt_id,
        )
    }

    fn non_terminal(
        journey: Journey,
        action: ProductAction,
        phase: Phase,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            journey,
            action,
            phase,
            stage,
            surface,
            trigger,
            attempt_id: attempt_id.into(),
            result: None,
            failure_kind: None,
            duration_ms: None,
        }
    }

    /// Finish one interaction at its actual product receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn finished(
        journey: Journey,
        action: ProductAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        attempt_id: impl Into<String>,
        duration_ms: u64,
        result: ProductResult,
        failure_kind: Option<FailureKind>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            journey,
            action,
            phase: Phase::Finished,
            stage,
            surface,
            trigger,
            attempt_id: attempt_id.into(),
            result: Some(result),
            failure_kind,
            duration_ms: Some(duration_ms.min(MAX_DURATION_MS)),
        }
    }

    /// Validate and return the exact PostHog property allowlist.
    pub fn validated_properties(&self) -> Result<Map<String, Value>, ValidationError> {
        self.validate()?;
        let Value::Object(properties) =
            serde_json::to_value(self).expect("product event serializes")
        else {
            unreachable!("product event serializes as an object")
        };
        Ok(properties)
    }

    /// Parse a relayed property object and re-apply the schema validation.
    pub fn from_properties(properties: &Value) -> Result<Self, ValidationError> {
        let event: Self = serde_json::from_value(properties.clone())
            .map_err(|_| ValidationError::InvalidProperties)?;
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::InvalidSchema);
        }
        if self.attempt_id.is_empty() || self.attempt_id.len() > 36 || !self.attempt_id.is_ascii() {
            return Err(ValidationError::InvalidAttemptId);
        }
        if self
            .duration_ms
            .is_some_and(|value| value > MAX_DURATION_MS)
        {
            return Err(ValidationError::DurationTooLong);
        }
        match self.phase {
            Phase::Started | Phase::Checkpoint => {
                if self.result.is_some()
                    || self.failure_kind.is_some()
                    || self.duration_ms.is_some()
                {
                    return Err(ValidationError::InvalidPhase);
                }
            }
            Phase::Finished => {
                let Some(result) = self.result else {
                    return Err(ValidationError::InvalidPhase);
                };
                if self.duration_ms.is_none() {
                    return Err(ValidationError::InvalidPhase);
                }
                let needs_failure = !matches!(
                    result,
                    ProductResult::Success | ProductResult::DegradedSuccess | ProductResult::Noop
                );
                if needs_failure != self.failure_kind.is_some() {
                    return Err(ValidationError::InvalidOutcome);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_event_serializes_only_the_closed_contract() {
        let event = ProductEvent::finished(
            Journey::Account,
            ProductAction::SaveDisplayName,
            Stage::RemoteCommit,
            Surface::Settings,
            Trigger::User,
            "attempt-1",
            42,
            ProductResult::Success,
            None,
        );
        let properties = event.validated_properties().unwrap();
        assert_eq!(properties.len(), 10);
        assert_eq!(properties["action"], "save_display_name");
        assert_eq!(properties["phase"], "finished");
        assert_eq!(properties["result"], "success");
        assert!(!properties.contains_key("name"));
        assert_eq!(
            ProductEvent::from_properties(&Value::Object(properties)).unwrap(),
            event
        );
    }

    #[test]
    fn product_event_rejects_invalid_relayed_shapes() {
        let invalid = serde_json::json!({
            "schema_version": 1,
            "journey": "account",
            "action": "save_display_name",
            "phase": "finished",
            "stage": "remote_commit",
            "surface": "settings",
            "trigger": "user",
            "attempt_id": "attempt-1",
            "duration_ms": 3,
            "result": "success",
            "name": "private"
        });
        assert_eq!(
            ProductEvent::from_properties(&invalid),
            Err(ValidationError::InvalidProperties)
        );
    }

    #[test]
    fn attempt_ids_are_bounded_opaque_tokens() {
        let first = attempt_id();
        let second = attempt_id();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
