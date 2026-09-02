#![warn(missing_docs)]
//! Shared PostHog analytics for Tonk: one event vocabulary for the
//! web app and the CLI, plus a capture client per target.
//!
//! - Native (CLI): [`native::Client`] queues events and POSTs one
//!   `/batch` request, best-effort within a caller-supplied timeout.
//! - Browser (web app): `web` bridges to the self-hosted posthog-js
//!   bundle loaded by `tonk-ui`'s `index.html`.
//!
//! Everything here is content-free by construction: identifiers are
//! hashed ([`distinct_id`], [`anonymize`]) and route paths are
//! normalized ([`normalize_path`]) before they leave the machine.

use sha2_0_10::{Digest, Sha256};

pub mod account;
pub mod launch;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

/// Event names shared by every Tonk surface. Keep dashboards coherent:
/// never capture a string literal, always one of these.
pub mod event {
    /// Typed account journey lifecycle shared by web and CLI.
    pub const ACCOUNT: &str = "account_event";
    /// One CLI invocation: command, subcommand, duration, outcome.
    pub const CLI_COMMAND_RUN: &str = "cli_command_run";
    /// The web shell finished booting (service worker controlling).
    pub const APP_LOADED: &str = "app_loaded";
    /// PostHog's built-in pageview event, captured manually with a
    /// normalized route.
    pub const PAGEVIEW: &str = "$pageview";
    /// A local commit landed (web; from the worker's
    /// `tonk:local-commit` broadcast).
    pub const COMMIT: &str = "commit";
    /// A workspace sheet tab was activated.
    pub const SHEET_ACTIVATED: &str = "sheet_activated";
    /// A wasm panic reached the panic hook.
    pub const PANIC: &str = "panic";
    /// A new browser analytics session entered Tonk.
    pub const VISIT: &str = "visit";
    /// Account creation completed, including enrollment when configured.
    pub const ACCOUNT_CREATED: &str = "account_created";
    /// A space was successfully created or joined.
    pub const SPACE_CONVERSION: &str = "space_conversion";
    /// An invite was successfully minted for a space.
    pub const SPACE_SHARED: &str = "space_shared";
}

/// Default PostHog ingestion host (EU cloud).
pub const DEFAULT_HOST: &str = "https://eu.i.posthog.com";

/// Stable, anonymous PostHog `distinct_id` for a profile DID:
/// `tonk:<sha256-hex>`. The raw DID never leaves the machine; hashing
/// is deterministic so CLI and web sessions of one profile correlate.
pub fn distinct_id(did: &str) -> String {
    format!("tonk:{}", hex::encode(Sha256::digest(did.as_bytes())))
}

/// Short stable token for an identifier-ish value (space name, entity,
/// branch): first 8 bytes of its SHA-256, hex-encoded (16 chars).
pub fn anonymize(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..8])
}

/// Route literals kept verbatim in [`normalize_path`]. Everything else
/// in a path is user data and gets [`anonymize`]d.
const ROUTE_LITERALS: &[&str] = &[
    "space", "view", "board", "profile", "join", "branch", "concept", "display",
];

/// Normalize a document path for analytics: known route literals stay,
/// every other segment is replaced by its [`anonymize`] token. `""`
/// and `"/"` normalize to `"/"`.
pub fn normalize_path(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        if ROUTE_LITERALS.contains(&segment) {
            out.push_str(segment);
        } else {
            out.push_str(&anonymize(segment));
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// Whether the environment opts out of telemetry: `DO_NOT_TRACK` set
/// to anything non-empty except `0`, or `TONK_TELEMETRY` set to `0`,
/// `false`, or `off`. Takes a lookup closure so it's testable without
/// mutating process env.
pub fn env_opt_out(var: impl Fn(&str) -> Option<String>) -> bool {
    if let Some(value) = var("DO_NOT_TRACK")
        && !value.is_empty()
        && value != "0"
    {
        return true;
    }
    matches!(
        var("TONK_TELEMETRY").as_deref(),
        Some("0") | Some("false") | Some("off")
    )
}

/// The PostHog project API key: runtime `TONK_POSTHOG_KEY` env var
/// first (tests, local overrides), then the value baked in at compile
/// time. `None` disables analytics entirely.
pub fn api_key() -> Option<String> {
    match std::env::var("TONK_POSTHOG_KEY") {
        Ok(value) if !value.is_empty() => Some(value),
        _ => option_env!("TONK_POSTHOG_KEY")
            .map(str::to_owned)
            .filter(|value| !value.is_empty()),
    }
}

/// The PostHog ingestion host: runtime `TONK_POSTHOG_ENDPOINT` (tests)
/// or `TONK_POSTHOG_HOST`, then the compile-time `TONK_POSTHOG_HOST`,
/// then [`DEFAULT_HOST`].
pub fn host() -> String {
    for var in ["TONK_POSTHOG_ENDPOINT", "TONK_POSTHOG_HOST"] {
        if let Ok(value) = std::env::var(var)
            && !value.is_empty()
        {
            return value;
        }
    }
    option_env!("TONK_POSTHOG_HOST")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| DEFAULT_HOST.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn distinct_id_is_stable_and_prefixed() {
        let id = distinct_id("did:key:z6MkExample");
        assert_eq!(id, distinct_id("did:key:z6MkExample"));
        assert!(id.starts_with("tonk:"));
        assert_eq!(id.len(), 5 + 64);
        assert!(!id.contains("z6Mk"));
    }

    #[dialog_common::test]
    fn anonymize_is_short_and_content_free() {
        let token = anonymize("my-secret-space");
        assert_eq!(token.len(), 16);
        assert!(!token.contains("secret"));
        assert_ne!(token, anonymize("other-space"));
    }

    #[dialog_common::test]
    fn normalize_path_keeps_literals_and_hashes_the_rest() {
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("/profile"), "/profile");
        let normalized = normalize_path("/space/main@notes:abc123/view/xyz");
        assert!(normalized.starts_with("/space/"));
        assert!(normalized.contains("/view/"));
        assert!(!normalized.contains("notes"));
        assert!(!normalized.contains("xyz"));
        assert_eq!(
            normalized,
            format!(
                "/space/{}/view/{}",
                anonymize("main@notes:abc123"),
                anonymize("xyz")
            )
        );
    }

    #[dialog_common::test]
    fn env_opt_out_honors_do_not_track_and_tonk_telemetry() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(!env_opt_out(env(&[])));
        assert!(!env_opt_out(env(&[("DO_NOT_TRACK", "0")])));
        assert!(!env_opt_out(env(&[("DO_NOT_TRACK", "")])));
        assert!(env_opt_out(env(&[("DO_NOT_TRACK", "1")])));
        assert!(env_opt_out(env(&[("TONK_TELEMETRY", "0")])));
        assert!(env_opt_out(env(&[("TONK_TELEMETRY", "off")])));
        assert!(!env_opt_out(env(&[("TONK_TELEMETRY", "1")])));
    }
}
