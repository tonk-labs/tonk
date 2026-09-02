//! Self-update for `install.sh` installs: an explicit `tonk update`
//! and a once-a-day check that nags when a newer release exists.
//!
//! Self-update follows the channel recorded by `install.sh` for the
//! running copy, defaulting safely to stable. Releases publish a
//! `manifest.json` carrying version + commit: the nag compares versions
//! (a real update bumps it, staging churn does not), `tonk update`
//! compares commits (catching same-version rebuilds).

use anyhow::{Context as _, bail};

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
    /// Parse the exact labels persisted by `install.sh`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stable" => Some(Channel::Stable),
            "staging" => Some(Channel::Staging),
            _ => None,
        }
    }

    /// Label as it appears in the receipt and `TONK_CHANNEL`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Staging => "staging",
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

/// Select the release channel for this installed copy.
///
/// A receipt can steer updates only when it describes the directory of
/// the running binary. Missing, corrupt, unknown, or stale receipts fail
/// toward stable rather than moving an unrelated copy onto prereleases.
pub fn resolve_channel(receipt: Option<&receipt::Receipt>, install_dir: &str) -> Channel {
    receipt
        .filter(|receipt| receipt.install_dir == install_dir)
        .and_then(|receipt| Channel::parse(&receipt.channel))
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

    // Resolved and guarded up front, before any network call: the
    // design says the npm/nix rule is enforced by where we actually
    // live, not by trusting a receipt, so the guard must run before
    // the "already current" shortcut below could return early and
    // before the ~10MB archive download that would otherwise precede
    // `swap::install`'s own check. `swap::install` keeps its check too,
    // as defense in depth.
    let target = running_binary()?;
    if let Some(foreign) = swap::foreign_install(&target) {
        bail!("{}", foreign.refusal(&target));
    }
    let install_dir = target_dir(&target);

    let receipt = receipt::load();
    let channel = resolve_channel(receipt.as_ref(), &install_dir);
    let platform = manifest::platform().with_context(|| {
        format!(
            "no published tonk build for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let remote = fetch::manifest(channel).await?;
    if remote.channel != channel.as_str() {
        bail!(
            "release manifest says channel {}, but this install selected {}; refusing to update",
            remote.channel,
            channel.as_str()
        );
    }
    let local_version = env!("CARGO_PKG_VERSION");

    // Commit, not version: on a rolling channel the same version can
    // be many builds, and someone who typed `tonk update` wants the
    // build the channel is actually serving. Gated on install_dir too:
    // a receipt only describes the copy that wrote it, so two
    // `install.sh` copies (or a stale receipt beside an npm/nix copy)
    // must not share one "already current" answer. A false mismatch
    // just costs a redundant reinstall — the safe direction, versus
    // wrongly claiming current.
    if receipt.is_some_and(|receipt| {
        receipt.commit == remote.commit && receipt.install_dir == install_dir
    }) {
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

    let archive = fetch::archive(channel, &asset).await?;
    swap::install(&archive, &expected, &target)?;

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

/// How long the background check may take before we give up on it.
/// It runs concurrently with the 300ms telemetry flush, so the worst
/// case is roughly this once a day, not on every command.
///
/// The budget has to cover native-root loading, DNS, TCP, TLS, and the
/// GET itself on a freshly built reqwest client — not just the request
/// on an already-warm connection. It is deliberately not tighter than
/// 5s: on timeout `last_checked_at` still advances (so an offline
/// machine backs off to a daily retry, which is correct), but a
/// machine that is merely slower than the timeout would back off
/// forever and never once complete the check — silently losing the
/// nag for good rather than just running it late. The human chose 5s
/// over an initial 2s for exactly this reason.
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether the background check is allowed to run at all.
fn check_permitted() -> bool {
    // CI is noise, not an audience: nobody acts on a nag there.
    if std::env::var_os("CI").is_some() {
        return false;
    }
    if std::env::var_os(NO_CHECK_ENV).is_some() {
        return false;
    }
    true
}

/// Refresh the cached release info if it is stale.
///
/// Deliberately silent on every failure — offline, DNS, rate limit,
/// 404. Nothing depends on the result and the user did not ask for
/// it, so a failure here must never print or change an exit code.
/// `last_checked_at` still advances, so an offline machine retries
/// daily rather than on every command.
pub async fn check() {
    if !check_permitted() {
        return;
    }
    let mut state = state::load();
    if !state::should_check(&state, chrono::Utc::now()) {
        return;
    }
    let channel = running_binary()
        .ok()
        .map(|target| {
            let install_dir = target_dir(&target);
            let receipt = receipt::load();
            resolve_channel(receipt.as_ref(), &install_dir)
        })
        .unwrap_or(Channel::Stable);
    let fetched = tokio::time::timeout(CHECK_TIMEOUT, fetch::manifest(channel)).await;
    state.last_checked_at = Some(chrono::Utc::now().to_rfc3339());
    if let Ok(Ok(manifest)) = fetched
        && manifest.channel == channel.as_str()
    {
        state.latest_version = Some(manifest.version);
        state.latest_commit = Some(manifest.commit);
    }
    let _ = state::store(&state);
}

/// Print the nag if the cache says a newer version exists.
///
/// Reads the cache rather than an in-flight check, so a check that
/// misses the exit window just shifts the nag one invocation later.
/// stderr, always: agents parse this CLI's stdout.
pub fn nag() {
    if !check_permitted() {
        return;
    }
    let mut state = state::load();
    let now = chrono::Utc::now();
    if !state::should_nag(&state, env!("CARGO_PKG_VERSION"), now) {
        return;
    }
    let Some(latest) = state.latest_version.clone() else {
        return;
    };
    eprintln!(
        "tonk {latest} is available (you have {}) — run `tonk update`",
        env!("CARGO_PKG_VERSION")
    );
    state.last_nagged_at = Some(now.to_rfc3339());
    let _ = state::store(&state);
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

/// `target`'s parent directory as stored in [`receipt::Receipt::install_dir`].
fn target_dir(target: &std::path::Path) -> String {
    target
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_string_lossy()
        .into_owned()
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
        // A pinned install ahead of staging must never
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
    fn it_names_channels() {
        assert_eq!(Channel::Stable.as_str(), "stable");
        assert_eq!(Channel::Staging.as_str(), "staging");
    }

    #[dialog_common::test]
    fn it_parses_known_channels_exactly() {
        assert_eq!(Channel::parse("stable"), Some(Channel::Stable));
        assert_eq!(Channel::parse("staging"), Some(Channel::Staging));
        assert_eq!(Channel::parse("Stable"), None);
        assert_eq!(Channel::parse("nightly"), None);
    }

    fn sample_receipt(channel: &str, install_dir: &str) -> receipt::Receipt {
        receipt::Receipt {
            channel: channel.to_owned(),
            version: "0.6.11".to_owned(),
            commit: "abc1234".to_owned(),
            install_dir: install_dir.to_owned(),
            installed_at: "2026-09-02T00:00:00Z".to_owned(),
        }
    }

    #[dialog_common::test]
    fn it_resolves_the_channel_from_a_matching_receipt() {
        let stable = sample_receipt("stable", "/opt/tonk");
        let staging = sample_receipt("staging", "/opt/tonk");
        assert_eq!(resolve_channel(Some(&stable), "/opt/tonk"), Channel::Stable);
        assert_eq!(
            resolve_channel(Some(&staging), "/opt/tonk"),
            Channel::Staging
        );
    }

    #[dialog_common::test]
    fn it_defaults_to_stable_for_missing_unknown_or_mismatched_receipts() {
        let unknown = sample_receipt("nightly", "/opt/tonk");
        let other_copy = sample_receipt("staging", "/other/tonk");
        assert_eq!(resolve_channel(None, "/opt/tonk"), Channel::Stable);
        assert_eq!(
            resolve_channel(Some(&unknown), "/opt/tonk"),
            Channel::Stable
        );
        assert_eq!(
            resolve_channel(Some(&other_copy), "/opt/tonk"),
            Channel::Stable
        );
    }

    #[dialog_common::test]
    fn it_abbreviates_a_commit_without_panicking_on_short_input() {
        assert_eq!(short_commit("27b74e22b1234567"), "27b74e2");
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }
}
