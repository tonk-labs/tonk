# tonk CLI Self-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `tonk update` (explicit upgrade for `install.sh` installs) plus a once-a-day passive check that nags on stderr when a newer release exists.

**Architecture:** Both release channels are rolling tags, so the version string alone cannot identify a build. Releases grow a `manifest.json` carrying `version` + `commit` (from `GITHUB_SHA` — the nix build is untouched). The nag compares **versions** (a real update bumps it; staging churn does not); `tonk update` compares **commits** (catches same-version rebuilds). An install receipt records which channel to check. The update swap prepares a fully validated temp file — including a `--version` smoke test — and `rename()`s last, so no failure can leave a half-applied binary.

**Tech Stack:** Rust, clap, tokio (`current_thread`), reqwest (rustls w/ native roots), serde/serde_json, chrono, semver, tar + flate2, sha2, tempfile.

**Spec:** `docs/superpowers/specs/2026-07-16-cli-self-update-design.md`

## Global Constraints

- **Scope: the `install.sh` channel only.** npm (`node_modules`) and nix (`/nix/store`) installs are detected and refused with a pointer, never modified.
- **`checksums.txt` is never changed.** `install.sh` copies already in the wild verify against it; `manifest.json` is purely additive.
- **`install.sh` is served from the release, not the repo.** Old copies run forever. Every change to it must be additive and best-effort — an install must never fail because the manifest fetch 404'd.
- **The nag prints to stderr, never stdout.** Agents parse this CLI's stdout; a nag on stdout corrupts `--json` output.
- **The background check fails silently** — no message, no exit-code impact. It still bumps `last_checked_at` on failure so an offline machine retries daily, not per command.
- **`tonk update` fails loudly** — non-zero (`ExitCode::IoError`), a distinct message per failure mode.
- **The existing binary is untouched on every failure path.** The rename is always last.
- **No new `ExitCode` variant.** `ExitCode::IoError` (4) covers update failures; adding a variant would force a `telemetry::exit_label` change for no gain.
- **No `mod.rs`.** Use the `update.rs` + `update/` form (mirrors `data_ops.rs` + `data_ops/flags.rs`).
- **Tests use `#[dialog_common::test]`** and `it_does_x`-style names grouped by behaviour.
- **Env vars** (names fixed, follow existing precedent): `TONK_NO_UPDATE_CHECK` (opt-out, per `TONK_NO_SYNC` in `auto_sync.rs`), `TONK_UPDATE_STATE` (state dir override for tests, per `TONK_TELEMETRY_STATE`), `TONK_UPDATE_ENDPOINT` (release base URL override for tests, per `TONK_POSTHOG_ENDPOINT`), `TONK_CHANNEL` (existing, read by `install.sh`).
- **Default endpoint:** `https://github.com/tonk-labs/tonk/releases`
- **Release tags:** stable → `latest/download/…`; staging → `download/tonk-staging/…` (matches `install.sh`).
- **Assets:** `tonk-macos-arm64.tar.gz`, `tonk-linux-x86_64.tar.gz`. Published platforms are darwin/arm64 and linux/x86_64 only.
- **Nag/check interval:** 24h each.
- **Timeouts:** background check 2s; it runs concurrently with the existing 300ms telemetry flush.

## File Structure

| File | Responsibility |
|---|---|
| `rust/tonk-cli/src/update.rs` | Module root: `Channel`, `is_newer`, the `check`/`nag` entry points main calls |
| `rust/tonk-cli/src/update/manifest.rs` | `Manifest` type, platform slug, asset names, `checksums.txt` parsing |
| `rust/tonk-cli/src/update/receipt.rs` | `install.json` read/write |
| `rust/tonk-cli/src/update/state.rs` | `update.json` cache + check/nag cadence decisions |
| `rust/tonk-cli/src/update/fetch.rs` | HTTP: manifest, checksums, archive |
| `rust/tonk-cli/src/update/swap.rs` | Foreign-install guard, sha256 verify, extract, smoke-test, atomic rename |
| `rust/tonk-cli/src/bin/tonk.rs` | `tonk update` subcommand wiring; check+nag at exit |
| `rust/tonk-cli/tests/update.rs` | Integration: real binary against a local `TcpListener` release server |
| `install.sh` | Writes the receipt (additive, best-effort) |
| `.github/workflows/cli.yml` | Publishes + verifies `manifest.json` on both channels |

Pure logic (parsing, cadence, compares) lives apart from I/O so the bulk is unit-testable with no network and no filesystem. `swap.rs` is the risky half and gets real temp dirs.

---

### Task 1: Dependencies, platform detection, manifest parsing

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`, alphabetical)
- Modify: `rust/tonk-cli/Cargo.toml`
- Create: `rust/tonk-cli/src/update.rs`
- Create: `rust/tonk-cli/src/update/manifest.rs`
- Modify: `rust/tonk-cli/src/lib.rs`

**Interfaces:**
- Produces: `update::manifest::{Manifest, platform, asset_name, parse_checksums}`

- [ ] **Step 1: Add workspace dependencies**

In the root `Cargo.toml` `[workspace.dependencies]` block, add these entries in alphabetical position (`chrono`, `reqwest`, `sha2_0_10`, `tempfile` are already there — do not duplicate them):

```toml
flate2 = "1"
semver = "1"
tar = "0.4"
```

- [ ] **Step 2: Add crate dependencies**

In `rust/tonk-cli/Cargo.toml`, add to `[dependencies]` (alphabetical):

```toml
chrono = { workspace = true }
flate2 = { workspace = true }
semver = { workspace = true }
sha2_0_10 = { workspace = true }
tar = { workspace = true }
tempfile = { workspace = true }
```

`tempfile` is currently only in `[dev-dependencies]` — leave that entry alone; a dep in both sections is fine and Cargo unifies them.

Then update the `reqwest` comment in `[dependencies]`, which currently claims tonk never calls reqwest directly. Replace the comment block above `reqwest = { workspace = true }` with:

```toml
# `tonk update` calls reqwest directly to fetch release manifests and archives.
# The workspace enables `rustls-tls-native-roots` (see workspace Cargo.toml), so
# those downloads trust the OS CA store and work behind TLS-intercepting proxies.
reqwest = { workspace = true }
```

- [ ] **Step 3: Write the failing test**

Create `rust/tonk-cli/src/update/manifest.rs`:

```rust
//! Release identity: the `manifest.json` a release publishes next to
//! `checksums.txt`, plus the platform/asset naming shared with
//! `install.sh`.

use serde::{Deserialize, Serialize};

