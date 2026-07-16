//! Cached result of the release check (`update.json`), and the
//! cadence rules for when to check and when to nag.
//!
//! The nag prints from this cache, never from an in-flight check: a
//! check that misses the exit window just shifts the nag one
//! invocation later, so nothing has to finish in time for anything to
//! be correct.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How long a cached check stays fresh.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Minimum gap between two nags, so an ignored update is a daily
/// line rather than a per-command one.
pub const NAG_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Persisted check state. Lives beside `telemetry.json` in the
/// platform data dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Whether the background check runs (opt-out model: default on).
    pub check_enabled: bool,
    /// RFC3339 time of the last check attempt, successful or not.
    pub last_checked_at: Option<String>,
    /// RFC3339 time the nag last printed.
    pub last_nagged_at: Option<String>,
    /// Version the last successful check saw on the channel.
    pub latest_version: Option<String>,
    /// Commit the last successful check saw on the channel.
    pub latest_commit: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            check_enabled: true,
            last_checked_at: None,
            last_nagged_at: None,
            latest_version: None,
            latest_commit: None,
        }
    }
}

/// Path to `update.json`. [`crate::update::STATE_ENV`] overrides the
/// directory so tests isolate state.
pub fn path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(crate::update::STATE_ENV) {
        return Some(PathBuf::from(dir).join("update.json"));
    }
    Some(dirs::data_dir()?.join("tonk").join("update.json"))
}

/// Load state; missing or corrupt means defaults.
pub fn load() -> State {
    let Some(path) = path() else {
        return State::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Persist state, creating the parent directory if needed.
pub fn store(state: &State) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(state).unwrap_or_default(),
    )
}

/// Seconds since `stamp`, or `None` if absent or unparseable.
///
/// Unparseable is deliberately `None` ("never seen") rather than an
/// error: it fails toward checking, not toward permanent silence.
fn elapsed_secs(stamp: Option<&String>, now: DateTime<Utc>) -> Option<i64> {
    let parsed = DateTime::parse_from_rfc3339(stamp?).ok()?;
    Some((now - parsed.with_timezone(&Utc)).num_seconds())
}

/// Whether the background check should run now.
pub fn should_check(state: &State, now: DateTime<Utc>) -> bool {
    if !state.check_enabled {
        return false;
    }
    match elapsed_secs(state.last_checked_at.as_ref(), now) {
        None => true,
        Some(elapsed) => elapsed >= CHECK_INTERVAL_SECS,
    }
}

/// Whether the nag should print now, given this binary's version.
pub fn should_nag(state: &State, local_version: &str, now: DateTime<Utc>) -> bool {
    if !state.check_enabled {
        return false;
    }
    let Some(latest) = state.latest_version.as_deref() else {
        return false;
    };
    if !crate::update::is_newer(local_version, latest) {
        return false;
    }
    match elapsed_secs(state.last_nagged_at.as_ref(), now) {
        None => true,
        Some(elapsed) => elapsed >= NAG_INTERVAL_SECS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(stamp: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(stamp)
            .expect("parse")
            .with_timezone(&Utc)
    }

    #[dialog_common::test]
    fn it_checks_when_never_checked_before() {
        assert!(should_check(&State::default(), at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_skips_the_check_within_the_interval() {
        let state = State {
            last_checked_at: Some("2026-07-16T00:00:00Z".to_owned()),
            ..State::default()
        };
        assert!(!should_check(&state, at("2026-07-16T12:00:00Z")));
    }

    #[dialog_common::test]
    fn it_checks_again_after_the_interval() {
        let state = State {
            last_checked_at: Some("2026-07-16T00:00:00Z".to_owned()),
            ..State::default()
        };
        assert!(should_check(&state, at("2026-07-17T00:00:01Z")));
    }

    #[dialog_common::test]
    fn it_never_checks_when_disabled() {
        let state = State {
            check_enabled: false,
            ..State::default()
        };
        assert!(!should_check(&state, at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_checks_when_the_timestamp_is_unparseable() {
        let state = State {
            last_checked_at: Some("nonsense".to_owned()),
            ..State::default()
        };
        assert!(should_check(&state, at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_nags_when_a_newer_version_is_cached() {
        let state = State {
            latest_version: Some("0.5.0".to_owned()),
            ..State::default()
        };
        assert!(should_nag(&state, "0.4.0", at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_does_not_nag_when_current() {
        let state = State {
            latest_version: Some("0.4.0".to_owned()),
            ..State::default()
        };
        assert!(!should_nag(&state, "0.4.0", at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_does_not_nag_before_nothing_was_checked() {
        assert!(!should_nag(
            &State::default(),
            "0.4.0",
            at("2026-07-16T00:00:00Z")
        ));
    }

    #[dialog_common::test]
    fn it_does_not_nag_twice_within_the_interval() {
        let state = State {
            latest_version: Some("0.5.0".to_owned()),
            last_nagged_at: Some("2026-07-16T00:00:00Z".to_owned()),
            ..State::default()
        };
        assert!(!should_nag(&state, "0.4.0", at("2026-07-16T12:00:00Z")));
        assert!(should_nag(&state, "0.4.0", at("2026-07-17T00:00:01Z")));
    }

    #[dialog_common::test]
    fn it_never_nags_when_disabled() {
        let state = State {
            check_enabled: false,
            latest_version: Some("0.5.0".to_owned()),
            ..State::default()
        };
        assert!(!should_nag(&state, "0.4.0", at("2026-07-16T00:00:00Z")));
    }

    #[dialog_common::test]
    fn it_round_trips_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: tests in this mod run on one thread per process
        // invocation; nothing else reads this var concurrently.
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        let state = State {
            check_enabled: false,
            latest_version: Some("0.5.0".to_owned()),
            ..State::default()
        };
        store(&state).expect("store");
        assert_eq!(load(), state);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_defaults_to_enabled_when_state_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        assert!(load().check_enabled);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }
}
