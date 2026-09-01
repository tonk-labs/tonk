//! Content-free account failure records for Cloudflare Workers Logs.

use serde::Serialize;

/// Account-related Worker operation.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessOperation {
    /// Customer enrollment.
    Enrollment,
    /// Email-link activation.
    Activation,
    /// Activation email resend.
    Resend,
    /// Address lookup.
    Lookup,
    /// Customer state probe.
    CustomerProbe,
    /// UCAN authorization.
    Authorization,
    /// Customer provisioning gate.
    Provisioning,
}

/// Coarse operational outcome.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessOutcome {
    /// Caller or policy refusal.
    Refused,
    /// Dependency unavailable; retry may be safe.
    Unavailable,
    /// Unexpected internal failure.
    Failed,
}

/// Closed infrastructure failure family.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessFailureKind {
    /// Invalid request shape.
    Invalid,
    /// Authentication or authorization refusal.
    AccessDenied,
    /// Requested record was absent.
    NotFound,
    /// State conflict.
    Conflict,
    /// Request throttled.
    RateLimited,
    /// A required binding or upstream dependency was unavailable.
    Unavailable,
    /// Provisioning policy refused service.
    NotProvisioned,
    /// Unexpected internal failure.
    Internal,
}

/// Static source location for otherwise unknown internal failures.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessSite {
    /// Worker request entrypoint.
    Entry,
    /// Control D1 access.
    ControlStore,
    /// Lookup rate limiter.
    LookupLimiter,
    /// UCAN parsing or authorization.
    Ucan,
    /// Provisioning screen.
    Provisioning,
    /// Registration command handler.
    Registration,
}

/// Exact JSON object written for one failed account request.
#[derive(Debug, Serialize)]
pub struct AccessFailureLog {
    schema_version: u8,
    system: &'static str,
    operation: AccessOperation,
    outcome: AccessOutcome,
    failure_kind: AccessFailureKind,
    status_class: &'static str,
    retryable: bool,
    version: &'static str,
    site: AccessSite,
}

impl AccessFailureLog {
    /// Construct a record exclusively from closed, content-free values.
    pub const fn new(
        operation: AccessOperation,
        outcome: AccessOutcome,
        failure_kind: AccessFailureKind,
        status: u16,
        retryable: bool,
        site: AccessSite,
    ) -> Self {
        Self {
            schema_version: 1,
            system: "access_worker",
            operation,
            outcome,
            failure_kind,
            status_class: if status >= 500 { "5xx" } else { "4xx" },
            retryable,
            version: env!("CARGO_PKG_VERSION"),
            site,
        }
    }

    /// Serialize for Workers Logs. This cannot fail for the closed schema.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("closed access log serializes")
    }

    /// Write at a severity matching the HTTP status class.
    #[cfg(target_arch = "wasm32")]
    pub fn emit(&self) {
        let json = self.to_json();
        if self.status_class == "5xx" {
            worker::console_error!("{json}");
        } else {
            worker::console_warn!("{json}");
        }
    }

    /// Native builds retain the schema for tests but have no deployed log sink.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn emit(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_serializes_the_exact_content_free_shape() {
        let log = AccessFailureLog::new(
            AccessOperation::Lookup,
            AccessOutcome::Unavailable,
            AccessFailureKind::Unavailable,
            503,
            true,
            AccessSite::ControlStore,
        );
        let value: serde_json::Value = serde_json::from_str(&log.to_json()).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            [
                "failure_kind",
                "operation",
                "outcome",
                "retryable",
                "schema_version",
                "site",
                "status_class",
                "system",
                "version"
            ]
        );
        assert_eq!(value["operation"], "lookup");
        assert_eq!(value["status_class"], "5xx");
    }

    #[test]
    fn sensitive_inputs_cannot_enter_the_log() {
        let nearby = "person@example.com did:key:zSensitive r2/key invocation activation?ucan=x database-exploded-secret";
        let json = AccessFailureLog::new(
            AccessOperation::Authorization,
            AccessOutcome::Failed,
            AccessFailureKind::Internal,
            500,
            false,
            AccessSite::Ucan,
        )
        .to_json();
        for sentinel in nearby.split_whitespace() {
            assert!(!json.contains(sentinel));
        }
    }

    #[test]
    fn every_account_operation_has_a_stable_value() {
        for operation in [
            AccessOperation::Enrollment,
            AccessOperation::Activation,
            AccessOperation::Resend,
            AccessOperation::Lookup,
            AccessOperation::CustomerProbe,
            AccessOperation::Authorization,
            AccessOperation::Provisioning,
        ] {
            let json = AccessFailureLog::new(
                operation,
                AccessOutcome::Refused,
                AccessFailureKind::Invalid,
                400,
                false,
                AccessSite::Entry,
            )
            .to_json();
            assert!(json.contains("\"operation\":"));
        }
    }
}