/// A release's self-description, published as `manifest.json`.
///
/// `built_at` is display-only — never parsed — so its format can
/// change without breaking older CLIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Cargo workspace version of the published binaries.
    pub version: String,
    /// Full git SHA the release was built from.
    pub commit: String,
    /// `stable` or `staging`.
    pub channel: String,
    /// RFC3339 build time, for humans reading the file.
    pub built_at: String,
}

/// Release asset slug for the host, or `None` where nothing is
/// published. Mirrors the `uname` mapping in `install.sh`.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

/// Archive name for a platform slug, as published on the release.
pub fn asset_name(platform: &str) -> String {
    format!("tonk-{platform}.tar.gz")
}

/// Pull one asset's SHA256 out of a `checksums.txt` body.
///
/// The file is `sha256sum` output: `<hex>  <name>`, where the name
/// may carry a `*` binary-mode marker.
pub fn parse_checksums(text: &str, asset: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset).then(|| hash.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_parses_a_checksum_for_the_named_asset() {
        let text = "aaa  tonk-macos-arm64.tar.gz\nbbb  tonk-linux-x86_64.tar.gz\n";
        assert_eq!(
            parse_checksums(text, "tonk-linux-x86_64.tar.gz"),
            Some("bbb".to_owned())
        );
    }

    #[dialog_common::test]
    fn it_parses_a_checksum_with_a_binary_mode_marker() {
        let text = "aaa *tonk-macos-arm64.tar.gz\n";
        assert_eq!(
            parse_checksums(text, "tonk-macos-arm64.tar.gz"),
            Some("aaa".to_owned())
        );
    }

    #[dialog_common::test]
    fn it_returns_none_when_the_asset_is_absent() {
        let text = "aaa  tonk-macos-arm64.tar.gz\n";
        assert_eq!(parse_checksums(text, "tonk-windows-x64.tar.gz"), None);
    }

    #[dialog_common::test]
    fn it_names_the_asset_for_a_platform() {
        assert_eq!(asset_name("macos-arm64"), "tonk-macos-arm64.tar.gz");
    }

    #[dialog_common::test]
    fn it_round_trips_a_manifest() {
        let json = r#"{"version":"0.4.0","commit":"abc","channel":"stable","built_at":"2026-07-16T00:00:00Z"}"#;
        let manifest: Manifest = serde_json::from_str(json).expect("parse");
        assert_eq!(manifest.version, "0.4.0");
        assert_eq!(manifest.commit, "abc");
    }
}
```

Create `rust/tonk-cli/src/update.rs`:

```rust
//! Self-update for `install.sh` installs: an explicit `tonk update`
//! and a once-a-day check that nags when a newer release exists.
//!
//! Both release channels are rolling tags, so the version string
//! cannot identify a build. Releases publish a `manifest.json`
//! carrying version + commit: the nag compares versions (a real
//! update bumps it, staging churn does not), `tonk update` compares
//! commits (catching same-version rebuilds).

pub mod manifest;
```

- [ ] **Step 4: Register the module**

In `rust/tonk-cli/src/lib.rs`, add to the `pub mod` list (alphabetical, between `transfer` and `views`):

```rust
pub mod update;
```

And add to the module doc-comment list, after the `- [`telemetry`]` line:

```rust
//! - [`update`] — self-update: `tonk update` plus the release check.
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::manifest)'`
Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock rust/tonk-cli/Cargo.toml rust/tonk-cli/src/update.rs rust/tonk-cli/src/update/manifest.rs rust/tonk-cli/src/lib.rs
git commit -m "feat(tonk-cli): release manifest and platform slugs for self-update"
```

---

### Task 2: Channel resolution and version compare

**Files:**
- Modify: `rust/tonk-cli/src/update.rs`

**Interfaces:**
- Consumes: `update::manifest`
- Produces: `update::{Channel, is_newer, NO_CHECK_ENV, STATE_ENV, ENDPOINT_ENV, DEFAULT_ENDPOINT}`; `Channel::{as_str, from_label, base_url}`

- [ ] **Step 1: Write the failing test**

Append to `rust/tonk-cli/src/update.rs`:

```rust
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::tests)'`
Expected: FAIL — `cannot find function is_newer`, `cannot find type Channel`.

- [ ] **Step 3: Write minimal implementation**

Insert into `rust/tonk-cli/src/update.rs`, after `pub mod manifest;` and before the `#[cfg(test)]` block:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::tests)'`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-cli/src/update.rs
git commit -m "feat(tonk-cli): channel resolution and semver compare for self-update"
```

---

### Task 3: The install receipt

**Files:**
- Create: `rust/tonk-cli/src/update/receipt.rs`
- Modify: `rust/tonk-cli/src/update.rs`

**Interfaces:**
- Consumes: `update::{Channel, STATE_ENV}`
- Produces: `update::receipt::{Receipt, path, load, store}`; `update::resolve_channel`

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-cli/src/update/receipt.rs`:

```rust
//! The install receipt (`install.json`): a snapshot of the release
//! manifest taken at install time, plus where it landed.
//!
//! `install.sh` writes it best-effort — a failed manifest fetch must
//! never fail an install, so a missing receipt is normal and means
//! "assume stable". It is not the basis of detection: it records
//! which channel to check, and lets `tonk update` answer "already
//! current" without downloading an archive to find out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What the last install put on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Channel label (`stable` / `staging`) this copy came from.
    pub channel: String,
    /// Version of the installed build.
    pub version: String,
    /// Git SHA of the installed build.
    pub commit: String,
    /// Directory the binary was installed into.
    pub install_dir: String,
    /// RFC3339 install time, for humans reading the file.
    pub installed_at: String,
}

/// Path to `install.json`. [`crate::update::STATE_ENV`] overrides the
/// directory so tests isolate state.
pub fn path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(crate::update::STATE_ENV) {
        return Some(PathBuf::from(dir).join("install.json"));
    }
    Some(dirs::data_dir()?.join("tonk").join("install.json"))
}

/// Load the receipt; missing or corrupt means `None`.
pub fn load() -> Option<Receipt> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist the receipt, creating the parent directory if needed.
pub fn store(receipt: &Receipt) -> std::io::Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(receipt).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Receipt {
        Receipt {
            channel: "staging".to_owned(),
            version: "0.4.0".to_owned(),
            commit: "abc1234".to_owned(),
            install_dir: "/usr/local/bin".to_owned(),
            installed_at: "2026-07-16T00:00:00Z".to_owned(),
        }
    }

    #[dialog_common::test]
    fn it_round_trips_through_the_state_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: tests in this mod run on one thread per process
        // invocation; nothing else reads this var concurrently.
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        store(&sample()).expect("store");
        assert_eq!(load(), Some(sample()));
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        assert_eq!(load(), None);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }

    #[dialog_common::test]
    fn it_loads_none_when_the_receipt_is_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var(crate::update::STATE_ENV, dir.path()) };
        std::fs::write(dir.path().join("install.json"), "{ not json").expect("write");
        assert_eq!(load(), None);
        unsafe { std::env::remove_var(crate::update::STATE_ENV) };
    }
}
```

