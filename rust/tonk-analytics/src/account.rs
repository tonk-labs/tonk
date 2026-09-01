//! Privacy-safe, cross-interface account journey events.
//!
//! The public constructors accept only closed vocabulary. Callers cannot add
//! arbitrary properties, which keeps account content and diagnostics out of
//! analytics by construction.

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

/// Current account event schema.
pub const SCHEMA_VERSION: u8 = 1;
/// Longest duration retained in analytics (ten minutes).
pub const MAX_DURATION_MS: u64 = 600_000;

macro_rules! closed_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize)]
        #[allow(missing_docs)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }
    };
}

closed_enum! {
    /// Account journey used for funnel grouping.
    Journey { Onboarding, Login, Activation, Passkey, AccountManagement, CliHandoff, AccountDeletion }
}

closed_enum! {
    /// Stable account operation shared by web and CLI.
    AccountAction {
        OpenRegistration, LoadAccount, LoadRegistration, CheckEmail, CreateAccount, Login,
        AddPasskey, ChangeDisplayName, ResendActivation, LoadDevices, LoadProfiles, LinkCli,
        SwitchProfile, SignOut, LoadDeletionPlan, DeleteAccount, DeleteSpace, RevokeDevice,
        FinishAccountBackup, ActivateAccount, WatchActivation, SaveInitialDisplayName,
        CopyInvite, FinishPreviousAction, SettleAccount, LoadAccountSpaces, PullAccountSpace,
        OpenAccountDeletion, OpenSpaceDeletion, SyncAccount
    }
}

closed_enum! {
    /// Position reached by an account attempt.
    Stage {
        Input, EmailLookup, LocalPreflight, PasskeyCreate, PasskeyAssert, Prf, WorkerHandoff,
        AccessService, LocalCommit, RemoteCommit, ActivationWait, CallbackBind, BrowserOpen,
        CallbackWait, CallbackDelivery, DelegationValidate, ActivationStage, AccountSync,
        ContentDiscovery, CustodyRotation, AccountLoad, Complete
    }
}

closed_enum! {
    /// Interface on which this attempt is owned.
    Surface { RegistrationDialog, Settings, ActivationPage, CustodyConsent, Hub, CliCallback, NativeCli }
}

closed_enum! {
    /// What initiated an attempt.
    Trigger { User, Automatic, Recovery }
}

closed_enum! {
    /// Coarse account readiness at attempt start.
    AccountState { None, Onboarding, PendingActivation, RegisteredUnready, Ready, Unknown }
}

closed_enum! {
    /// Terminal result. Expected friction remains distinct from reliability failures.
    AccountResult { Success, DegradedSuccess, Cancelled, Blocked, RetryableFailure, TerminalFailure, UnknownCommit }
}

closed_enum! {
    /// Privacy-safe reason a non-success terminal event ended.
    FailureKind {
        InvalidInput, Cancelled, Timeout, CredentialExists, PasskeyUnsupported, PrfUnsupported,
        SecurityContext, AwaitingActivation, Suspended, NotProvisioned, AccessDenied, Conflict,
        NotFound, RateLimited, Network, ServiceUnavailable, InvalidResponse, LocalState,
        Callback, Unknown
    }
}

closed_enum! {
    /// Earliest incomplete follow-up after the durable account operation succeeded.
    DegradationKind { BrowserOpen, AccountSync, ContentDiscovery, CustodyRotation, SpaceRotation }
}

