//! Product analytics owned by workspace components.

use tonk_analytics::product::{
    FailureKind, Journey, ProductAction, ProductEvent, ProductResult, Stage, Surface, Trigger,
};

/// One workspace-owned product attempt.
pub(crate) struct Attempt {
    journey: Journey,
    action: ProductAction,
    surface: Surface,
    trigger: Trigger,
    attempt_id: String,
    started_ms: f64,
    finished: bool,
}

impl Attempt {
    /// Start and immediately relay an attempt.
    pub(crate) fn start(
        journey: Journey,
        action: ProductAction,
        surface: Surface,
        trigger: Trigger,
        stage: Stage,
    ) -> Self {
        let attempt_id = tonk_analytics::product::attempt_id();
        capture(&ProductEvent::started(
            journey,
            action,
            stage,
            surface,
            trigger,
            attempt_id.clone(),
        ));
        Self {
            journey,
            action,
            surface,
            trigger,
            attempt_id,
            started_ms: js_sys::Date::now(),
            finished: false,
        }
    }

    /// Finish exactly once at a known product receipt.
    pub(crate) fn finish(
        &mut self,
        stage: Stage,
        result: ProductResult,
        failure: Option<FailureKind>,
    ) {
        if self.finished {
            return;
        }
        self.finished = true;
        let duration_ms = (js_sys::Date::now() - self.started_ms).max(0.0) as u64;
        capture(&ProductEvent::finished(
            self.journey,
            self.action,
            stage,
            self.surface,
            self.trigger,
            self.attempt_id.clone(),
            duration_ms,
            result,
            failure,
        ));
    }

    /// Finish from a structured host transport error.
    pub(crate) fn finish_error(&mut self, stage: Stage, error: &tonk_host::error::ErrorDetail) {
        let (result, failure) = classify_error(error);
        self.finish(stage, result, Some(failure));
    }

    /// Return the opaque correlation token needed when a receipt arrives via
    /// a later subscription frame rather than this call stack.
    pub(crate) fn token(&self) -> (&str, f64) {
        (&self.attempt_id, self.started_ms)
    }
}

/// Finish an attempt whose receipt crossed an asynchronous subscription.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_existing(
    journey: Journey,
    action: ProductAction,
    surface: Surface,
    trigger: Trigger,
    attempt_id: String,
    started_ms: f64,
    stage: Stage,
    result: ProductResult,
    failure: Option<FailureKind>,
) {
    capture(&ProductEvent::finished(
        journey,
        action,
        stage,
        surface,
        trigger,
        attempt_id,
        (js_sys::Date::now() - started_ms).max(0.0) as u64,
        result,
        failure,
    ));
}

/// Record a synchronous receipt as a matched start and finish pair.
pub(crate) fn instant(
    journey: Journey,
    action: ProductAction,
    surface: Surface,
    trigger: Trigger,
    stage: Stage,
    result: ProductResult,
    failure: Option<FailureKind>,
) {
    let mut attempt = Attempt::start(journey, action, surface, trigger, Stage::Intent);
    attempt.finish(stage, result, failure);
}

fn capture(event: &ProductEvent) {
    let Ok(properties) = event.validated_properties() else {
        return;
    };
    tonk_host::analytics::capture(
        tonk_analytics::event::PRODUCT,
        &serde_json::Value::Object(properties),
    );
}

fn classify_error(error: &tonk_host::error::ErrorDetail) -> (ProductResult, FailureKind) {
    match error.status {
        Some(401 | 403) => (ProductResult::Blocked, FailureKind::AccessDenied),
        Some(404) => (ProductResult::Blocked, FailureKind::NotFound),
        Some(409) => (ProductResult::Blocked, FailureKind::Conflict),
        Some(500..=599) => (
            ProductResult::RetryableFailure,
            FailureKind::ServiceUnavailable,
        ),
        Some(_) => (ProductResult::TerminalFailure, FailureKind::InvalidResponse),
        None => match error.kind {
            tonk_host::error::ErrorKind::Network => {
                (ProductResult::RetryableFailure, FailureKind::Network)
            }
            tonk_host::error::ErrorKind::Parse => {
                (ProductResult::TerminalFailure, FailureKind::InvalidResponse)
            }
            tonk_host::error::ErrorKind::UnknownSource
            | tonk_host::error::ErrorKind::Descriptor => {
                (ProductResult::TerminalFailure, FailureKind::LocalState)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_errors_map_to_closed_failure_classes() {
        assert_eq!(
            classify_error(&tonk_host::error::ErrorDetail::http(403, "private")),
            (ProductResult::Blocked, FailureKind::AccessDenied)
        );
        assert_eq!(
            classify_error(&tonk_host::error::ErrorDetail::new(
                tonk_host::error::ErrorKind::Parse,
                "private"
            )),
            (ProductResult::TerminalFailure, FailureKind::InvalidResponse)
        );
    }
}