- [ ] **Step 2: Add the channel-resolution test**

Add to the `mod tests` block in `rust/tonk-cli/src/update.rs`:

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::)'`
Expected: FAIL — `cannot find function resolve_channel`, unresolved module `receipt`.

- [ ] **Step 4: Write minimal implementation**

In `rust/tonk-cli/src/update.rs`, add the module declaration under `pub mod manifest;`:

```rust
pub mod receipt;
```

And add after `is_newer`:

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::)'`
Expected: all pass (3 receipt tests + 8 update tests).

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/update/receipt.rs
git commit -m "feat(tonk-cli): install receipt and channel resolution"
```

---

### Task 4: Nag state and cadence

**Files:**
- Create: `rust/tonk-cli/src/update/state.rs`
- Modify: `rust/tonk-cli/src/update.rs`

**Interfaces:**
- Consumes: `update::{STATE_ENV, is_newer}`
- Produces: `update::state::{State, path, load, store, should_check, should_nag, CHECK_INTERVAL_SECS, NAG_INTERVAL_SECS}`

Timestamps are RFC3339 strings so `install.sh` can write them with `date -u` and a human can read the file. `chrono` parses them for the interval compare; an unparseable timestamp is treated as "never", which fails toward checking rather than going permanently quiet.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-cli/src/update/state.rs`:

```rust
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
    std::fs::write(path, serde_json::to_string_pretty(state).unwrap_or_default())
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
```

- [ ] **Step 2: Register the module**

In `rust/tonk-cli/src/update.rs`, add under `pub mod receipt;`:

```rust
pub mod state;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::state)'`
Expected: 12 tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/update/state.rs
git commit -m "feat(tonk-cli): update check cache and nag cadence"
```

---

### Task 5: Fetching release metadata

**Files:**
- Create: `rust/tonk-cli/src/update/fetch.rs`
- Modify: `rust/tonk-cli/src/update.rs`

**Interfaces:**
- Consumes: `update::{Channel, endpoint}`, `update::manifest::Manifest`
- Produces: `update::fetch::{manifest, checksums, archive}`

- [ ] **Step 1: Write the implementation**

There is no useful unit test here — it is three HTTP calls, and the real coverage is the integration test in Task 7 that serves a fake release over a local `TcpListener` (the pattern `tests/telemetry.rs` already uses). Write it directly.

Create `rust/tonk-cli/src/update/fetch.rs`:

```rust
//! HTTP against a release: manifest, checksums, archive.
//!
//! `TONK_UPDATE_ENDPOINT` repoints the base URL so tests serve a fake
//! release from a local listener.

use anyhow::{Context as _, bail};

use crate::update::{Channel, endpoint, manifest::Manifest};

/// Fetch and parse the channel's `manifest.json`.
pub async fn manifest(channel: Channel) -> anyhow::Result<Manifest> {
    let url = format!("{}/manifest.json", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!(
            "no manifest.json on the {} channel ({} returned {})",
            channel.as_str(),
            url,
            response.status()
        );
    }
    let text = response
        .text()
        .await
        .with_context(|| format!("could not read {url}"))?;
    serde_json::from_str(&text).with_context(|| format!("could not parse {url}"))
}

/// Fetch the channel's raw `checksums.txt`.
pub async fn checksums(channel: Channel) -> anyhow::Result<String> {
    let url = format!("{}/checksums.txt", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!("could not download {url} ({})", response.status());
    }
    response
        .text()
        .await
        .with_context(|| format!("could not read {url}"))
}

/// Download one release archive.
pub async fn archive(channel: Channel, asset: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!("{}/{asset}", channel.base_url(&endpoint()));
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if !response.status().is_success() {
        bail!("could not download {url} ({})", response.status());
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("could not read {url}"))?;
    Ok(bytes.to_vec())
}
```

- [ ] **Step 2: Register the module**

In `rust/tonk-cli/src/update.rs`, add under `pub mod manifest;` (keep alphabetical: `fetch`, `manifest`, `receipt`, `state`):

```rust
pub mod fetch;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --package tonk-cli`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/update/fetch.rs
git commit -m "feat(tonk-cli): fetch release manifest, checksums, and archives"
```

---

### Task 6: The swap — guard, verify, extract, smoke-test, rename

**Files:**
- Create: `rust/tonk-cli/src/update/swap.rs`
- Modify: `rust/tonk-cli/src/update.rs`

**Interfaces:**
- Produces: `update::swap::{ForeignInstall, foreign_install, verify_sha256, extract_binary, install}`

This is the risky half. The tests use real temp dirs and a `#!/bin/sh` script as the "new binary" so the real `--version` gate and the real `rename()` are exercised.

- [ ] **Step 1: Write the failing test**

Create `rust/tonk-cli/src/update/swap.rs`:

