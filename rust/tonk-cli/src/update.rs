//! Self-update for `install.sh` installs: an explicit `tonk update`
//! and a once-a-day check that nags when a newer release exists.
//!
//! Both release channels are rolling tags, so the version string
//! cannot identify a build. Releases publish a `manifest.json`
//! carrying version + commit: the nag compares versions (a real
//! update bumps it, staging churn does not), `tonk update` compares
//! commits (catching same-version rebuilds).

use anyhow::Context as _;

pub mod fetch;
pub mod manifest;
pub mod receipt;
pub mod state;
pub mod swap;

/// Opt out of the background check for one run or an environment.
/// Named to match `TONK_NO_SYNC` in [`crate::auto_sync`].
pub const NO_CHECK_ENV: &str = "TONK_NO_UPDATE_CHECK";

/// Overrides the directory holding `update.json` and `install.json`,
/// so tests isolate state. Matches `TONK_TELEMETRY_STATE`.
pub const STATE_ENV: &str = "TONK_UPDATE_STATE";

/// Overrides the release base URL, so tests serve a fake release.
/// Matches `TONK_POSTHOG_ENDPOINT`.
pub const ENDPOINT_ENV: &str = "TONK_UPDATE_ENDPOINT";

/// Where releases live when [`ENDPOINT_ENV`] is unset.
pub const DEFAULT_ENDPOINT: &str = "https://github.com/tonk-labs/tonk/releases";

/// A release channel, as `install.sh` understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The `tonk-latest` rolling release, built from `stable`.
    Stable,
    /// The `tonk-staging` rolling pre-release, built from `staging`.
    Staging,
}

impl Channel {
    /// Label as it appears in the receipt and `TONK_CHANNEL`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Staging => "staging",
        }
    }

    /// Parse a channel label; `None` for anything unrecognized.
    pub fn from_label(label: &str) -> Option<Channel> {
        match label {
            "stable" => Some(Channel::Stable),
            "staging" => Some(Channel::Staging),
            _ => None,
        }
    }

    /// Directory URL holding this channel's assets. Stable resolves
    /// through GitHub's `/latest/` alias; staging names its tag —
    /// the same two shapes `install.sh` builds.
    pub fn base_url(self, endpoint: &str) -> String {
        let endpoint = endpoint.trim_end_matches('/');
        match self {
            Channel::Stable => format!("{endpoint}/latest/download"),
            Channel::Staging => format!("{endpoint}/download/tonk-staging"),
        }
    }
}

/// Release base URL: [`ENDPOINT_ENV`] if set, else [`DEFAULT_ENDPOINT`].
pub fn endpoint() -> String {
    std::env::var(ENDPOINT_ENV).unwrap_or_else(|_| DEFAULT_ENDPOINT.to_owned())
}

/// Whether `remote` is a strictly newer release than `local`.
///
/// Semver `>`, not `!=`: a version ahead of the channel (a
/// `TONK_RELEASE` pin) must not be nagged into a downgrade. An
/// unparseable version on either side means "no update" — a nag is
/// never worth acting on a guess.
pub fn is_newer(local: &str, remote: &str) -> bool {
    let (Ok(local), Ok(remote)) = (
        semver::Version::parse(local),
        semver::Version::parse(remote),
    ) else {
        return false;
    };
    remote > local
}

/// Which channel this copy tracks: the receipt, else `TONK_CHANNEL`,
/// else stable.
///
/// The receipt wins because it records what was actually installed;
/// `TONK_CHANNEL` is what `install.sh` reads, so honouring it lets a
/// receipt-less copy still be checked against the right release.
pub fn resolve_channel() -> Channel {
    if let Some(channel) = receipt::load().and_then(|receipt| Channel::from_label(&receipt.channel))
    {
        return channel;
    }
    std::env::var("TONK_CHANNEL")
        .ok()
        .and_then(|label| Channel::from_label(&label))
        .unwrap_or(Channel::Stable)
}

