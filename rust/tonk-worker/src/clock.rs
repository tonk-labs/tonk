//! Wall-clock stamps that work on every target the worker builds for.
//!
//! The worker used to read the clock as `js_sys::Date::now()`, which is
//! only callable from wasm, with a native branch returning `0.0` on the
//! reasoning that native meant tests and tests had "no clock dependency".
//!
//! That stops being true the moment a command runs outside the browser:
//! the CLI dispatches the same handlers, and a sync-queue whose entries
//! all carry timestamp zero has no activity order at all — every space
//! looks equally stale, which is a scheduling bug rather than a test
//! convenience.
//!
//! `web_time` is the shim the rest of this crate already uses for exactly
//! this (`onboarding.rs`, `router/account.rs`, `session.rs`): it is
//! `std::time` natively and `Performance`/`Date` on wasm. Reading the
//! clock through one function here means a host is never the reason a
//! handler cannot run.

use web_time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
///
/// Used for sync-queue activity priority, where only the ordering
/// matters. A clock before the epoch (only reachable if the host clock
/// is badly wrong) reads as `0.0` rather than panicking — a bogus
/// ordering is survivable; taking the process down over a wall clock is
/// not.
pub fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Whole seconds since the Unix epoch, for facts that record a moment
/// (`created_at` and friends) rather than an ordering.
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The native stub this replaced returned zero, which would have
    /// given every sync-queue entry the same priority once the CLI began
    /// dispatching commands.
    #[dialog_common::test]
    fn it_reads_a_real_clock_on_every_target() {
        assert!(
            now_millis() > 1_700_000_000_000.0,
            "expected a wall-clock stamp after 2023, got {}",
            now_millis()
        );
        assert!(now_seconds() > 1_700_000_000);
    }

    #[dialog_common::test]
    fn the_two_units_agree() {
        let millis = now_millis();
        let seconds = now_seconds();
        assert!(
            (millis / 1000.0 - seconds as f64).abs() < 2.0,
            "millis {millis} and seconds {seconds} disagree by more than a tick",
        );
    }
}