closed_enum! {
    /// HTTP status category retained without a URL or response body.
    HttpStatusClass { #[serde(rename = "4xx")] ClientError, #[serde(rename = "5xx")] ServerError }
}

closed_enum! {
    /// Stable, explicitly reviewed service code. Unknown input is normalized to `Unknown`.
    ServiceCode {
        RootRequired, CredentialRevoked, UpstreamTimeout, UpstreamUnavailable,
        AccountStateUnavailable, Invalid, Unauthorized, Forbidden, UnknownCustomer,
        UnknownConsumer, CustomerActive, CustomerInactive, CustomerSuspended, AddressTaken,
        ConsumerProvided, Internal, Unknown
    }
}

impl ServiceCode {
    /// Normalize an already parsed service code into the allowlist.
    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "root_required" => Self::RootRequired,
            "credential_revoked" => Self::CredentialRevoked,
            "upstream_timeout" => Self::UpstreamTimeout,
            "upstream_unavailable" => Self::UpstreamUnavailable,
            "account_state_unavailable" => Self::AccountStateUnavailable,
            "invalid" => Self::Invalid,
            "unauthorized" => Self::Unauthorized,
            "forbidden" => Self::Forbidden,
            "unknown_customer" => Self::UnknownCustomer,
            "unknown_consumer" => Self::UnknownConsumer,
            "customer_active" => Self::CustomerActive,
            "customer_inactive" => Self::CustomerInactive,
            "customer_suspended" => Self::CustomerSuspended,
            "address_taken" => Self::AddressTaken,
            "consumer_provided" => Self::ConsumerProvided,
            "internal" => Self::Internal,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Started,
    Checkpoint,
    Finished,
}

impl Serialize for Phase {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(match self {
            Self::Started => "started",
            Self::Checkpoint => "checkpoint",
            Self::Finished => "finished",
        })
    }
}

/// Closed terminal outcome supplied to [`AccountEvent::finished`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountOutcome {
    result: AccountResult,
    failure_kind: Option<FailureKind>,
    degradation_kind: Option<DegradationKind>,
    http_status_class: Option<HttpStatusClass>,
    service_code: Option<ServiceCode>,
}

impl AccountOutcome {
    /// Full success.
    pub const fn success() -> Self {
        Self::new(AccountResult::Success, None, None)
    }

    /// Success with a non-transactional follow-up left incomplete.
    pub const fn degraded(kind: DegradationKind) -> Self {
        Self::new(AccountResult::DegradedSuccess, None, Some(kind))
    }

    /// User cancellation.
    pub const fn cancelled() -> Self {
        Self::new(AccountResult::Cancelled, Some(FailureKind::Cancelled), None)
    }

    /// Expected account or policy gate.
    pub const fn blocked(kind: FailureKind) -> Self {
        Self::new(AccountResult::Blocked, Some(kind), None)
    }

    /// Failure for which retry is known to be safe.
    pub const fn retryable(kind: FailureKind) -> Self {
        Self::new(AccountResult::RetryableFailure, Some(kind), None)
    }

    /// Failure known not to have committed.
    pub const fn terminal_failure(kind: FailureKind) -> Self {
        Self::new(AccountResult::TerminalFailure, Some(kind), None)
    }

    /// Failure after a boundary at which commitment cannot be disproved.
    pub const fn unknown_commit(kind: FailureKind) -> Self {
        Self::new(AccountResult::UnknownCommit, Some(kind), None)
    }

    const fn new(
        result: AccountResult,
        failure_kind: Option<FailureKind>,
        degradation_kind: Option<DegradationKind>,
    ) -> Self {
        Self {
            result,
            failure_kind,
            degradation_kind,
            http_status_class: None,
            service_code: None,
        }
    }

    /// Add an HTTP status category.
    pub const fn with_http_status_class(mut self, value: HttpStatusClass) -> Self {
        self.http_status_class = Some(value);
        self
    }

    /// Add an allowlisted stable service code.
    pub const fn with_service_code(mut self, value: ServiceCode) -> Self {
        self.service_code = Some(value);
        self
    }

    /// Terminal result value.
    pub const fn result(self) -> AccountResult {
        self.result
    }

    /// Failure classification, when applicable.
    pub const fn failure_kind(self) -> Option<FailureKind> {
        self.failure_kind
    }

    /// Degradation classification, when applicable.
    pub const fn degradation_kind(self) -> Option<DegradationKind> {
        self.degradation_kind
    }
}

/// A canonical account event. Its property map is private and validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountEvent {
    journey: Journey,
    action: AccountAction,
    phase: Phase,
    stage: Stage,
    surface: Surface,
    trigger: Trigger,
    account_state: AccountState,
    attempt_id: String,
    outcome: Option<AccountOutcome>,
    duration_ms: Option<u64>,
}

