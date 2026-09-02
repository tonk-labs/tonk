//! Anonymous usage telemetry (PostHog).
//!
//! One `cli_command_run` event per invocation: command name,
//! duration, exit class — never document content, paths, or URLs.
//! Opt-out model: a first-run notice on stderr, `tonk telemetry off`,
//! `TONK_TELEMETRY=0`, or `DO_NOT_TRACK=1`. Without a build-time or
//! runtime `TONK_POSTHOG_KEY` the whole module is a no-op.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ExitCode;

/// First-run disclosure, printed once to stderr.
pub const NOTICE: &str = "tonk collects anonymous usage data (command names, durations, \
success/failure — never your data) to improve the tool.\n\
Disable with `tonk telemetry off` or DO_NOT_TRACK=1. Details: docs/telemetry.md";

/// Persisted telemetry state. Lives in the platform data dir under
/// `tonk/`, deliberately outside the dialog profile directory so
/// `tonk identity --reset` doesn't wipe the user's choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Whether telemetry is enabled (opt-out model: defaults true).
    pub enabled: bool,
    /// Whether the first-run notice was already printed.
    pub notice_shown: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            notice_shown: false,
        }
    }
}

/// Path to `telemetry.json`. `TONK_TELEMETRY_STATE` overrides the
/// directory so tests can isolate state.
pub fn state_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("TONK_TELEMETRY_STATE") {
        return Some(PathBuf::from(dir).join("telemetry.json"));
    }
    Some(dirs::data_dir()?.join("tonk").join("telemetry.json"))
}

/// Load persisted settings; any missing/corrupt state means defaults.
pub fn load() -> Settings {
    let Some(path) = state_path() else {
        return Settings::default();
    };
    load_from(&path)
}

/// Persist settings, creating the parent directory if needed.
pub fn store(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    store_at(&path, settings)
}

/// [`load`] against an explicit file, with no environment lookup.
/// Tests drive this directly — a test that pointed the environment at
/// a temp dir would mutate process-global state that every other test
/// in the binary reads concurrently.
fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// [`store`] against an explicit file, with no environment lookup.
fn store_at(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(settings).unwrap_or_default(),
    )
}

/// In-flight record of one command invocation.
pub struct Recorder {
    client: tonk_analytics::native::Client,
    properties: Map<String, Value>,
}

/// Start recording one invocation. Returns `None` (skip everything)
/// when telemetry is off, opted out via env, or there's no API key.
/// Prints the first-run notice exactly once.
pub async fn begin(command: &'static str, subcommand: Option<&'static str>) -> Option<Recorder> {
    let settings = load();
    if !settings.enabled {
        return None;
    }
    let distinct = if crate::identity::exists() {
        match crate::identity::open().await {
            Ok(profile) => tonk_analytics::distinct_id(profile.did().as_ref()),
            Err(_) => "tonk:anonymous".to_owned(),
        }
    } else {
        "tonk:anonymous".to_owned()
    };
    let client = tonk_analytics::native::Client::from_env(distinct, true);
    if !client.is_enabled() {
        return None;
    }
    if !settings.notice_shown {
        eprintln!("{NOTICE}");
        let _ = store(&Settings {
            notice_shown: true,
            ..settings
        });
    }
    let mut properties = Map::new();
    properties.insert("command".to_owned(), Value::from(command));
    if let Some(sub) = subcommand {
        properties.insert("subcommand".to_owned(), Value::from(sub));
    }
    // Same dimension the web app registers as a posthog super
    // property; keeps all surfaces separable on one dashboard axis.
    properties.insert("environment".to_owned(), Value::from("cli"));
    Some(Recorder { client, properties })
}

impl Recorder {
    /// Queue canonical account events into this invocation's existing batch.
    pub fn account_events(
        &mut self,
        events: impl IntoIterator<Item = tonk_analytics::account::AccountEvent>,
    ) {
        for event in events {
            let _ = self.client.capture_account(&event);
        }
    }