```rust
//! Replacing the running binary.
//!
//! Everything is prepared on a temp file in the target's own
//! directory — extracted, permissioned, de-quarantined, and
//! smoke-tested — and the `rename()` happens last. A failure at any
//! step therefore leaves the working binary untouched: there is no
//! rollback path to get wrong, because nothing is ever half-applied.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use sha2_0_10::{Digest as _, Sha256};

/// An install this updater must not touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignInstall {
    /// Under `/nix/store` — read-only, owned by nix.
    Nix,
    /// Under `node_modules` — owned by npm.
    Npm,
}

impl ForeignInstall {
    /// The command that actually updates this kind of install.
    pub fn remedy(self) -> &'static str {
        match self {
            ForeignInstall::Nix => "update it through nix (e.g. `nix flake update`)",
            ForeignInstall::Npm => "run `npm i -g @tonk/cli@latest`",
        }
    }

    /// How this install is described in the refusal message.
    pub fn label(self) -> &'static str {
        match self {
            ForeignInstall::Nix => "a nix store path",
            ForeignInstall::Npm => "an npm install",
        }
    }
}

/// Classify a binary path we must not overwrite.
///
/// Checked by where the binary actually lives rather than by trusting
/// the receipt, so a copy installed by another package manager is
/// refused even if a stale receipt claims otherwise.
pub fn foreign_install(path: &Path) -> Option<ForeignInstall> {
    let text = path.to_string_lossy();
    if text.starts_with("/nix/store/") {
        return Some(ForeignInstall::Nix);
    }
    if path.components().any(|c| c.as_os_str() == "node_modules") {
        return Some(ForeignInstall::Npm);
    }
    None
}

/// Verify bytes against an expected hex SHA256.
///
/// The integrity gate: the binary is unsigned, so this is the only
/// thing between a download and your PATH. Mismatch is fatal.
pub fn verify_sha256(bytes: &[u8], expected: &str) -> anyhow::Result<()> {
    let actual = hex(&Sha256::digest(bytes));
    if actual != expected.trim().to_ascii_lowercase() {
        bail!("checksum mismatch (expected {expected}, got {actual})");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Extract the `tonk` entry from a `.tar.gz` to `dest`.
pub fn extract_binary(archive: &[u8], dest: &Path) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries().context("archive is not a readable tar")? {
        let mut entry = entry.context("archive entry is unreadable")?;
        let path = entry.path().context("archive entry has no path")?;
        if path.file_name().is_some_and(|name| name == "tonk") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).context("could not read tonk from archive")?;
            std::fs::write(dest, bytes)
                .with_context(|| format!("could not write {}", dest.display()))?;
            return Ok(());
        }
    }
    bail!("archive did not contain a 'tonk' binary")
}

/// Temp path beside `target`. Same directory because `rename()`
/// cannot cross filesystems.
fn temp_path(target: &Path) -> PathBuf {
    let dir = target.parent().unwrap_or(Path::new("."));
    dir.join(format!(".tonk-update-{}", std::process::id()))
}

/// Verify, unpack, validate, and atomically replace `target`.
///
/// On success `target` is the new binary. On any failure `target` is
/// byte-for-byte what it was.
pub fn install(archive: &[u8], expected_sha: &str, target: &Path) -> anyhow::Result<()> {
    if let Some(foreign) = foreign_install(target) {
        bail!(
            "{} is {} — tonk will not overwrite it; {}",
            target.display(),
            foreign.label(),
            foreign.remedy()
        );
    }
    verify_sha256(archive, expected_sha)?;

    let temp = temp_path(target);
    // A leftover temp from a killed run must not be mistaken for ours.
    let _ = std::fs::remove_file(&temp);
    let result = prepare(archive, &temp, target);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|err| with_permission_hint(err, target))
}

/// Add the directory and the `sudo` hint to a permission failure.
///
/// Both the temp write and the rename land in the target's own
/// directory, so an unwritable directory can fail at either. Attaching
/// the hint here — where every failure funnels through — keeps it
/// reachable without duplicating the message at each call site.
fn with_permission_hint(err: anyhow::Error, target: &Path) -> anyhow::Error {
    let denied = err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied);
    if !denied {
        return err;
    }
    let dir = target.parent().unwrap_or(Path::new(".")).display();
    err.context(format!("{dir} is not writable — try `sudo tonk update`"))
}

/// Everything up to and including the rename.
fn prepare(archive: &[u8], temp: &Path, target: &Path) -> anyhow::Result<()> {
    extract_binary(archive, temp)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(temp, std::fs::Permissions::from_mode(0o755))
            .context("could not make the new binary executable")?;
    }

    // macOS: the release binary is unsigned, so Gatekeeper would
    // quarantine it and arm64 requires *some* signature to execute.
    // Mirrors what install.sh does after a download.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr").arg("-c").arg(temp).status();
        let _ = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(temp)
            .status();
    }

    // Smoke-test BEFORE the rename. install.sh tests --version after
    // overwriting, so a bad binary is already your `tonk` by the time
    // you find out; testing first means a bad download never lands.
    let output = std::process::Command::new(temp)
        .arg("--version")
        .output()
        .with_context(|| format!("could not run the new binary at {}", temp.display()))?;
    if !output.status.success() {
        bail!(
            "the downloaded binary failed to run (`--version` exited {}); keeping the current one",
            output.status
        );
    }

    std::fs::rename(temp, target)
        .with_context(|| format!("could not replace {}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.tar.gz` holding one entry named `tonk` with `body`.
    fn archive_with(body: &str) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        header.set_path("tonk").expect("path");
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();

        let mut tar = tar::Builder::new(Vec::new());
        tar.append(&header, body.as_bytes()).expect("append");
        let tar = tar.into_inner().expect("finish");

        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).expect("gz");
        encoder.finish().expect("gz finish")
    }

    fn sha_of(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[dialog_common::test]
    fn it_accepts_bytes_matching_the_checksum() {
        assert!(verify_sha256(b"hello", &sha_of(b"hello")).is_ok());
    }

    #[dialog_common::test]
    fn it_rejects_bytes_not_matching_the_checksum() {
        let err = verify_sha256(b"hello", &sha_of(b"goodbye")).expect_err("must reject");
        assert!(err.to_string().contains("checksum mismatch"));
    }

    #[dialog_common::test]
    fn it_extracts_the_tonk_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("out");
        extract_binary(&archive_with("payload"), &dest).expect("extract");
        assert_eq!(std::fs::read_to_string(&dest).expect("read"), "payload");
    }

    #[dialog_common::test]
    fn it_rejects_an_archive_without_a_tonk_entry() {
        let mut header = tar::Header::new_gnu();
        header.set_path("other").expect("path");
        header.set_size(3);
        header.set_cksum();
        let mut tar = tar::Builder::new(Vec::new());
        tar.append(&header, &b"abc"[..]).expect("append");
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar.into_inner().expect("finish")).expect("gz");
        let bytes = encoder.finish().expect("gz finish");

        let dir = tempfile::tempdir().expect("tempdir");
        let err = extract_binary(&bytes, &dir.path().join("out")).expect_err("must reject");
        assert!(err.to_string().contains("did not contain"));
    }

    #[dialog_common::test]
    fn it_flags_nix_and_npm_paths_as_foreign() {
        assert_eq!(
            foreign_install(Path::new("/nix/store/abc-tonk/bin/tonk")),
            Some(ForeignInstall::Nix)
        );
        assert_eq!(
            foreign_install(Path::new("/home/x/node_modules/@tonk/cli-linux-x64/bin/tonk")),
            Some(ForeignInstall::Npm)
        );
    }

    #[dialog_common::test]
    fn it_does_not_flag_a_normal_install_as_foreign() {
        assert_eq!(foreign_install(Path::new("/usr/local/bin/tonk")), None);
        assert_eq!(foreign_install(Path::new("/home/x/.local/bin/tonk")), None);
    }

    #[cfg(unix)]
    #[dialog_common::test]
    fn it_replaces_the_target_when_the_new_binary_runs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        std::fs::write(&target, "old").expect("write");

        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let sha = sha_of(&archive);
        install(&archive, &sha, &target).expect("install");

        assert!(std::fs::read_to_string(&target).expect("read").contains("0.5.0"));
    }

    #[cfg(unix)]
    #[dialog_common::test]
    fn it_leaves_the_target_untouched_when_the_new_binary_fails_to_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        std::fs::write(&target, "old").expect("write");

        // Exits non-zero: the smoke test must reject it.
        let archive = archive_with("#!/bin/sh\nexit 1\n");
        let sha = sha_of(&archive);
        let err = install(&archive, &sha, &target).expect_err("must reject");

        assert!(err.to_string().contains("failed to run"));
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "old");
    }

    #[cfg(unix)]
    #[dialog_common::test]
    fn it_leaves_the_target_untouched_when_the_checksum_mismatches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        std::fs::write(&target, "old").expect("write");

        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let err = install(&archive, &sha_of(b"different"), &target).expect_err("must reject");

        assert!(err.to_string().contains("checksum mismatch"));
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "old");
    }

    #[cfg(unix)]
    #[dialog_common::test]
    fn it_leaves_no_temp_file_behind_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        std::fs::write(&target, "old").expect("write");

        let archive = archive_with("#!/bin/sh\nexit 1\n");
        let sha = sha_of(&archive);
        install(&archive, &sha, &target).expect_err("must reject");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tonk-update-"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind");
    }

    #[dialog_common::test]
    fn it_refuses_to_overwrite_a_foreign_install() {
        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let sha = sha_of(&archive);
        let err = install(&archive, &sha, Path::new("/nix/store/abc/bin/tonk"))
            .expect_err("must refuse");
        assert!(err.to_string().contains("will not overwrite"));
    }

    #[cfg(unix)]
    #[dialog_common::test]
    fn it_suggests_sudo_when_the_target_directory_is_not_writable() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        std::fs::write(&target, "old").expect("write");

        // Read+execute but not write: the temp file cannot be created,
        // which is where an unwritable /usr/local/bin actually fails.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("chmod");

        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let sha = sha_of(&archive);
        let result = install(&archive, &sha, &target);

        // Restore before asserting, so a failed assert still lets the
        // tempdir clean itself up.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("chmod");

        let err = result.expect_err("must fail on an unwritable directory");
        let message = format!("{err:#}");
        assert!(message.contains("sudo tonk update"), "message: {message}");
        assert!(
            message.contains(&dir.path().display().to_string()),
            "message: {message}"
        );
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "old");
    }
}
```

- [ ] **Step 2: Register the module**

In `rust/tonk-cli/src/update.rs`, add under `pub mod state;`:

```rust
pub mod swap;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo nextest run --package tonk-cli --lib -E 'test(update::swap)'`
Expected: 12 tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/update/swap.rs
git commit -m "feat(tonk-cli): atomic binary swap with a pre-rename smoke test"
```

