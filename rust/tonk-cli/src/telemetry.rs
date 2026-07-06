//! Anonymous usage telemetry (PostHog).
//!
//! One `cli_command_run` event per invocation: command name,
//! duration, exit class — never document content, paths, or URLs.
//! Opt-out model: a first-run notice on stderr, `tonk telemetry off`,
//! `TONK_TELEMETRY=0`, or `DO_NOT_TRACK=1`. Without a build-time or
//! runtime `TONK_POSTHOG_KEY` the whole module is a no-op.

use std::path::PathBuf;
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
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Persist settings, creating the parent directory if needed.
pub fn store(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
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
        // SAFETY: tests in this mod run on one thread per process
        // invocation; nothing else reads this var concurrently.
        unsafe { std::env::set_var("TONK_TELEMETRY_STATE", dir.path()) };
        let settings = Settings {
            enabled: false,
            notice_shown: true,
        };
        store(&settings).expect("store");
        let loaded = load();
        assert!(!loaded.enabled);
        assert!(loaded.notice_shown);
        unsafe { std::env::remove_var("TONK_TELEMETRY_STATE") };
    }
}