    /// Attach one extra content-free property (flags, buckets, counts).
    pub fn property(&mut self, key: &str, value: impl Into<Value>) {
        self.properties.insert(key.to_owned(), value.into());
    }

    /// Capture the event and flush, capped at 300 ms so a slow or
    /// absent network never holds the command hostage.
    pub async fn finish(mut self, exit: ExitCode, duration: Duration) {
        self.properties
            .insert("success".to_owned(), Value::from(exit == ExitCode::Success));
        self.properties
            .insert("exit".to_owned(), Value::from(exit_label(exit)));
        self.properties.insert(
            "duration_ms".to_owned(),
            Value::from(duration.as_millis() as u64),
        );
        let properties = std::mem::take(&mut self.properties);
        self.client
            .capture(tonk_analytics::event::CLI_COMMAND_RUN, properties);
        self.client.flush(Duration::from_millis(300)).await;
    }
}

/// Coarse exit classification — the only error signal we send.
fn exit_label(exit: ExitCode) -> &'static str {
    match exit {
        ExitCode::Success => "success",
        ExitCode::ParseError => "parse-error",
        ExitCode::AnalyzeError => "analyze-error",
        ExitCode::CommitError => "commit-error",
        ExitCode::IoError => "io-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn settings_default_to_enabled_without_notice() {
        let settings = Settings::default();
        assert!(settings.enabled);
        assert!(!settings.notice_shown);
    }

    #[dialog_common::test]
    fn settings_round_trip_through_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("telemetry.json");
        store_at(
            &path,
            &Settings {
                enabled: false,
                notice_shown: true,
            },
        )
        .expect("store");
        let loaded = load_from(&path);
        assert!(!loaded.enabled);
        assert!(loaded.notice_shown);
    }

    #[dialog_common::test]
    fn settings_default_to_enabled_when_state_is_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("telemetry.json");
        std::fs::write(&path, "{ not json").expect("write");
        assert!(load_from(&path).enabled);
    }

    #[dialog_common::test]
    fn account_and_generic_events_share_one_batch_without_mixing_schemas() {
        use tonk_analytics::account::{
            AccountAction, AccountEvent, AccountOutcome, AccountState, Journey, Stage, Surface,
            Trigger,
        };
        let mut recorder = Recorder {
            client: tonk_analytics::native::Client::new(
                "http://localhost:1".to_owned(),
                "key".to_owned(),
                "tonk:anonymous".to_owned(),
            ),
            properties: Map::from_iter([
                ("command".to_owned(), Value::from("account")),
                ("subcommand".to_owned(), Value::from("login")),
                ("environment".to_owned(), Value::from("cli")),
            ]),
        };
        let start = AccountEvent::started(
            Journey::Login,
            AccountAction::Login,
            Stage::CallbackBind,
            Surface::NativeCli,
            Trigger::User,
            AccountState::None,
            "opaque-attempt",
        );
        let finish = AccountEvent::finished(
            Journey::Login,
            AccountAction::Login,
            Stage::Complete,
            Surface::NativeCli,
            Trigger::User,
            AccountState::Ready,
            "opaque-attempt",
            10,
            AccountOutcome::success(),
        );
        recorder.account_events([start, finish]);
        recorder.client.capture(
            tonk_analytics::event::CLI_COMMAND_RUN,
            recorder.properties.clone(),
        );
        let payload = recorder.client.payload().unwrap();
        let batch = payload["batch"].as_array().unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0]["event"], "account_event");
        assert_eq!(batch[2]["event"], "cli_command_run");
        assert!(batch[2]["properties"].get("stage").is_none());
        assert!(batch[2]["properties"].get("failure_kind").is_none());
        let wire = payload.to_string();
        for sentinel in [
            "person@example.com",
            "did:key:zSensitive",
            "http://127.0.0.1/callback",
            "delegation-secret",
            "/Users/person/private",
        ] {
            assert!(!wire.contains(sentinel));
        }
    }
}
