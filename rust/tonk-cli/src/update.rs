//! Self-update for `install.sh` installs: an explicit `tonk update`
//! and a once-a-day check that nags when a newer release exists.
//!
//! Both release channels are rolling tags, so the version string
//! cannot identify a build. Releases publish a `manifest.json`
//! carrying version + commit: the nag compares versions (a real
//! update bumps it, staging churn does not), `tonk update` compares
//! commits (catching same-version rebuilds).

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
    channel_from(
        receipt::load()
            .as_ref()
            .map(|receipt| receipt.channel.as_str()),
        std::env::var("TONK_CHANNEL").ok().as_deref(),
    )
}

/// The precedence [`resolve_channel`] applies, over labels the caller
/// has already read. Split out so it can be tested without mutating
/// process-global environment state that every other test in the
/// binary reads concurrently.
fn channel_from(receipt_label: Option<&str>, env_label: Option<&str>) -> Channel {
    receipt_label
        .and_then(Channel::from_label)
        .or_else(|| env_label.and_then(Channel::from_label))
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
    // build the channel is actually serving. Gated on install_dir too:
    // a receipt only describes the copy that wrote it, so two
    // `install.sh` copies (or a stale receipt beside an npm/nix copy)
    // must not share one "already current" answer. A false mismatch
    // just costs a redundant reinstall — the safe direction, versus
    // wrongly claiming current.
    if receipt::load().is_some_and(|receipt| {
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
    let fetched = tokio::time::timeout(CHECK_TIMEOUT, fetch::manifest(resolve_channel())).await;
    state.last_checked_at = Some(chrono::Utc::now().to_rfc3339());
    if let Ok(Ok(manifest)) = fetched {
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
        // Neither source: stable.
        assert_eq!(channel_from(None, None), Channel::Stable);

        // Env alone is honoured.
        assert_eq!(channel_from(None, Some("staging")), Channel::Staging);

        // A receipt wins over the env.
        assert_eq!(
            channel_from(Some("stable"), Some("staging")),
            Channel::Stable
        );
    }

    #[dialog_common::test]
    fn it_falls_through_an_unrecognized_receipt_channel_to_env_then_stable() {
        // A receipt channel label of "beta" is unrecognized: it must
        // not win, falling through to the env and then to stable.
        assert_eq!(channel_from(Some("beta"), None), Channel::Stable);
        assert_eq!(
            channel_from(Some("beta"), Some("staging")),
            Channel::Staging
        );
    }

    #[dialog_common::test]
    fn it_abbreviates_a_commit_without_panicking_on_short_input() {
        assert_eq!(short_commit("27b74e22b1234567"), "27b74e2");
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }
}
