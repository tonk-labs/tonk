//! Cached result of the release check (`update.json`), and the
//! cadence rules for when to check and when to nag.
//!
//! The nag prints from this cache, never from an in-flight check: a
//! check that misses the exit window just shifts the nag one
//! invocation later, so nothing has to finish in time for anything to
//! be correct.

use std::path::{Path, PathBuf};

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
    /// Release channel that produced the cached release information.
    #[serde(default)]
    pub channel: Option<String>,
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
            channel: None,
            last_checked_at: None,
            last_nagged_at: None,
            latest_version: None,
            latest_commit: None,
        }
    }
}

impl State {
    /// Select the active release channel, invalidating cached release
    /// information when it changes. The user's check preference is
    /// independent of the selected channel and is preserved.
    pub fn select_channel(&mut self, channel: &str) -> bool {
        if self.channel.as_deref() == Some(channel) {
            return false;
        }

        self.channel = Some(channel.to_owned());
        self.last_checked_at = None;
        self.last_nagged_at = None;
        self.latest_version = None;
        self.latest_commit = None;
        true
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
    load_from(&path)
}

/// Persist state, creating the parent directory if needed.
pub fn store(state: &State) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    store_at(&path, state)
}

/// [`load`] against an explicit file, with no environment lookup.
/// Tests drive this directly — a test that pointed the environment at
/// a temp dir would mutate process-global state that every other test
/// in the binary reads concurrently.
fn load_from(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// [`store`] against an explicit file, with no environment lookup.
fn store_at(path: &Path, state: &State) -> std::io::Result<()> {
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
        let path = dir.path().join("update.json");
        let state = State {
            check_enabled: false,
            latest_version: Some("0.5.0".to_owned()),
            ..State::default()
        };
        store_at(&path, &state).expect("store");
        assert_eq!(load_from(&path), state);
    }

    #[dialog_common::test]
    fn it_loads_state_written_before_channels_were_cached() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("update.json");
        std::fs::write(
            &path,
            r#"{
  "check_enabled": false,
  "last_checked_at": "2026-07-16T00:00:00Z",
  "last_nagged_at": null,
  "latest_version": "0.5.0",
  "latest_commit": "abc1234"
}"#,
        )
        .expect("write legacy state");

        let state = load_from(&path);
        assert!(!state.check_enabled);
        assert_eq!(state.channel, None);
        assert_eq!(state.latest_version.as_deref(), Some("0.5.0"));
    }

    #[dialog_common::test]
    fn it_invalidates_cached_release_information_when_the_channel_changes() {
        let mut state = State {
            check_enabled: false,
            channel: Some("staging".to_owned()),
            last_checked_at: Some("2026-07-16T00:00:00Z".to_owned()),
            last_nagged_at: Some("2026-07-16T00:00:00Z".to_owned()),
            latest_version: Some("99.0.0".to_owned()),
            latest_commit: Some("fff9999".to_owned()),
        };

        assert!(state.select_channel("stable"));
        assert!(!state.check_enabled);
        assert_eq!(state.channel.as_deref(), Some("stable"));
        assert_eq!(state.last_checked_at, None);
        assert_eq!(state.last_nagged_at, None);
        assert_eq!(state.latest_version, None);
        assert_eq!(state.latest_commit, None);
    }

    #[dialog_common::test]
    fn it_keeps_cached_release_information_when_the_channel_is_unchanged() {
        let mut state = State {
            channel: Some("stable".to_owned()),
            last_checked_at: Some("2026-07-16T00:00:00Z".to_owned()),
            latest_version: Some("0.5.0".to_owned()),
            ..State::default()
        };
        let before = state.clone();

        assert!(!state.select_channel("stable"));
        assert_eq!(state, before);
    }

    #[dialog_common::test]
    fn it_defaults_to_enabled_when_state_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_from(&dir.path().join("update.json")).check_enabled);
    }

    #[dialog_common::test]
    fn it_defaults_to_enabled_when_state_is_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("update.json");
        std::fs::write(&path, "{ not json").expect("write");
        assert!(load_from(&path).check_enabled);
    }
}