---

### Task 7: The `tonk update` command

**Files:**
- Modify: `rust/tonk-cli/src/update.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Create: `rust/tonk-cli/tests/update.rs`
- Modify: `rust/tonk-cli/Cargo.toml` (add the `[[test]]` entry)

**Interfaces:**
- Consumes: everything from Tasks 1–6
- Produces: `update::run(disable_check: Option<bool>) -> anyhow::Result<String>`

- [ ] **Step 1: Write the run() implementation**

Add to `rust/tonk-cli/src/update.rs`, after `resolve_channel`:

```rust
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
    let expected = manifest::parse_checksums(&checksums, &asset)
        .with_context(|| format!("no checksum entry for {asset} on the {} channel", channel.as_str()))?;

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
```

Add the import at the top of `rust/tonk-cli/src/update.rs`, above the module declarations:

```rust
use anyhow::Context as _;
```

- [ ] **Step 2: Add the short_commit test**

Add to the `mod tests` block in `rust/tonk-cli/src/update.rs`:

```rust
    #[dialog_common::test]
    fn it_abbreviates_a_commit_without_panicking_on_short_input() {
        assert_eq!(short_commit("27b74e22b1234567"), "27b74e2");
        assert_eq!(short_commit("abc"), "abc");
        assert_eq!(short_commit(""), "");
    }
```

- [ ] **Step 3: Wire the subcommand**

In `rust/tonk-cli/src/bin/tonk.rs`, add to the `enum Command` block, immediately before the closing `}` that follows the `Telemetry { … }` variant:

```rust
    /// Update tonk to the latest release
    ///
    /// Upgrades installs made by the install script. Copies installed
    /// via npm or nix are left to those tools.
    #[command(after_help = "Examples:\n  tonk update\n  tonk update --disable-check")]
    Update {
        /// Stop checking for new releases in the background.
        #[arg(long, conflicts_with = "enable_check")]
        disable_check: bool,
        /// Resume checking for new releases in the background.
        #[arg(long)]
        enable_check: bool,
    },
```

Add to `fn descriptor`, next to the `Command::Telemetry` arm:

```rust
        Command::Update { .. } => ("update", None),
```

Add to the `match cli.command` dispatch in `main`, next to the other arms:

```rust
        Command::Update {
            disable_check,
            enable_check,
        } => update(disable_check, enable_check).await,