/// Why an account event shape was rejected.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// Attempt identifier was empty, too long, or non-ASCII.
    #[error("attempt_id must be 1..=36 ASCII characters")]
    InvalidAttemptId,
    /// A non-terminal event carried terminal properties, or vice versa.
    #[error("phase and terminal properties are inconsistent")]
    InvalidPhase,
    /// Result, failure, and degradation fields do not form a valid outcome.
    #[error("terminal outcome fields are inconsistent")]
    InvalidOutcome,
    /// Duration exceeded the ten-minute analytics cap.
    #[error("duration_ms exceeds 600000")]
    DurationTooLong,
}

impl AccountEvent {
    fn non_terminal(
        phase: Phase,
        journey: Journey,
        action: AccountAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self {
            journey,
            action,
            phase,
            stage,
            surface,
            trigger,
            account_state,
            attempt_id: attempt_id.into(),
            outcome: None,
            duration_ms: None,
        }
    }

    /// Begin an account attempt.
    pub fn started(
        journey: Journey,
        action: AccountAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self::non_terminal(
            phase::started(),
            journey,
            action,
            stage,
            surface,
            trigger,
            account_state,
            attempt_id,
        )
    }

    /// Record progress in an existing attempt.
    pub fn checkpoint(
        journey: Journey,
        action: AccountAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self::non_terminal(
            Phase::Checkpoint,
            journey,
            action,
            stage,
            surface,
            trigger,
            account_state,
            attempt_id,
        )
    }

    /// Finish an account attempt.
    #[allow(clippy::too_many_arguments)]
    pub fn finished(
        journey: Journey,
        action: AccountAction,
        stage: Stage,
        surface: Surface,
        trigger: Trigger,
        account_state: AccountState,
        attempt_id: impl Into<String>,
        duration_ms: u64,
        outcome: AccountOutcome,
    ) -> Self {
        Self {
            journey,
            action,
            phase: Phase::Finished,
            stage,
            surface,
            trigger,
            account_state,
            attempt_id: attempt_id.into(),
            outcome: Some(outcome),
            duration_ms: Some(duration_ms.min(MAX_DURATION_MS)),
        }
    }

    /// Validate and serialize the exact account-owned property allowlist.
    pub fn validated_properties(&self) -> Result<Map<String, Value>, ValidationError> {
        self.validate()?;
        let mut properties = Map::new();
        insert(&mut properties, "schema_version", SCHEMA_VERSION);
        insert(&mut properties, "journey", self.journey);
        insert(&mut properties, "action", self.action);
        insert(&mut properties, "phase", self.phase);
        insert(&mut properties, "stage", self.stage);
        insert(&mut properties, "surface", self.surface);
        insert(&mut properties, "trigger", self.trigger);
        insert(&mut properties, "account_state", self.account_state);
        insert(&mut properties, "attempt_id", &self.attempt_id);
        if let Some(outcome) = self.outcome {
            insert(&mut properties, "result", outcome.result);
            if let Some(value) = outcome.failure_kind {
                insert(&mut properties, "failure_kind", value);
            }
            if let Some(value) = outcome.degradation_kind {
                insert(&mut properties, "degradation_kind", value);
            }
            if let Some(value) = outcome.http_status_class {
                insert(&mut properties, "http_status_class", value);
            }
            if let Some(value) = outcome.service_code {
                insert(&mut properties, "service_code", value);
            }
        }
        if let Some(value) = self.duration_ms {
            insert(&mut properties, "duration_ms", value);
        }
        Ok(properties)
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.attempt_id.is_empty() || self.attempt_id.len() > 36 || !self.attempt_id.is_ascii() {
            return Err(ValidationError::InvalidAttemptId);
        }
        match (self.phase, self.outcome, self.duration_ms) {
            (Phase::Finished, Some(_), Some(duration)) if duration <= MAX_DURATION_MS => {}
            (Phase::Started | Phase::Checkpoint, None, None) => return Ok(()),
            (Phase::Finished, Some(_), Some(_)) => return Err(ValidationError::DurationTooLong),
            _ => return Err(ValidationError::InvalidPhase),
        }
        let outcome = self.outcome.expect("validated terminal outcome");
        let valid = match outcome.result {
            AccountResult::Success => {
                outcome.failure_kind.is_none() && outcome.degradation_kind.is_none()
            }
            AccountResult::DegradedSuccess => {
                outcome.failure_kind.is_none() && outcome.degradation_kind.is_some()
            }
            AccountResult::Cancelled => {
                outcome.failure_kind == Some(FailureKind::Cancelled)
                    && outcome.degradation_kind.is_none()
            }
            AccountResult::Blocked
            | AccountResult::RetryableFailure
            | AccountResult::TerminalFailure
            | AccountResult::UnknownCommit => {
                outcome.failure_kind.is_some() && outcome.degradation_kind.is_none()
            }
        };
        valid.then_some(()).ok_or(ValidationError::InvalidOutcome)
    }
}