/// Handle `tonk update`. Returns the line to print on success.
///
/// `set_check` toggles the background check and returns without
/// touching the binary — `--disable-check` / `--enable-check`.
///
/// Unlike the background check, every failure here is loud: the user
/// asked for this, so a silent no-op would be a lie.
pub async fn run(set_check: Option<bool>) -> anyhow::Result<String> {
    if let Some(enabled) = set_check {
        let mut state = state::load();
        state.check_enabled = enabled;
        state::store(&state).context("could not persist the update-check setting")?;
        return Ok(match enabled {
            true => "update check enabled".to_owned(),
            false => "update check disabled".to_owned(),
        });
    }

    let channel = resolve_channel();
    let platform = manifest::platform().with_context(|| {
        format!(
            "no published tonk build for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let remote = fetch::manifest(channel).await?;
    let local_version = env!("CARGO_PKG_VERSION");

    // Commit, not version: on a rolling channel the same version can
    // be many builds, and someone who typed `tonk update` wants the
    // build the channel is actually serving.
    if receipt::load().is_some_and(|receipt| receipt.commit == remote.commit) {
        return Ok(format!(
            "already current: {local_version} ({}, {})",
            short_commit(&remote.commit),
            channel.as_str()
        ));
    }

    let asset = manifest::asset_name(platform);
    let checksums = fetch::checksums(channel).await?;
    let expected = manifest::parse_checksums(&checksums, &asset).with_context(|| {
        format!(
            "no checksum entry for {asset} on the {} channel",
            channel.as_str()
        )
    })?;

    let target = running_binary()?;
    let archive = fetch::archive(channel, &asset).await?;
    swap::install(&archive, &expected, &target)?;

    let install_dir = target
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_string_lossy()
        .into_owned();
    let stored = receipt::store(&receipt::Receipt {
        channel: channel.as_str().to_owned(),
        version: remote.version.clone(),
        commit: remote.commit.clone(),
        install_dir,
        installed_at: chrono::Utc::now().to_rfc3339(),
    });
    // The binary is already swapped; a receipt we couldn't write is
    // worth a warning, not a failure that implies the update failed.
    if let Err(err) = stored {
        eprintln!("warning: updated, but could not write the install receipt: {err}");
    }

    // A fresh check next run rather than a stale "newer available".
    let mut state = state::load();
    state.latest_version = Some(remote.version.clone());
    state.latest_commit = Some(remote.commit.clone());
    let _ = state::store(&state);

    Ok(format!(
        "updated {} -> {} ({}, {})",
        local_version,
        remote.version,
        short_commit(&remote.commit),
        channel.as_str()
    ))
}

/// First 7 characters of a git SHA, as git itself abbreviates.
pub fn short_commit(commit: &str) -> &str {
    let end = commit.len().min(7);
    &commit[..end]
}

/// The binary to replace: the running one, symlinks resolved.
///
/// Canonicalized because the foreign-install guard matches on the
/// path. A nix profile puts a symlink at `~/.nix-profile/bin/tonk`
/// pointing into `/nix/store`; without resolving it the guard would
/// miss the store prefix and rename over nix's symlink. An
/// `install.sh` install is a real file, so this is a no-op there.
/// If canonicalization fails, fall back to the raw path rather than
/// refusing to update.
fn running_binary() -> anyhow::Result<std::path::PathBuf> {
    let path = std::env::current_exe().context("could not locate the running tonk binary")?;
    Ok(std::fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_reports_a_higher_version_as_newer() {
        assert!(is_newer("0.4.0", "0.5.0"));
        assert!(is_newer("0.4.0", "0.4.1"));
    }

    #[dialog_common::test]
    fn it_does_not_report_an_equal_version_as_newer() {
        assert!(!is_newer("0.4.0", "0.4.0"));
    }

    #[dialog_common::test]
    fn it_does_not_report_an_older_version_as_newer() {
        // A TONK_RELEASE pin to something ahead of stable must never
        // be nagged into a downgrade.
        assert!(!is_newer("0.5.0", "0.4.0"));
    }

    #[dialog_common::test]
    fn it_treats_a_prerelease_as_older_than_its_release() {
        assert!(is_newer("0.4.1-rc.1", "0.4.1"));
        assert!(!is_newer("0.4.1", "0.4.1-rc.1"));
    }

    #[dialog_common::test]
    fn it_reports_unparseable_versions_as_not_newer() {
        assert!(!is_newer("not-a-version", "0.4.0"));
        assert!(!is_newer("0.4.0", "not-a-version"));
    }

    #[dialog_common::test]
    fn it_builds_channel_urls_matching_the_installer() {
        assert_eq!(
            Channel::Stable.base_url("https://example.test/releases"),
            "https://example.test/releases/latest/download"
        );
        assert_eq!(
            Channel::Staging.base_url("https://example.test/releases"),
            "https://example.test/releases/download/tonk-staging"
        );
    }

    #[dialog_common::test]
    fn it_reads_channel_labels() {
        assert_eq!(Channel::from_label("staging"), Some(Channel::Staging));
        assert_eq!(Channel::from_label("stable"), Some(Channel::Stable));
        assert_eq!(Channel::from_label("nonsense"), None);
        assert_eq!(Channel::Staging.as_str(), "staging");
    }

    #[dialog_common::test]
    fn it_resolves_the_channel_from_receipt_then_env_then_stable() {
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: tests in this mod run on one thread per process
        // invocation; nothing else reads these vars concurrently.
        unsafe { std::env::set_var(STATE_ENV, dir.path()) };
        unsafe { std::env::remove_var("TONK_CHANNEL") };

        // No receipt, no env: stable.
        assert_eq!(resolve_channel(), Channel::Stable);

        // Env alone is honoured.
        unsafe { std::env::set_var("TONK_CHANNEL", "staging") };
        assert_eq!(resolve_channel(), Channel::Staging);

        // A receipt wins over the env.
        receipt::store(&receipt::Receipt {
            channel: "stable".to_owned(),
            version: "0.4.0".to_owned(),
            commit: "abc".to_owned(),
            install_dir: "/usr/local/bin".to_owned(),
            installed_at: "2026-07-16T00:00:00Z".to_owned(),
        })
        .expect("store");
        assert_eq!(resolve_channel(), Channel::Stable);

        unsafe { std::env::remove_var("TONK_CHANNEL") };
        unsafe { std::env::remove_var(STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_abbreviates_a_commit_without_panicking_on_short_input() {
        assert_eq!(short_commit("27b74e22b1234567"), "27b74e2");
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }
}