```

Add the handler next to the other command handlers (near `print_error`):

```rust
/// `tonk update` — upgrade in place, or toggle the background check.
async fn update(disable_check: bool, enable_check: bool) -> ExitCode {
    let set_check = match (disable_check, enable_check) {
        (true, _) => Some(false),
        (_, true) => Some(true),
        _ => None,
    };
    match tonk_cli::update::run(set_check).await {
        Ok(message) => {
            println!("{message}");
            ExitCode::Success
        }
        Err(err) => print_error(format!("{err:#}")),
    }
}
```

`{err:#}` renders the full anyhow context chain, so a failure reports the URL or path that caused it rather than just the outermost message.

- [ ] **Step 4: Verify it builds and the command appears**

Run: `cargo run --package tonk-cli --bin tonk -- update --help`
Expected: help text showing `--disable-check` and `--enable-check`.

- [ ] **Step 5: Write the integration test**

Create `rust/tonk-cli/tests/update.rs`:

```rust
//! End-to-end `tonk update`: the real binary against a fake release
//! served from a local listener.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;

/// Serve a fixed set of `path -> body` responses until dropped.
/// Anything unmapped gets a 404, which is how the "missing manifest"
/// case is exercised.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 2048];
            let Ok(n) = stream.read(&mut buffer) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..n]).into_owned();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_owned();
            let body = routes
                .iter()
                .find(|(route, _)| *route == path)
                .map(|(_, body)| body.clone());
            let response = match body {
                Some(body) => {
                    let mut head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    head.extend_from_slice(&body);
                    head
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    format!("http://127.0.0.1:{port}/releases")
}

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

/// Run `tonk update` against `endpoint` with state isolated in `state_dir`.
fn run_update(endpoint: &str, state_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("update")
        .args(args)
        .env("TONK_UPDATE_ENDPOINT", endpoint)
        .env("TONK_UPDATE_STATE", state_dir)
        // Isolated so the test never reads the developer's real
        // telemetry choice; without a key nothing is sent anyway.
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env_remove("TONK_CHANNEL")
        .env_remove("TONK_POSTHOG_KEY")
        .output()
        .expect("run tonk update")
}

fn manifest_body(version: &str, commit: &str) -> Vec<u8> {
    format!(
        r#"{{"version":"{version}","commit":"{commit}","channel":"stable","built_at":"2026-07-16T00:00:00Z"}}"#
    )
    .into_bytes()
}

#[dialog_common::test]
fn it_reports_already_current_when_the_receipt_matches_the_release() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("install.json"),
        r#"{"channel":"stable","version":"0.4.0","commit":"abc1234def","install_dir":"/usr/local/bin","installed_at":"2026-07-16T00:00:00Z"}"#,
    )
    .expect("write receipt");

    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.4.0", "abc1234def"),
    )]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("already current"), "stdout: {stdout}");
    assert!(stdout.contains("abc1234"), "stdout: {stdout}");
}

#[dialog_common::test]
fn it_fails_loudly_when_the_channel_has_no_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Empty route table: every request 404s.
    let endpoint = serve(vec![]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr.contains("no manifest.json"), "stderr: {stderr}");
}

#[dialog_common::test]
fn it_toggles_the_background_check_without_touching_the_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Unroutable endpoint: proves the toggle never fetches.
    let endpoint = "http://127.0.0.1:1/releases";

    let output = run_update(endpoint, dir.path(), &["--disable-check"]);
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("disabled"));

    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    assert!(state.contains("\"check_enabled\": false"), "state: {state}");

    let output = run_update(endpoint, dir.path(), &["--enable-check"]);
    assert!(output.status.success());
    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    assert!(state.contains("\"check_enabled\": true"), "state: {state}");
}
```

- [ ] **Step 6: Register the test binary**

In `rust/tonk-cli/Cargo.toml`, add after the `telemetry` `[[test]]` entry (`autotests = false` means every test binary must be listed):

```toml
[[test]]
name = "update"
path = "tests/update.rs"
```

- [ ] **Step 7: Run the tests**

Run: `cargo nextest run --package tonk-cli --test update`
Expected: 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/update.rs rust/tonk-cli/Cargo.toml
git commit -m "feat(tonk-cli): add the tonk update command"
```

---

### Task 8: The background check and nag

**Files:**
- Modify: `rust/tonk-cli/src/update.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Modify: `rust/tonk-cli/tests/update.rs`

**Interfaces:**
- Consumes: `update::{state, fetch, resolve_channel, is_newer}`
- Produces: `update::{check, nag, CHECK_TIMEOUT}`

- [ ] **Step 1: Write the check and nag implementation**

Add to `rust/tonk-cli/src/update.rs`, after `run`:

```rust
/// How long the background check may take before we give up on it.
/// It runs concurrently with the 300ms telemetry flush, so the worst
/// case is roughly this once a day, not on every command.
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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
```

- [ ] **Step 2: Wire it into main**

In `rust/tonk-cli/src/bin/tonk.rs`, `cli.command` is moved by the dispatch `match`, so capture the flag before it. Change line 745 from:

```rust
    let started = std::time::Instant::now();
```

to:

```rust
    let started = std::time::Instant::now();
    // `cli.command` is moved by the dispatch below, so ask now.
    let is_update = matches!(cli.command, Command::Update { .. });
```

Then replace `main`'s tail (currently `src/bin/tonk.rs:784-787`, immediately after the `};` closing the `let exit = match cli.command` dispatch):

```rust
    if let Some(recorder) = recorder {
        recorder.finish(exit, started.elapsed()).await;
    }
    std::process::exit(exit.into_raw());
```

with:

```rust
    let duration = started.elapsed();

    // `tonk update` speaks for itself: a nag mid-update contradicts
    // the command that just ran, and the toggle must stay silent.
    // Run the check alongside the telemetry flush rather than in
    // front of the command, so the marginal cost is one small GET
    // parallel to a request already in flight.
    let check = async {
        if !is_update {
            tonk_cli::update::check().await;
        }
    };
    match recorder {
        Some(recorder) => {
            tokio::join!(recorder.finish(exit, duration), check);
        }
        None => check.await,
    }
    if !is_update {
        tonk_cli::update::nag();
    }

    std::process::exit(exit.into_raw());
```

- [ ] **Step 3: Add the nag integration tests**

Add to `rust/tonk-cli/tests/update.rs`:

```rust
/// Run a command that reaches main's exit path with no repo and no
/// network of its own, so the nag runs against a controlled cache.
///
/// NOT `--version`: clap handles that inside `Cli::parse()` and exits
/// before main's tail, so the check and nag would never run and these
/// tests would pass while proving nothing. `tonk telemetry` (status)
/// needs no repo, prints two predictable lines, and exits through the
/// tail. Without `TONK_POSTHOG_KEY` it sends nothing.
fn run_probe(endpoint: &str, state_dir: &std::path::Path, extra: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(binary());
    cmd.arg("telemetry")
        .env("TONK_UPDATE_ENDPOINT", endpoint)
        .env("TONK_UPDATE_STATE", state_dir)
        // Keep `tonk telemetry` off the real telemetry.json too.
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env_remove("CI")
        .env_remove("TONK_CHANNEL")
        .env_remove("TONK_NO_UPDATE_CHECK")
        .env_remove("TONK_POSTHOG_KEY");
    for (key, value) in extra {
        cmd.env(key, value);
    }
    cmd.output().expect("run tonk telemetry")
}

#[dialog_common::test]
fn it_nags_on_stderr_when_the_release_is_newer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stderr.contains("99.0.0 is available"), "stderr: {stderr}");
    assert!(stderr.contains("run `tonk update`"), "stderr: {stderr}");
    // stdout is parsed by agents and must stay clean.
    assert!(!stdout.contains("is available"), "stdout: {stdout}");
}

#[dialog_common::test]
fn it_does_not_nag_when_the_release_is_not_newer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.0.1", "aaa0001"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
}

#[dialog_common::test]
fn it_does_not_nag_when_opted_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[("TONK_NO_UPDATE_CHECK", "1")]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
    // Opting out must not even leave state behind.
    assert!(!dir.path().join("update.json").exists());
}

#[dialog_common::test]
fn it_does_not_nag_in_ci() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[("CI", "true")]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
}

#[dialog_common::test]
fn it_stays_silent_and_succeeds_when_the_check_cannot_reach_the_release() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Nothing listening: the check must fail invisibly.
    let output = run_probe("http://127.0.0.1:1/releases", dir.path(), &[]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("error"), "stderr: {stderr}");
    assert!(!stderr.contains("is available"), "stderr: {stderr}");
    // last_checked_at must actually advance, so an offline machine backs
    // off to a daily retry instead of re-checking on every command. Parse
    // rather than substring-match: serde always emits the key, so
    // `contains("last_checked_at")` would pass even when it is null.
    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    let parsed: serde_json::Value = serde_json::from_str(&state).expect("parse state");
    assert!(
        !parsed["last_checked_at"].is_null(),
        "last_checked_at should have advanced despite the failed check: {state}"
    );
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo nextest run --package tonk-cli --test update`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-cli/src/update.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/update.rs
git commit -m "feat(tonk-cli): check for new releases daily and nag on stderr"
```

---

### Task 9: `install.sh` writes the receipt

**Files:**
- Modify: `install.sh`

**Interfaces:**
- Produces: `install.json` in the state dir, matching `update::receipt::Receipt`

`install.sh` is served **from the release**, so copies in the wild never get this change. Everything here must be additive and best-effort: an install must never fail because the manifest fetch 404'd (older releases have no `manifest.json` at all).

- [ ] **Step 1: Write the receipt after a successful install**

In `install.sh`, insert immediately after the `say "installed tonk to $dest"` line:

```sh
# Record what we installed, so `tonk update` knows which channel to
# check and can answer "already current" without downloading an
# archive. Best-effort: an older release has no manifest.json, and a
# receipt we cannot write is not a reason to fail an install.
#
# Mirrors `update::receipt::Receipt`; TONK_UPDATE_STATE overrides the
# directory for tests, matching the CLI.
if [ -n "${TONK_UPDATE_STATE:-}" ]; then
  state_dir="$TONK_UPDATE_STATE"
elif [ "$os" = "Darwin" ]; then
  state_dir="$HOME/Library/Application Support/tonk"
else
  state_dir="${XDG_DATA_HOME:-$HOME/.local/share}/tonk"
fi

# Escape a value for use inside a JSON string: backslashes first, then
# quotes. An install dir containing either would otherwise emit JSON the
# CLI silently fails to parse (it reads the receipt with `.ok()`).
json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# Everything that can fail lives here, and it is only ever called as an
# `if` condition, so a failure skips the receipt instead of aborting an
# install whose binary is already in place. `mkdir -p` succeeds on an
# existing directory even when it is unwritable, so the write itself has
# to be guarded, not just the mkdir.
write_receipt() {
  mkdir -p "$state_dir" 2>/dev/null || return 1
  cat > "$state_dir/install.json" 2>/dev/null <<EOF || return 1
{
  "channel": "$channel",
  "version": "$m_version",
  "commit": "$m_commit",
  "install_dir": "$(json_escape "$INSTALL_DIR")",
  "installed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
  return 0
}

if fetch "${url%/*}/manifest.json" "$tmp/manifest.json" 2>/dev/null; then
  # Pull two string fields out without requiring jq.
  m_version="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/manifest.json")"
  m_commit="$(sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/manifest.json")"
  if [ -n "$m_version" ] && [ -n "$m_commit" ]; then
    # 2>/dev/null on the call, not just inside: the shell opens the
    # redirect target before `cat` runs, so `cat`'s own redirect cannot
    # suppress a "Permission denied" for an unwritable state dir. This is
    # best-effort, so it stays silent.
    if write_receipt 2>/dev/null; then
      say "recorded install receipt in $state_dir"
    fi
  fi