fn insert<T: Serialize>(properties: &mut Map<String, Value>, key: &str, value: T) {
    properties.insert(
        key.to_owned(),
        serde_json::to_value(value).expect("closed account value serializes"),
    );
}

mod phase {
    use super::Phase;
    pub(super) const fn started() -> Phase {
        Phase::Started
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> AccountEvent {
        AccountEvent::started(
            Journey::Login,
            AccountAction::Login,
            Stage::Input,
            Surface::Settings,
            Trigger::User,
            AccountState::None,
            "attempt-1",
        )
    }

    #[test]
    fn it_serializes_only_the_account_event_allowlist() {
        let start = started().validated_properties().unwrap();
        assert_eq!(
            start.keys().cloned().collect::<Vec<_>>(),
            [
                "account_state",
                "action",
                "attempt_id",
                "journey",
                "phase",
                "schema_version",
                "stage",
                "surface",
                "trigger"
            ]
        );
        assert_eq!(start["action"], "login");
        assert_eq!(start["phase"], "started");
        assert!(!start.contains_key("result"));
        assert!(!start.contains_key("duration_ms"));

        let terminal = AccountEvent::finished(
            Journey::Activation,
            AccountAction::ActivateAccount,
            Stage::AccessService,
            Surface::ActivationPage,
            Trigger::Recovery,
            AccountState::PendingActivation,
            "attempt-2",
            42,
            AccountOutcome::blocked(FailureKind::AwaitingActivation)
                .with_http_status_class(HttpStatusClass::ClientError)
                .with_service_code(ServiceCode::Forbidden),
        )
        .validated_properties()
        .unwrap();
        assert_eq!(terminal["result"], "blocked");
        assert_eq!(terminal["failure_kind"], "awaiting_activation");
        assert_eq!(terminal["http_status_class"], "4xx");
        assert_eq!(terminal["duration_ms"], 42);
    }

    #[test]
    fn it_rejects_invalid_account_event_shapes() {
        let mut event = started();
        event.duration_ms = Some(1);
        assert_eq!(event.validate(), Err(ValidationError::InvalidPhase));

        let mut event = AccountEvent::finished(
            Journey::Login,
            AccountAction::Login,
            Stage::Complete,
            Surface::Settings,
            Trigger::User,
            AccountState::Ready,
            "x",
            1,
            AccountOutcome::success(),
        );
        event.outcome.as_mut().unwrap().failure_kind = Some(FailureKind::Unknown);
        assert_eq!(event.validate(), Err(ValidationError::InvalidOutcome));

        event.outcome = Some(AccountOutcome::degraded(DegradationKind::AccountSync));
        event.outcome.as_mut().unwrap().degradation_kind = None;
        assert_eq!(event.validate(), Err(ValidationError::InvalidOutcome));

        event.duration_ms = Some(MAX_DURATION_MS + 1);
        assert_eq!(event.validate(), Err(ValidationError::DurationTooLong));
    }

    #[test]
    fn privacy_sentinels_cannot_enter_the_typed_event() {
        let nearby_inputs = "person@example.com did:key:zSensitive credential-id https://x/activate?ucan=secret http://127.0.0.1/callback body-secret";
        let json = Value::Object(started().validated_properties().unwrap()).to_string();
        for sentinel in nearby_inputs.split_whitespace() {
            assert!(!json.contains(sentinel));
        }
    }

    #[test]
    fn service_codes_are_allowlisted() {
        assert_eq!(
            ServiceCode::from_wire("ROOT-REQUIRED"),
            ServiceCode::RootRequired
        );
        assert_eq!(
            ServiceCode::from_wire("person@example.com"),
            ServiceCode::Unknown
        );
    }
}
