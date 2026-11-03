//! `/api/inspect/diagnostics`: the worker's own counters and durations.
//!
//! The page cannot reach the service worker's globals, so this route is how
//! a test, a trace capture or a person reads what the worker recorded in
//! [`tonk_common::diagnostics`]: the state lock's waits and holds by the line
//! that took it, among others. The snapshot's `instance` changes when the
//! browser restarts the worker, which is how a restart shows up from outside.
//!
//! Neither handler takes the state lock, so the route answers while the lock
//! is contended, which is when it is most useful.

use axum::Json;
use axum::http::StatusCode;
use tonk_common::diagnostics::{self, Snapshot};

/// Everything the worker has recorded since it started or was last reset.
pub async fn read() -> Json<Snapshot> {
    Json(diagnostics::snapshot())
}

/// Clear the worker's counters and durations, so a scenario can measure one
/// phase alone. The instance identifier stays.
pub async fn reset() -> StatusCode {
    diagnostics::reset();
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonk_common::diagnostics::names;

    #[dialog_common::test]
    async fn it_reads_and_resets_the_worker_registry() {
        let _serial = crate::state_lock::DIAGNOSTICS_TESTS.lock().await;
        diagnostics::reset();
        diagnostics::count(names::SITE_REROUTE);
        let Json(snapshot) = read().await;
        assert_eq!(snapshot.counters[names::SITE_REROUTE], 1);

        assert_eq!(reset().await, StatusCode::NO_CONTENT);
        let Json(cleared) = read().await;
        assert!(cleared.counters.is_empty());
        assert_eq!(cleared.instance, snapshot.instance);
    }
}