fi
```

**Why the write is guarded and not just the `mkdir`.** `mkdir -p` exits 0
when the directory already exists, *regardless of whether it is writable* —
it never checks write permission. So on a pre-existing unwritable state dir
(a prior sudo-owned install, a managed home, a read-only CI home) or a full
disk, an unguarded `cat > "$state_dir/install.json"` fails and `set -eu`
aborts the whole script — **after** the binary is already installed. The user
would see a failed install despite a working `tonk`, and the PATH note and
`--version` smoke test below would never run. Putting every fallible step
inside a function called as an `if` condition makes a write failure skip the
receipt, which is the whole point of "best-effort".

`${url%/*}` strips the asset name off the download URL, leaving the release directory — so the manifest is fetched from the same release the archive came from, whether that resolved via `latest/download` or an explicit tag.

- [ ] **Step 2: Verify the receipt lands**

Note `data_dir()` on macOS is `~/Library/Application Support`, and on Linux `$XDG_DATA_HOME` or `~/.local/share` — the branches above match what the `dirs` crate resolves.

Run:

```bash
sh -n install.sh
```

Expected: no output (syntax OK).

Then dry-run the receipt logic against a temp state dir without touching your real install:

```bash
TONK_UPDATE_STATE=$(mktemp -d) TONK_INSTALL_DIR=$(mktemp -d) sh install.sh && \
  cat "$TONK_UPDATE_STATE/install.json"
```

Expected: an `install.json` with a `channel`, `version`, `commit`, `install_dir`, and `installed_at`.

**Note:** this only succeeds once Task 10 has published a `manifest.json` to the release. Until then the fetch 404s and the install completes with no receipt — which is exactly the required behaviour, and worth confirming by running the command above *before* Task 10 and seeing the install still succeed.

Then verify the best-effort guarantee against the failure mode that actually
breaks it — a state dir that exists but cannot be written:

```bash
ro=$(mktemp -d) && chmod 555 "$ro"
TONK_UPDATE_STATE="$ro" TONK_INSTALL_DIR=$(mktemp -d) sh install.sh; echo "exit=$?"
chmod 755 "$ro"
```

Expected: `exit=0`, the binary installed, no receipt, and the script's trailing
`--version` output still printed. A non-zero exit here means the receipt block
can fail an install, which is the one thing it must never do.

Finally, exercise the success path, which the real release cannot until Task 10
ships the asset. Serve a fake release locally (`manifest.json`, `checksums.txt`,
and a matching `tonk-<platform>.tar.gz`) with `python3 -m http.server`, point a
**copy** of `install.sh` outside the repo at it, and confirm the resulting
`install.json` carries all five fields. Use a `TONK_INSTALL_DIR` containing a
double quote to confirm `json_escape` keeps the JSON parseable.

- [ ] **Step 3: Commit**

```bash
git add install.sh
git commit -m "feat(install): record an install receipt for tonk update"
```

---

### Task 10: Publish `manifest.json` from both channels

**Files:**
- Modify: `.github/workflows/cli.yml`

**Interfaces:**
- Produces: `manifest.json` on the `tonk-latest` and `tonk-staging` releases

- [ ] **Step 1: Build and verify the manifest in `publish-staging`**

In `.github/workflows/cli.yml`, in the `publish-staging` job's `Prepare release assets` step, append to the `run:` block after the `sha256sum … > checksums.txt` line:

```yaml
          # Release identity for `tonk update`. The version string alone
          # cannot identify a build on a rolling tag, so carry the commit.
          version="$(grep -A30 '^\[workspace.package\]' Cargo.toml \
            | grep -m1 '^version' \
            | sed -E 's/.*"([^"]+)".*/\1/')"
          cat > manifest.json <<EOF
          {
            "version": "$version",
            "commit": "${{ github.sha }}",
            "channel": "staging",
            "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
          }
          EOF
          # Fail the release rather than publish a manifest the CLI
          # cannot parse.
          python3 -c 'import json,sys; m=json.load(open("manifest.json")); [sys.exit(f"manifest missing {k}") for k in ("version","commit","channel","built_at") if not m.get(k)]'
          cat manifest.json
```

Then add `manifest.json` to that job's `files:` list:

```yaml
          files: |
            tonk-macos-arm64.tar.gz
            tonk-linux-x86_64.tar.gz
            checksums.txt
            manifest.json
            install.sh
```

- [ ] **Step 2: Do the same for `publish-stable`**

Identical, with `"channel": "stable"`. In the `publish-stable` job's `Prepare release assets` step, append after the `sha256sum` line:

```yaml
          # Release identity for `tonk update`. The version string alone
          # cannot identify a build on a rolling tag, so carry the commit.
          version="$(grep -A30 '^\[workspace.package\]' Cargo.toml \
            | grep -m1 '^version' \
            | sed -E 's/.*"([^"]+)".*/\1/')"
          cat > manifest.json <<EOF
          {
            "version": "$version",
            "commit": "${{ github.sha }}",
            "channel": "stable",
            "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
          }
          EOF
          # Fail the release rather than publish a manifest the CLI
          # cannot parse.
          python3 -c 'import json,sys; m=json.load(open("manifest.json")); [sys.exit(f"manifest missing {k}") for k in ("version","commit","channel","built_at") if not m.get(k)]'
          cat manifest.json
```

And add `manifest.json` to that job's `files:` list, exactly as in Step 1.

- [ ] **Step 3: Verify the version extraction works locally**

The `grep`/`sed` pipeline is copied verbatim from the existing `cli-npm.yml` `Derive and validate version` step, so it is already proven against this `Cargo.toml`. Confirm:

```bash
grep -A30 '^\[workspace.package\]' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/'
```

Expected: `0.4.0`

- [ ] **Step 4: Verify the workflow parses**

Run: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/cli.yml"))'`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/cli.yml
git commit -m "ci(cli): publish a release manifest for tonk update"
```

---

### Task 11: Documentation

**Files:**
- Modify: `README.md:72-82` (the "Update" section)
- Modify: `rust/tonk-cli/README.md` (the "Update" section, same text)

**Interfaces:** none

- [ ] **Step 1: Locate both copies**

Run: `grep -rn "There is no self-update command yet" README.md rust/tonk-cli/README.md`
Expected: a hit in each. Both get the same replacement.

- [ ] **Step 2: Replace the Update section**

In each file, replace the section that begins `### Update` and ends before `### Quick start` with:

````markdown
### Update

```sh
tonk update
```

This upgrades an install made by the install script: it downloads the
newest release for your channel, verifies it against the release
checksums, checks the new binary runs, and only then replaces the old
one — so a failed update leaves your working `tonk` untouched. On
macOS it re-runs the de-quarantine and ad-hoc re-sign for you.

`tonk` checks for new releases once a day and prints a one-line notice
on stderr when one exists. Turn that off with `tonk update
--disable-check` (or `TONK_NO_UPDATE_CHECK=1`), and back on with `tonk
update --enable-check`. It never runs in CI.

Check what you have with `tonk --version`. `tonk update` follows the
channel you installed from, so a `TONK_CHANNEL=staging` install keeps
tracking staging.

If `tonk` was installed some other way, `tonk update` says so instead
of interfering: use `npm i -g @tonk/cli@latest` for an npm install, or
your flake for a nix one. Re-running the install command also still
works:

```sh
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
```
````

- [ ] **Step 3: Verify no stale claim survives**

Run: `grep -rn "no self-update" README.md rust/tonk-cli/README.md`
Expected: no matches.

- [ ] **Step 4: Commit**

```bash
git add README.md rust/tonk-cli/README.md
git commit -m "docs: document tonk update and the release check"
```

---

## Final Verification

- [ ] **Full test suite**

Run: `cargo nextest run --package tonk-cli`
Expected: all pass, including the 8 `update` integration tests.

**Use `cargo nextest run`, not `cargo test`.** nextest is this repo's runner
(it's in the flake dev shell, and `.config/nextest.toml` already forces
integration tests into a serial group to avoid deadlock). It runs each test
in its own process, which is what makes the `set_var`/`remove_var` env
overrides in these tests safe — the same assumption `telemetry.rs` already
relies on, stated in its SAFETY comments. Under `cargo test`, every test in
a binary shares one process across threads, so the tests that point
`TONK_UPDATE_STATE` at their own tempdir race and fail. That is a property
of the runner, not a defect in the tests.

- [ ] **The lint gate** (this is the real gate — workspace clippy with `--all-targets --all-features`, not per-crate)

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --check`
Expected: no output.

- [ ] **Manual end-to-end** (cannot be automated — needs a real release)

After the workflow has published a `manifest.json` at least once:

```bash
# 1. The check is real:
curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/manifest.json

# 2. A real install writes a receipt:
TONK_INSTALL_DIR=$(mktemp -d) sh install.sh
cat ~/Library/Application\ Support/tonk/install.json   # macOS

# 3. An update against a current release is a no-op:
tonk update      # expect: "already current: 0.4.0 (<sha>, stable)"
```

---

## Notes for the implementer

**Why the rename is last.** `install.sh` smoke-tests `--version` *after* it overwrites, so a bad binary is already your `tonk` by the time the failure prints. Doing every step on a temp file and renaming last means there is no rollback path to get wrong. If you find yourself adding one, something has moved out of order.

**Why the check fails silently but `tonk update` shouts.** This repo treats silent failure as a defect and a reviewer will flag it. The check is the exception on purpose: nothing depends on its result and no user asked for it. `tonk update` is the opposite — the user asked, so every failure names what broke.

**Why versions and commits are compared in different places.** Both channels are rolling tags rebuilt on every push, so `0.4.0` is many builds. Versions answer "is there an update worth interrupting someone for" (the nag); commits answer "is this the exact build the channel serves" (`tonk update`). Do not unify them.

**The `TONK_UPDATE_STATE` env var covers both `install.json` and `update.json`** — one override for both files, so a test isolates all update state with one variable.
