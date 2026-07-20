# CLI Canonical Spots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace cwd-based `.tonk/` discovery with a platform-canonical spot store plus a name registry, so `tonk` works from anywhere against a selected spot.

**Architecture:** A new `spot` module owns a JSON registry (`spots.json`, name → site path + `current` selection) under `dirs::data_dir()/tonk/`. Every data command resolves `--spot` > `TONK_SPOT` > registry `current` and opens the site at the registered path; `discover_and_open` is deleted. New commands `tonk use` and `tonk spot new|list|rm` manage the registry; `tonk join` lands in a canonical site; `tonk init` is removed. Spec: `docs/superpowers/specs/2026-07-20-cli-canonical-spots-design.md`.

**Tech Stack:** Rust (tonk-cli crate), clap, serde_json, dirs, tempfile-based tests.

## Global Constraints

- Work on branch `feat/cli-canonical-dir` in `/Users/jackdouglas/tonk/tonk/.wt/cli`. All paths below are relative to `rust/tonk-cli/` unless they start with `docs/` or `rust/`.
- Lint gate (matches CI): `cargo clippy --all-targets --all-features -- -D warnings` natively, plus `cargo fmt --check`.
- Tests use `#[dialog_common::test]` for async, plain `#[test]` for pure sync logic; names are `it_does_x`, grouped in behaviour modules.
- No `mod.rs`. New integration test files need a `[[test]]` entry in `Cargo.toml` (`autotests = false`).
- Commit messages: Conventional Commits, imperative, lowercase, no trailing period, no emojis. End the body with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Doc comments are load-bearing in this repo: every new pub item gets one in the style of the neighboring code. The doc comments in the code blocks below are part of the deliverable.
- Vocabulary: a **spot** is the named registry entry; a **site** is the physical directory backing it. Never say "workspace" (collides with the tonk-ui tab surface).
- Registry semantics (from the spec): absolute expanded paths in `spots.json`; atomic temp-file + rename writes; corrupt registry is a hard error naming the file, never silently recreated; spot names match `[a-z0-9][a-z0-9-_]*`.

---

### Task 1: `spot` module — store, registry, resolution

**Files:**
- Create: `src/spot.rs`
- Modify: `src/lib.rs` (add `pub mod spot;` to the module list and a one-line entry in the crate doc's module inventory, alongside the `site` entry)

**Interfaces:**
- Produces: `spot::SpotStore` (`open()`, `at(dir)`, `registry_path()`, `canonical_site(name)`, `load()`, `save(&Registry)`, `resolve(flag, env)`), `spot::Registry { current, spots }`, `spot::SpotEntry { site }`, `spot::Resolved { name, site, source }`, `spot::Source` (Display: `flag`/`env`/`global`), `spot::SpotError`, `spot::validate_name(&str)`, consts `spot::SPOT_ENV = "TONK_SPOT"`, `spot::STATE_ENV = "TONK_SPOTS_STATE"`. Tasks 3–5 consume all of these.

- [ ] **Step 1: Write `src/spot.rs` with failing-to-compile consumers in mind — module plus unit tests in one pass**

```rust
//! Spot registry: named spots, canonical storage, selection.
//!
//! A *spot* is a named entry in `spots.json` mapping to the *site*
//! directory that backs it (see [`crate::site`]). The registry and
//! the canonical site directories live under the platform data dir
//! (`~/Library/Application Support/tonk/` on macOS), next to
//! `telemetry.json` / `update.json`:
//!
//! ```text
//! tonk/
//!   spots.json      registry: name → site path, plus `current`
//!   spots/<name>/   canonical site dirs
//! ```
//!
//! Selection resolves `--spot` > `TONK_SPOT` > the registry's
//! `current`. The flag and env forms are per-invocation /
//! per-process, so concurrent sessions pinning their own spot can
//! never mix regardless of who rewrites the shared `current`.
//!
//! `spots.json` stores absolute, expanded paths so applications
//! built on tonk can resolve a name with zero path logic. Writes
//! go through a temp file + atomic rename. A corrupt registry is a
//! hard error naming the file — never silently recreated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment variable naming the spot to use, beaten only by the
/// `--spot` flag. Automation (agents, bench, CI) should always set
/// this (or pass `--spot`) and never rely on `tonk use`.
pub const SPOT_ENV: &str = "TONK_SPOT";

/// Environment variable overriding the directory that holds
/// `spots.json` and the canonical `spots/` root, so tests can
/// isolate state (same pattern as `TONK_TELEMETRY_STATE`).
pub const STATE_ENV: &str = "TONK_SPOTS_STATE";

/// File name of the registry inside the store directory.
const REGISTRY_FILE: &str = "spots.json";

/// Directory name (inside the store) holding canonical site dirs.
const SPOTS_DIRNAME: &str = "spots";

/// On-disk registry: the shared `current` selection plus one entry
/// per spot. `BTreeMap` keeps listing and serialization order
/// stable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    /// The globally selected spot, if any. A convenience for the
    /// human at the keyboard; concurrent sessions pin their spot
    /// via [`SPOT_ENV`] or `--spot` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// Name → entry. Paths inside are absolute and expanded.
    #[serde(default)]
    pub spots: BTreeMap<String, SpotEntry>,
}

/// One registered spot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpotEntry {
    /// Absolute path to the site directory backing this spot.
    pub site: PathBuf,
}

/// Where a resolution's spot name came from. Surfaced in status
/// and error output so a session can always tell what it is about
/// to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `--spot` flag.
    Flag,
    /// [`SPOT_ENV`] environment variable.
    Env,
    /// The registry's `current` field.
    Global,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Flag => "flag",
            Source::Env => "env",
            Source::Global => "global",
        })
    }
}

/// A successful resolution: which spot, where its site lives, and
/// which selection mechanism picked it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The spot's registry name.
    pub name: String,
    /// Absolute path to the site directory.
    pub site: PathBuf,
    /// Which selection mechanism named it.
    pub source: Source,
}

/// Failure modes for registry access and resolution.
#[derive(Debug, Error)]
pub enum SpotError {
    /// Spots exist but nothing is selected anywhere.
    #[error("no spot selected; run `tonk use <name>`, pass --spot, or set TONK_SPOT")]
    NoSelection,
    /// The registry has zero spots — selection is moot; the fix is
    /// creating one.
    #[error("no spots registered; create one with `tonk spot new <name>`")]
    NothingRegistered,
    /// A name that isn't in the registry.
    #[error("unknown spot '{name}'{}", unknown_hint(.available))]
    Unknown {
        /// The name that failed to resolve.
        name: String,
        /// Every registered name, for the error hint.
        available: Vec<String>,
    },
    /// `spot new` against a name that already exists. Re-pointing
    /// is an explicit `rm` + `new`, never an overwrite.
    #[error("spot '{0}' already exists; run `tonk spot rm {0}` first to re-point it")]
    Exists(String),
    /// The registry file exists but doesn't parse. Deliberately
    /// not self-healing: silently recreating it would orphan every
    /// registered spot.
    #[error("corrupt spot registry at {path}: {detail}")]
    Corrupt {
        /// Path to the offending `spots.json`.
        path: PathBuf,
        /// The serde error text.
        detail: String,
    },
    /// A name outside the allowed slug. Canonical names become
    /// directory names, so the alphabet is conservative.
    #[error("invalid spot name '{0}': use [a-z0-9][a-z0-9-_]*")]
    InvalidName(String),
    /// The platform reports no data directory (no home).
    #[error("could not determine the platform data directory")]
    NoDataDir,
    /// Site bootstrap (`spot new`) failed inside the site layer.
    #[error("failed to initialize site: {0}")]
    Init(String),
    /// Registry or site-directory I/O.
    #[error("{0}")]
    Io(String),
}

/// Hint suffix for [`SpotError::Unknown`]: list what is
/// registered, or point at `spot new` when nothing is.
fn unknown_hint(available: &[String]) -> String {
    if available.is_empty() {
        "; none registered (create one with `tonk spot new <name>`)".to_string()
    } else {
        format!("; registered: {}", available.join(", "))
    }
}

/// Handle on the spot state directory. All registry reads and
/// writes go through one of these; tests construct it with
/// [`SpotStore::at`] over a tempdir so nothing touches the user's
/// real data dir and no test ever mutates process-global env.
#[derive(Debug, Clone)]
pub struct SpotStore {
    dir: PathBuf,
}

impl SpotStore {
    /// The real store: [`STATE_ENV`] override, else the platform
    /// data dir (`dirs::data_dir()/tonk`, the same base telemetry
    /// and update state use).
    pub fn open() -> Result<Self, SpotError> {
        if let Ok(dir) = std::env::var(STATE_ENV)
            && !dir.is_empty()
        {
            return Ok(Self { dir: dir.into() });
        }
        let base = dirs::data_dir().ok_or(SpotError::NoDataDir)?;
        Ok(Self {
            dir: base.join("tonk"),
        })
    }

    /// A store rooted at an explicit directory (tests).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Path to `spots.json` inside this store.
    pub fn registry_path(&self) -> PathBuf {
        self.dir.join(REGISTRY_FILE)
    }

    /// Canonical site directory for `name` inside this store.
    /// Purely a path computation — nothing is created.
    pub fn canonical_site(&self, name: &str) -> PathBuf {
        self.dir.join(SPOTS_DIRNAME).join(name)
    }

    /// Load the registry. A missing file is an empty registry (the
    /// pre-first-use state); an unparseable one is
    /// [`SpotError::Corrupt`].
    pub fn load(&self) -> Result<Registry, SpotError> {
        let path = self.registry_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Registry::default());
            }
            Err(e) => {
                return Err(SpotError::Io(format!(
                    "could not read {}: {e}",
                    path.display()
                )));
            }
        };
        serde_json::from_str(&text).map_err(|e| SpotError::Corrupt {
            path,
            detail: e.to_string(),
        })
    }

    /// Persist the registry atomically: write a sibling temp file,
    /// then rename over `spots.json` so concurrent readers never
    /// observe a torn write.
    pub fn save(&self, registry: &Registry) -> Result<(), SpotError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| SpotError::Io(format!("could not create {}: {e}", self.dir.display())))?;
        let path = self.registry_path();
        let tmp = self.dir.join(format!("{REGISTRY_FILE}.tmp"));
        let text = serde_json::to_string_pretty(registry)
            .map_err(|e| SpotError::Io(format!("could not serialize registry: {e}")))?;
        std::fs::write(&tmp, text)
            .map_err(|e| SpotError::Io(format!("could not write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            SpotError::Io(format!(
                "could not move {} into place: {e}",
                tmp.display()
            ))
        })
    }

    /// Resolve the spot a command should operate on.
    ///
    /// Strict precedence: `flag` (`--spot`) > `env` ([`SPOT_ENV`],
    /// already read and empty-filtered by the caller) > the
    /// registry's `current`. The cwd is never consulted.
    pub fn resolve(&self, flag: Option<&str>, env: Option<&str>) -> Result<Resolved, SpotError> {
        let registry = self.load()?;
        let (name, source) = if let Some(name) = flag {
            (name.to_owned(), Source::Flag)
        } else if let Some(name) = env {
            (name.to_owned(), Source::Env)
        } else if let Some(name) = registry.current.clone() {
            (name, Source::Global)
        } else if registry.spots.is_empty() {
            return Err(SpotError::NothingRegistered);
        } else {
            return Err(SpotError::NoSelection);
        };
        match registry.spots.get(&name) {
            Some(entry) => Ok(Resolved {
                name,
                site: entry.site.clone(),
                source,
            }),
            None => Err(SpotError::Unknown {
                name,
                available: registry.spots.keys().cloned().collect(),
            }),
        }
    }
}

/// Validate a spot name against the canonical slug:
/// `[a-z0-9][a-z0-9-_]*`. Names become directory names under
/// `spots/`, so the alphabet stays conservative.
pub fn validate_name(name: &str) -> Result<(), SpotError> {
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let tail_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if head_ok && tail_ok {
        Ok(())
    } else {
        Err(SpotError::InvalidName(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SpotStore) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SpotStore::at(tmp.path());
        (tmp, store)
    }

    fn registry_with(names: &[(&str, &str)], current: Option<&str>) -> Registry {
        Registry {
            current: current.map(str::to_owned),
            spots: names
                .iter()
                .map(|(name, site)| {
                    (
                        (*name).to_owned(),
                        SpotEntry {
                            site: PathBuf::from(site),
                        },
                    )
                })
                .collect(),
        }
    }

    mod loading_and_saving {
        use super::*;

        #[test]
        fn it_treats_a_missing_registry_as_empty() {
            let (_tmp, store) = store();
            let registry = store.load().expect("load");
            assert_eq!(registry, Registry::default());
        }

        #[test]
        fn it_round_trips_the_registry() {
            let (_tmp, store) = store();
            let registry = registry_with(&[("garden", "/tmp/garden")], Some("garden"));
            store.save(&registry).expect("save");
            assert_eq!(store.load().expect("load"), registry);
        }

        #[test]
        fn it_errors_on_a_corrupt_registry_instead_of_recreating() {
            let (_tmp, store) = store();
            std::fs::create_dir_all(store.registry_path().parent().unwrap()).unwrap();
            std::fs::write(store.registry_path(), "not json").unwrap();
            let err = store.load().expect_err("corrupt must not load");
            assert!(matches!(err, SpotError::Corrupt { .. }), "{err}");
            // Still corrupt afterwards — load must not have healed it.
            assert_eq!(
                std::fs::read_to_string(store.registry_path()).unwrap(),
                "not json"
            );
        }

        #[test]
        fn it_leaves_no_temp_file_behind_after_save() {
            let (_tmp, store) = store();
            store.save(&Registry::default()).expect("save");
            let leftovers: Vec<_> = std::fs::read_dir(store.registry_path().parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .filter(|n| n.to_string_lossy().ends_with(".tmp"))
                .collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");
        }
    }

    mod resolving {
        use super::*;

        #[test]
        fn it_prefers_flag_over_env_over_global() {
            let (_tmp, store) = store();
            let registry = registry_with(
                &[("a", "/s/a"), ("b", "/s/b"), ("c", "/s/c")],
                Some("c"),
            );
            store.save(&registry).expect("save");

            let flag = store.resolve(Some("a"), Some("b")).expect("flag");
            assert_eq!((flag.name.as_str(), flag.source), ("a", Source::Flag));

            let env = store.resolve(None, Some("b")).expect("env");
            assert_eq!((env.name.as_str(), env.source), ("b", Source::Env));

            let global = store.resolve(None, None).expect("global");
            assert_eq!((global.name.as_str(), global.source), ("c", Source::Global));
            assert_eq!(global.site, PathBuf::from("/s/c"));
        }

        #[test]
        fn it_errors_when_nothing_is_selected() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            let err = store.resolve(None, None).expect_err("no selection");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
        }

        #[test]
        fn it_hints_spot_new_when_the_registry_is_empty() {
            let (_tmp, store) = store();
            let err = store.resolve(None, None).expect_err("empty");
            assert!(matches!(err, SpotError::NothingRegistered), "{err}");
            assert!(err.to_string().contains("tonk spot new"), "{err}");
        }

        #[test]
        fn it_errors_on_an_unknown_name_listing_available() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            let err = store.resolve(Some("nope"), None).expect_err("unknown");
            assert!(err.to_string().contains("registered: a"), "{err}");
        }
    }

    mod naming {
        use super::*;

        #[test]
        fn it_accepts_conservative_slugs() {
            for name in ["a", "garden", "work-2", "a_b", "0day"] {
                assert!(validate_name(name).is_ok(), "{name}");
            }
        }

        #[test]
        fn it_rejects_everything_else() {
            for name in ["", "Garden", "-lead", "_lead", "sp ace", "dot.", "sl/ash", "über"] {
                assert!(validate_name(name).is_err(), "{name}");
            }
        }
    }
}
```

In `src/lib.rs`, add `pub mod spot;` in alphabetical position among the existing `pub mod` declarations, and add one line to the crate-doc module inventory near the `site` line:

```rust
//! - [`spot`] — spot registry: named spots, canonical storage, selection.
```

- [ ] **Step 2: Run the module tests, expect all green**

Run: `cargo test -p tonk-cli --lib spot::`
Expected: all `it_*` tests PASS (10 tests).

- [ ] **Step 3: Lint**

Run: `cargo clippy -p tonk-cli --all-targets -- -D warnings && cargo fmt --check`
Expected: clean. (Full `--all-features` gate runs in Task 7.)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-cli/src/spot.rs rust/tonk-cli/src/lib.rs
git commit -m "feat(tonk-cli): spot registry store and resolution"
```

---

### Task 2: `TonkSite::init_at_with` — root-directly site init

**Files:**
- Modify: `src/site.rs` (around `init_with`, line ~136)
- Test: `tests/site.rs` (new behaviour module at the end)

**Interfaces:**
- Consumes: existing `TonkSite::init_with(parent, config)` body.
- Produces: `TonkSite::init_at_with(root: &Path, config: SiteConfig) -> Result<Self>` — treats `root` itself as the site directory (creates it, no `.tonk` nesting). Tasks 4 and 5 call this. `init_with` keeps its parent+`.tonk` semantics for existing tests by delegating.

- [ ] **Step 1: Write the failing test**

At the end of `tests/site.rs`:

```rust
mod when_initializing_at_an_explicit_root {
    use anyhow::Result;
    use tonk_cli::site::TonkSite;

    use crate::common;

    /// Canonical spots put repo blocks directly in the registered
    /// site directory — no `.tonk/` nesting. `init_at_with` must
    /// root the site at exactly the path it is given.
    #[dialog_common::test]
    async fn it_roots_the_site_at_the_given_directory() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;
        let root = parent.join("spots").join("garden");

        let site = TonkSite::init_at_with(&root, config.clone()).await?;
        assert_eq!(site.root, root.canonicalize()?);
        assert!(!root.join(".tonk").exists(), "no nested .tonk");

        // Idempotent: a second init at the same root adopts the
        // existing repo instead of erroring or re-seeding.
        let reopened = TonkSite::init_at_with(&root, config.clone()).await?;
        assert_eq!(reopened.repository.did(), site.repository.did());

        // And a plain open works against it.
        let opened = TonkSite::open_with(&root, config).await?;
        assert_eq!(opened.repository.did(), site.repository.did());
        Ok(())
    }
}
```

- [ ] **Step 2: Run it, expect compile failure**

Run: `cargo test -p tonk-cli --test site when_initializing_at_an_explicit_root`
Expected: FAIL — `no function or associated item named 'init_at_with'`.

- [ ] **Step 3: Implement by refactoring `init_with`**

In `src/site.rs`, replace the current `init_with` (lines 135–187) with a thin delegator plus the new root-direct form. The body after the `root` binding is the existing code, moved verbatim:

```rust
    /// [`Self::init`] with caller-supplied [`SiteConfig`].
    ///
    /// Historical parent-relative form: the site lands at
    /// `parent/.tonk/`. Kept for the test fixtures that model the
    /// pre-registry layout; new callers go through
    /// [`Self::init_at_with`].
    pub async fn init_with(parent: &Path, config: SiteConfig) -> Result<Self> {
        let parent = parent
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", parent.display()))?;
        Self::init_at_with(&parent.join(SITE_DIRNAME), config).await
    }

    /// Initialize (or adopt) a site whose directory is exactly
    /// `root` — no `.tonk/` nesting. This is what canonical spot
    /// storage uses: the registry maps a spot name to this
    /// directory. Idempotent: an existing repository at `root` is
    /// loaded, not clobbered, which is also how `tonk spot new
    /// --site <path>` adopts pre-existing storage.
    pub async fn init_at_with(root: &Path, config: SiteConfig) -> Result<Self> {
        std::fs::create_dir_all(root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        let root = root
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", root.display()))?;

        let (profile, operator) = build_profile_and_operator(&root, &config).await?;

        // ... existing body from `init_with` continues unchanged from the
        // "Try to load first" comment through returning `Ok(site)`,
        // with `root` used directly instead of `parent.join(SITE_DIRNAME)`.
    }
```

Concretely: the existing lines from `// Try to load first.` down to `Ok(site)` move into `init_at_with` unmodified (they already operate on `root`).

- [ ] **Step 4: Run the test again, expect pass; run the whole site suite**

Run: `cargo test -p tonk-cli --test site`
Expected: new test PASSES, all existing site tests still pass (they use `init_with`, whose observable behaviour is unchanged).

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-cli/src/site.rs rust/tonk-cli/tests/site.rs
git commit -m "feat(tonk-cli): init a site directly at an explicit root"
```

---

### Task 3: Registry resolution in the binary — `--spot`, delete discovery, remove `init`

**Files:**
- Modify: `src/bin/tonk.rs`
- Modify: `src/site.rs` (delete `discover_and_open` at line 81 and `find_site_root` at line 305; delete `TonkSite::init` at line 98 — its only caller was the removed `init` handler; keep `TonkSite::open`, `open_with`, `init_with`, `init_at_with`, `SITE_DIRNAME`)
- Modify: `src/migrate.rs` doc/print only if it references discovery (check with `rg -n "discover" src/migrate.rs`; expected: no hits, no change)

**Interfaces:**
- Consumes: `spot::SpotStore`, `spot::Resolved`, `spot::SPOT_ENV` (Task 1).
- Produces: `open_selected(flag: Option<&str>) -> Result<(spot::Resolved, site::TonkSite), ExitCode>` in `bin/tonk.rs` — every site-using handler goes through it. Handlers gain a trailing `spot: Option<&str>` parameter. Task 4 and 5 reuse `open_selected` untouched.

- [ ] **Step 1: Add the global flag and the helper**

In the `Cli` struct (line 38):

```rust
struct Cli {
    /// Operate on this spot instead of the selected one.
    /// Precedence: --spot > TONK_SPOT > `tonk use` selection.
    #[arg(long, global = true, value_name = "NAME")]
    spot: Option<String>,

    #[command(subcommand)]
    command: Command,
}
```

Below `print_error` (line ~1910), add:

```rust
/// Resolve the selected spot (--spot > TONK_SPOT > `tonk use`) and
/// open its site. Every failure path names the spot and the
/// selection source so a wrong-spot mistake is visible in the
/// error itself. The cwd is never consulted.
async fn open_selected(
    flag: Option<&str>,
) -> Result<(tonk_cli::spot::Resolved, site::TonkSite), ExitCode> {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return Err(print_error(err.to_string())),
    };
    let env = std::env::var(tonk_cli::spot::SPOT_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    let resolved = match store.resolve(flag, env.as_deref()) {
        Ok(resolved) => resolved,
        Err(err) => return Err(print_error(err.to_string())),
    };
    match site::TonkSite::open(&resolved.site).await {
        Ok(site) => Ok((resolved, site)),
        Err(err) => Err(print_error(format!(
            "spot '{name}' (via {source}, site {site}): {err:#}",
            name = resolved.name,
            source = resolved.source,
            site = resolved.site.display(),
        ))),
    }
}
```

- [ ] **Step 2: Swap every discovery call site**

Each of these handlers currently opens with the same block:

```rust
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return print_error(format!("could not determine current directory: {e}")),
    };
    let site = match site::TonkSite::discover_and_open(&cwd).await {
        Ok(s) => s,
        Err(err) => return print_error(err.to_string()),
    };
```

Replace it in every case with:

```rust
    let (_, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
```

and add a trailing `spot: Option<&str>` parameter to the handler. The full list (current line of the `cwd` block): `eval` (872), `sync_op` (959), `export_op` (986), `render_op` (1022), `import_op` (1050), `status_op` (1095), `remote_op` (1137), `blob_op` (1241), `share_op` (1306), `mint_invite` (1443), `query_op` (1576), `get_op` (1603), `assert_cmd` (1638), `retract_op` (1665), `concept_op` (1691), `view_op` (1728), `home_op` (1782), `print_schema` (1829).

`status_op` is the one non-uniform case — it keeps the `Resolved` and prints it (spec: status output names the resolved spot):

```rust
async fn status_op(spot: Option<&str>) -> ExitCode {
    let (resolved, site) = match open_selected(spot).await {
        Ok(opened) => opened,
        Err(code) => return code,
    };
    println!(
        "spot: {name} ({source})",
        name = resolved.name,
        source = resolved.source,
    );

    match sync::status(&site).await {
        // ... existing body unchanged
    }
}
```

In `main`'s dispatch (line 763), bind the flag before the `match` and thread it: `let spot = cli.spot;` then e.g. `Command::Eval(args) => eval(args, spot.as_deref()).await,` for every handler in the list. `claim_invite` (join) and `migrate` stay as they are until Task 5 / stay cwd-based respectively.

- [ ] **Step 3: Remove `tonk init`**

- Delete the `Command::Init` variant (line ~267) and the `init` handler (line ~830).
- Delete `Command::Init { .. } => ("init", None),` from `descriptor` (line 671).
- Delete the dispatch arm `Command::Init { label } => init(label).await,`.
- In `src/site.rs`: delete `TonkSite::init` (line 98), `TonkSite::discover_and_open` (line 81), and `find_site_root` (line 305). Update the module doc header (lines 1–11) to say a site is a directory holding the repo, located via the spot registry rather than cwd discovery, and update the `lib.rs` crate-doc line for `site` (line 10: currently "`.tonk/` discovery, repo+branch open/init") to match.
- Update the binary's help copy: the crate doc comment at the top of `bin/tonk.rs` (lines 1–8, mentions `init` and "local `.tonk/` site") and the `after_help` on `Cli` (line 36): change the setup line to `setup    spot · use · blob · telemetry` and reword "against the local `.tonk/` site" to "against the selected spot".

- [ ] **Step 4: Build and test**

Run: `cargo build -p tonk-cli && cargo test -p tonk-cli`
Expected: compiles; all tests pass (integration tests drive `TonkSite` directly and never used discovery).

Smoke check the no-selection error:

```bash
TONK_SPOTS_STATE=$(mktemp -d) cargo run -p tonk-cli --bin tonk -- status
```
Expected stderr: `error: no spots registered; create one with `tonk spot new <name>`` and non-zero exit.

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy -p tonk-cli --all-targets -- -D warnings && cargo fmt --check`

```bash
git add rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/src/site.rs rust/tonk-cli/src/lib.rs
git commit -m "feat(tonk-cli): resolve every command through the spot registry"
```

---

### Task 4: Spot management commands — `use`, `spot new|list|rm`

**Files:**
- Modify: `src/spot.rs` (append the ops section below)
- Modify: `src/bin/tonk.rs` (new commands + printing)
- Create: `tests/spot.rs`
- Modify: `Cargo.toml` (add the `[[test]]` entry)

**Interfaces:**
- Consumes: `SpotStore`, `Registry`, `SpotEntry`, `validate_name`, `SpotError` (Task 1); `TonkSite::init_at_with`, `SiteConfig` (Task 2).
- Produces: `spot::create(&SpotStore, name, site_override: Option<&Path>, config: SiteConfig) -> Result<CreateOutcome, SpotError>` with `CreateOutcome { name, site, did }`; `spot::select(&SpotStore, name) -> Result<Resolved, SpotError>` (source always `Global`); `spot::listing(&SpotStore, flag, env) -> Result<Listing, SpotError>` with `Listing { rows: Vec<(String, PathBuf)>, current: Option<Resolved> }`; `spot::remove(&SpotStore, name, delete: bool) -> Result<RemoveOutcome, SpotError>` with `RemoveOutcome { name, site, deleted }`. Task 5 reuses `create`'s register-and-select tail pattern via `register` below.

- [ ] **Step 1: Write the failing integration test**

`tests/spot.rs`:

```rust
//! Spot management ops: create/register/select/list/remove against
//! an isolated store. These exercise the `spot` module's ops layer
//! the way the `tonk use` / `tonk spot *` commands drive it —
//! nothing here touches process env or the user's data dir.

mod common;

use anyhow::Result;
use tonk_cli::site::TonkSite;
use tonk_cli::spot::{self, SpotStore};

mod when_creating_a_spot {
    use super::*;

    #[dialog_common::test]
    async fn it_creates_registers_and_selects_in_the_canonical_dir() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        let outcome = spot::create(&store, "garden", None, config.clone()).await?;
        assert_eq!(outcome.site, store.canonical_site("garden").canonicalize()?);

        // Registered and selected: a bare resolve now finds it.
        let resolved = store.resolve(None, None)?;
        assert_eq!(resolved.name, "garden");
        assert_eq!(resolved.site, outcome.site);

        // And the site actually opens.
        let opened = TonkSite::open_with(&resolved.site, config).await?;
        assert_eq!(opened.repository.did().to_string(), outcome.did);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_adopts_an_existing_site_via_site_override() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        // A pre-existing site (the old cwd-`.tonk` migration case).
        let legacy_root = parent.join("proj").join(".tonk");
        let legacy = TonkSite::init_at_with(&legacy_root, config.clone()).await?;

        let outcome =
            spot::create(&store, "proj", Some(&legacy_root), config.clone()).await?;
        assert_eq!(outcome.did, legacy.repository.did().to_string());
        assert_eq!(outcome.site, legacy_root.canonicalize()?);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_duplicate_name() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        spot::create(&store, "garden", None, config.clone()).await?;
        let err = spot::create(&store, "garden", None, config)
            .await
            .expect_err("duplicate");
        assert!(matches!(err, spot::SpotError::Exists(_)), "{err}");
        Ok(())
    }
}

mod when_removing_a_spot {
    use super::*;

    #[dialog_common::test]
    async fn it_unregisters_but_keeps_data_by_default() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let created = spot::create(&store, "garden", None, config).await?;

        let outcome = spot::remove(&store, "garden", false)?;
        assert!(!outcome.deleted);
        assert!(created.site.exists(), "data kept");
        // Entry and selection are gone.
        assert!(store.load()?.spots.is_empty());
        assert!(store.load()?.current.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_deletes_the_site_dir_with_delete() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let created = spot::create(&store, "garden", None, config).await?;

        let outcome = spot::remove(&store, "garden", true)?;
        assert!(outcome.deleted);
        assert!(!created.site.exists(), "data removed");
        Ok(())
    }
}

mod when_selecting_and_listing {
    use super::*;

    #[dialog_common::test]
    async fn it_selects_by_name_and_lists_with_the_resolved_current() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        spot::create(&store, "a", None, config.clone()).await?;
        spot::create(&store, "b", None, config).await?; // create selects b

        let selected = spot::select(&store, "a")?;
        assert_eq!(selected.name, "a");
        assert_eq!(store.load()?.current.as_deref(), Some("a"));

        let listing = spot::listing(&store, None, None)?;
        assert_eq!(
            listing.rows.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(listing.current.as_ref().map(|c| c.name.as_str()), Some("a"));

        let err = spot::select(&store, "nope").expect_err("unknown");
        assert!(matches!(err, spot::SpotError::Unknown { .. }), "{err}");
        Ok(())
    }
}
```

In `Cargo.toml`, after the `authoring` entry:

```toml
[[test]]
name = "spot"
path = "tests/spot.rs"
```

- [ ] **Step 2: Run it, expect compile failure**

Run: `cargo test -p tonk-cli --test spot`
Expected: FAIL — `cannot find function 'create' in module 'spot'` (and friends).

- [ ] **Step 3: Implement the ops in `src/spot.rs`**

Append after `validate_name` (before `mod tests`):

```rust
/// Outcome of [`create`]: the registered spot and the DID of the
/// repository backing it.
#[derive(Debug, Clone)]
pub struct CreateOutcome {
    /// Registry name.
    pub name: String,
    /// Absolute site directory.
    pub site: PathBuf,
    /// The site repository's DID.
    pub did: String,
}

/// Outcome of [`remove`].
#[derive(Debug, Clone)]
pub struct RemoveOutcome {
    /// The removed registry name.
    pub name: String,
    /// Where the site lived (still lives, unless `deleted`).
    pub site: PathBuf,
    /// Whether the site directory was deleted from disk.
    pub deleted: bool,
}

/// Rows for `tonk spot list` plus the resolved current selection
/// (None when nothing resolves — empty registry or dangling name).
#[derive(Debug, Clone)]
pub struct Listing {
    /// `(name, site)` per registered spot, in name order.
    pub rows: Vec<(String, PathBuf)>,
    /// The spot a bare command would hit right now, with source.
    pub current: Option<Resolved>,
}

/// Create (or adopt) a spot: initialize the site, register the
/// name, and select it. The site lands in the store's canonical
/// `spots/<name>/` unless `site_override` names another directory;
/// because [`crate::site::TonkSite::init_at_with`] is idempotent,
/// an override pointing at existing site storage adopts it — the
/// migration path for pre-registry `.tonk/` dirs.
pub async fn create(
    store: &SpotStore,
    name: &str,
    site_override: Option<&Path>,
    config: crate::site::SiteConfig,
) -> Result<CreateOutcome, SpotError> {
    validate_name(name)?;
    let mut registry = store.load()?;
    if registry.spots.contains_key(name) {
        return Err(SpotError::Exists(name.to_owned()));
    }

    let target = site_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.canonical_site(name));
    let site = crate::site::TonkSite::init_at_with(&target, config)
        .await
        .map_err(|e| SpotError::Init(format!("{e:#}")))?;

    let outcome = CreateOutcome {
        name: name.to_owned(),
        site: site.root.clone(),
        did: site.repository.did().to_string(),
    };
    registry.spots.insert(
        name.to_owned(),
        SpotEntry {
            site: outcome.site.clone(),
        },
    );
    registry.current = Some(name.to_owned());
    store.save(&registry)?;
    Ok(outcome)
}

/// Set the registry's `current` to `name`. This is the human
/// selection path (`tonk use`); automation pins spots per-process
/// via [`SPOT_ENV`] / `--spot` instead.
pub fn select(store: &SpotStore, name: &str) -> Result<Resolved, SpotError> {
    let mut registry = store.load()?;
    let Some(entry) = registry.spots.get(name) else {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
        });
    };
    let resolved = Resolved {
        name: name.to_owned(),
        site: entry.site.clone(),
        source: Source::Global,
    };
    registry.current = Some(name.to_owned());
    store.save(&registry)?;
    Ok(resolved)
}

/// Everything `tonk spot list` needs in one read: the rows plus
/// what a bare command would currently resolve to (honouring the
/// same `flag`/`env` precedence, so `tonk --spot x spot list`
/// marks `x`).
pub fn listing(
    store: &SpotStore,
    flag: Option<&str>,
    env: Option<&str>,
) -> Result<Listing, SpotError> {
    let registry = store.load()?;
    let rows = registry
        .spots
        .iter()
        .map(|(name, entry)| (name.clone(), entry.site.clone()))
        .collect();
    Ok(Listing {
        rows,
        current: store.resolve(flag, env).ok(),
    })
}

/// Remove `name` from the registry, clearing `current` if it
/// pointed there. Site data stays on disk unless `delete` — the
/// registry is the authority on names, not a lifecycle manager
/// for storage it didn't necessarily create.
pub fn remove(store: &SpotStore, name: &str, delete: bool) -> Result<RemoveOutcome, SpotError> {
    let mut registry = store.load()?;
    let Some(entry) = registry.spots.remove(name) else {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
        });
    };
    if registry.current.as_deref() == Some(name) {
        registry.current = None;
    }
    store.save(&registry)?;
    let deleted = if delete {
        std::fs::remove_dir_all(&entry.site).map_err(|e| {
            SpotError::Io(format!(
                "entry removed, but deleting {} failed: {e}",
                entry.site.display()
            ))
        })?;
        true
    } else {
        false
    };
    Ok(RemoveOutcome {
        name: name.to_owned(),
        site: entry.site,
        deleted,
    })
}
```

- [ ] **Step 4: Run the integration test, expect pass**

Run: `cargo test -p tonk-cli --test spot`
Expected: 6 tests PASS.

- [ ] **Step 5: Wire the CLI commands**

In `bin/tonk.rs`, add to the `Command` enum (in the `-- setup --` section where `Init` used to be):

```rust
    /// Select the current spot (used by every command from anywhere)
    ///
    /// The selection is global to this machine. Concurrent sessions
    /// (agents, CI) should pin their spot per-process with --spot or
    /// TONK_SPOT instead of relying on it.
    #[command(after_help = "Examples:\n  tonk use garden")]
    Use {
        /// A registered spot name (see `tonk spot list`).
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Manage spots: named, centrally registered fact stores
    Spot {
        #[command(subcommand)]
        command: SpotCommand,
    },
```

and the subcommand enum next to `RemoteCommand`:

```rust
#[derive(Subcommand, Debug)]
enum SpotCommand {
    /// Create (or adopt) a spot, register it, and select it
    ///
    /// The site lands in the canonical store
    /// (`~/Library/Application Support/tonk/spots/<name>` on macOS)
    /// unless --site points elsewhere. --site aimed at an existing
    /// site directory adopts it instead of creating fresh — the
    /// migration path for pre-registry `.tonk/` dirs.
    #[command(
        after_help = "Examples:\n  tonk spot new garden\n  tonk spot new work --site ~/work/site\n  tonk spot new proj --site ~/proj/.tonk"
    )]
    New {
        /// Spot name ([a-z0-9][a-z0-9-_]*).
        #[arg(value_name = "NAME")]
        name: String,
        /// Store the site at this directory instead of the
        /// canonical location.
        #[arg(long, value_name = "PATH")]
        site: Option<PathBuf>,
    },

    /// List registered spots and the current selection
    #[command(after_help = "Examples:\n  tonk spot list")]
    List,

    /// Remove a spot from the registry (data stays unless --delete)
    #[command(after_help = "Examples:\n  tonk spot rm garden\n  tonk spot rm garden --delete")]
    Rm {
        /// Spot name to unregister.
        #[arg(value_name = "NAME")]
        name: String,
        /// Also delete the site directory from disk.
        #[arg(long)]
        delete: bool,
    },
}
```

`descriptor` additions:

```rust
        Command::Use { .. } => ("use", None),
        Command::Spot { command } => (
            "spot",
            Some(match command {
                SpotCommand::New { .. } => "new",
                SpotCommand::List => "list",
                SpotCommand::Rm { .. } => "rm",
            }),
        ),
```

Dispatch arms:

```rust
        Command::Use { name } => use_op(name).await,
        Command::Spot { command } => spot_op(command, spot.as_deref()).await,
```

Handlers (near the other setup handlers; `use_op` is async only for signature uniformity in the dispatch — the ops are sync):

```rust
/// `tonk use` — set the global current spot.
async fn use_op(name: String) -> ExitCode {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_error(err.to_string()),
    };
    match tonk_cli::spot::select(&store, &name) {
        Ok(resolved) => {
            println!(
                "current spot: {name} ({site})",
                name = resolved.name,
                site = resolved.site.display(),
            );
            ExitCode::Success
        }
        Err(err) => print_error(err.to_string()),
    }
}

/// `tonk spot new|list|rm` — registry management.
async fn spot_op(command: SpotCommand, flag: Option<&str>) -> ExitCode {
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_error(err.to_string()),
    };
    match command {
        SpotCommand::New { name, site } => {
            match tonk_cli::spot::create(&store, &name, site.as_deref(), site::default_config())
                .await
            {
                Ok(outcome) => {
                    println!("Registered spot '{}'", outcome.name);
                    println!("site: {}", outcome.site.display());
                    println!("DID: {}", outcome.did);
                    println!("current spot: {}", outcome.name);
                    ExitCode::Success
                }
                Err(err) => print_error(err.to_string()),
            }
        }
        SpotCommand::List => {
            let env = std::env::var(tonk_cli::spot::SPOT_ENV)
                .ok()
                .filter(|value| !value.is_empty());
            match tonk_cli::spot::listing(&store, flag, env.as_deref()) {
                Ok(listing) => {
                    if listing.rows.is_empty() {
                        println!("(no spots registered; create one with `tonk spot new <name>`)");
                        return ExitCode::Success;
                    }
                    let current = listing.current.as_ref().map(|c| c.name.as_str());
                    for (name, site) in &listing.rows {
                        let marker = if Some(name.as_str()) == current { '*' } else { ' ' };
                        println!("{marker} {name}\t{site}", site = site.display());
                    }
                    if let Some(resolved) = &listing.current {
                        println!(
                            "current: {name} ({source})",
                            name = resolved.name,
                            source = resolved.source,
                        );
                    }
                    ExitCode::Success
                }
                Err(err) => print_error(err.to_string()),
            }
        }
        SpotCommand::Rm { name, delete } => {
            match tonk_cli::spot::remove(&store, &name, delete) {
                Ok(outcome) => {
                    println!("Removed spot '{}' from the registry", outcome.name);
                    if outcome.deleted {
                        println!("site deleted: {}", outcome.site.display());
                    } else {
                        println!("site kept at {}", outcome.site.display());
                    }
                    ExitCode::Success
                }
                Err(err) => print_error(err.to_string()),
            }
        }
    }
}
```

Note `spot_op` receives the global `--spot` flag value purely so `list` can mark what a bare command would resolve to.

- [ ] **Step 6: End-to-end smoke against an isolated store**

```bash
STATE=$(mktemp -d)
t() { env TONK_SPOTS_STATE="$STATE" cargo run -q -p tonk-cli --bin tonk -- "$@"; }
t spot new garden          # Registered spot 'garden' ... current spot: garden
t spot list                # * garden  <path>  /  current: garden (global)
t status                   # spot: garden (global) + no-upstream
t spot rm garden           # Removed ... site kept at <path>
t status                   # error: no spot selected... (registry has 0 spots → spot new hint)
```
Expected: as annotated. (`t status` after `rm` reports the empty-registry hint.)

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy -p tonk-cli --all-targets -- -D warnings && cargo fmt --check && cargo test -p tonk-cli`

```bash
git add rust/tonk-cli/src/spot.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/spot.rs rust/tonk-cli/Cargo.toml
git commit -m "feat(tonk-cli): tonk use and tonk spot new/list/rm"
```

---

### Task 5: `tonk join` lands in a canonical spot

**Files:**
- Modify: `src/invite.rs` (`claim`, line 224; `InviteError::SiteAlreadyExists`, line 93)
- Modify: `src/bin/tonk.rs` (`Join` command, line ~243; `claim_invite`, line 1493; `print_claim_outcome`, line 1512)
- Modify: `tests/site.rs` (the four `invite::claim` call sites at lines 162, 193, 229, 289, 359, 382, 397 — see step 4)

**Interfaces:**
- Consumes: `spot::{SpotStore, SpotEntry, validate_name, SpotError}` (Tasks 1/4).
- Produces: `invite::claim(root: &Path, invite_url: &str, config: SiteConfig)` — `root` is now the site directory itself, not the parent that gets `.tonk/` appended.

- [ ] **Step 1: Change `claim` to take the site root directly**

In `src/invite.rs`, replace the head of `claim` (lines 224–235):

```rust
/// Claim an invite, bootstrapping a fresh site at `root` (the
/// site directory itself — the caller picks it, typically the
/// canonical `spots/<name>/` dir) whose repository targets the
/// invited subject DID.
///
/// Steps:
///
/// 1. Refuse if `root` already exists — the join must never
///    clobber existing site storage.
/// 2. Parse the URL via [`Invite::parse_url`]; reject malformed
///    invites before touching disk.
/// 3. Stand up the on-disk site directory and build a tonk
///    operator rooted there, opening (or creating) the local
///    profile.
/// ... (steps 4–6 of the existing doc comment unchanged)
pub async fn claim(
    root: &Path,
    invite_url: &str,
    config: SiteConfig,
) -> Result<ClaimOutcome, InviteError> {
    if root.exists() {
        return Err(InviteError::SiteAlreadyExists(root.to_path_buf()));
    }
```

Then, where the old body did `let root = parent.join(SITE_DIRNAME);` + existence check: delete those lines. Move the existing `std::fs::create_dir_all(&root)` (line 254) up as-is, and canonicalize after creation so the operator gets an absolute root (the old code canonicalized `parent` up front):

```rust
    std::fs::create_dir_all(root)
        .map_err(|e| InviteError::Io(format!("failed to create {}: {e}", root.display())))?;
    let root = root.canonicalize().map_err(|e| {
        InviteError::Io(format!("could not canonicalize {}: {e}", root.display()))
    })?;
```

(The shortcut-resolution and parse block between the guard and `create_dir_all` stays where it is — parse before touching disk.) The rest of the body already uses `&root`. Update the error text at line 93:

```rust
    /// `claim` was asked to bootstrap a site directory that
    /// already exists. The join must never clobber existing site
    /// storage; the user removes it (or picks another spot name)
    /// first.
    #[error("a site already exists at {0}; remove it or pick another spot name")]
    SiteAlreadyExists(PathBuf),
```

Remove the now-unused `SITE_DIRNAME` import from `src/invite.rs` (line 30).

- [ ] **Step 2: Rework the `Join` command**

Command definition (line ~243) gains a required `--name`:

```rust
    /// Join a shared repo from an invite URL into a new spot
    #[command(after_help = "Examples:\n  tonk join 'https://...#invite' --name garden")]
    Join {
        /// The invite URL (quote it - the #fragment matters).
        #[arg(value_name = "URL")]
        url: String,
        /// Spot name to register the joined repo under.
        #[arg(long, value_name = "NAME")]
        name: String,
    },
```

Dispatch: `Command::Join { url, name } => claim_invite(url, name).await,`.

Replace `claim_invite` and `print_claim_outcome` (lines 1493–1525):

```rust
/// `tonk join` — claim an invite into a fresh canonical spot:
/// site at `spots/<name>/`, registered and selected on success.
/// Registration happens only after the claim succeeds, so a
/// failed join never leaves a dangling registry entry (a partial
/// site dir may remain; re-running with the same name reports it).
async fn claim_invite(url: String, name: String) -> ExitCode {
    if let Err(err) = tonk_cli::spot::validate_name(&name) {
        return print_error(err.to_string());
    }
    let store = match tonk_cli::spot::SpotStore::open() {
        Ok(store) => store,
        Err(err) => return print_error(err.to_string()),
    };
    let mut registry = match store.load() {
        Ok(registry) => registry,
        Err(err) => return print_error(err.to_string()),
    };
    if registry.spots.contains_key(&name) {
        return print_error(tonk_cli::spot::SpotError::Exists(name).to_string());
    }
    let root = store.canonical_site(&name);

    // Same default site config `tonk spot new` writes against, so
    // the joined site picks up the user's normal profile.
    match invite::claim(&root, &url, site::default_config()).await {
        Ok(outcome) => {
            registry.spots.insert(
                name.clone(),
                tonk_cli::spot::SpotEntry { site: root.clone() },
            );
            registry.current = Some(name.clone());
            if let Err(err) = store.save(&registry) {
                return print_error(format!(
                    "joined, but registering spot '{name}' failed: {err}\n\
                     re-register with `tonk spot new {name} --site {root}`",
                    root = root.display(),
                ));
            }
            print_claim_outcome(&name, &root, &outcome);
            ExitCode::Success
        }
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn print_claim_outcome(name: &str, root: &std::path::Path, outcome: &ClaimOutcome) {
    println!("Joined spot '{name}' ({})", root.display());
    println!("subject: {}", outcome.subject);
    if let Some(remote) = &outcome.auto_configured_remote
        && let Some(url) = &outcome.remote_url
    {
        println!("remote:  {remote} -> {url}");
        if outcome.synced {
            println!("synced:  pulled current state from {remote}");
        } else {
            println!("synced:  no (run `tonk pull` before making changes)");
        }
    }
    println!("current spot: {name}");
}
```

- [ ] **Step 3: Point `tonk migrate` output at the registry**

In `bin/tonk.rs`'s `migrate` handler (line ~1527), after the existing `println!("DID: ...")`, add:

```rust
            println!(
                "register it as a spot: `tonk spot new <name> --site {}`",
                outcome.destination.display()
            );
```

(`migrate` keeps its explicit cwd destination — it is a legacy, path-explicit import; the registry hint closes the loop.)

- [ ] **Step 4: Update the claim call sites in `tests/site.rs`**

Every `invite::claim(&claimer_parent, ...)` call passes a parent today and then opens `claimer_parent.join(SITE_DIRNAME)`. Switch each to an explicit root. Pattern (apply at lines 162, 193, 229, 289, 359, 382, 397, adjusting variable names in context):

```rust
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;
```

and each later `TonkSite::open_with(&claimer_parent.join(SITE_DIRNAME), ...)` becomes `TonkSite::open_with(&claimer_root, ...)`. The double-claim test (line ~382) claims twice against the same root and must now assert the `SiteAlreadyExists` error text `"a site already exists"`. Drop the `SITE_DIRNAME` import from the test file if it becomes unused.

- [ ] **Step 5: Run the affected suites**

Run: `cargo test -p tonk-cli --test site && cargo test -p tonk-cli`
Expected: all pass. (The `integration-tests`-gated shortcut test compiles against the new signature; if the feature isn't runnable locally, `cargo check -p tonk-cli --tests --features integration-tests` must still pass.)

- [ ] **Step 6: Lint and commit**

Run: `cargo clippy -p tonk-cli --all-targets --all-features -- -D warnings && cargo fmt --check`

```bash
git add rust/tonk-cli/src/invite.rs rust/tonk-cli/src/bin/tonk.rs rust/tonk-cli/tests/site.rs
git commit -m "feat(tonk-cli): join claims into a registered canonical spot"
```

---

### Task 6: Docs and agent guidance

**Files:**
- Modify: `README.md`
- Modify: `src/guide-index.md`

**Interfaces:** none — copy only.

- [ ] **Step 1: README**

- Line 5: replace "it operates on a `.tonk/` site" framing with: `tonk` operates on the selected **spot** — a named fact store resolved through a central registry, so the CLI works from any directory.
- Lines 16–17 (quickstart): replace

  ```bash
  # Initialize a .tonk/ repo in the current directory.
  tonk init
  ```

  with

  ```bash
  # Create a spot (stored canonically, e.g. ~/Library/Application Support/tonk/spots/garden).
  tonk spot new garden
  # Later, from anywhere:
  tonk use garden
  ```

- Section "### The `.tonk/` site" (line 77): retitle to "### Spots and sites" and rewrite the body: a **spot** is a named entry in `spots.json` under the platform data dir; its **site** is the directory holding the dialog repository (canonical `spots/<name>/`, or anywhere via `tonk spot new --site <path>`); resolution is `--spot` > `TONK_SPOT` > `tonk use` selection; the registry file is plain JSON any application can read. Mention adopting a legacy `.tonk/` dir with `tonk spot new proj --site ~/proj/.tonk`.
- Line 112: update "`tonk join` claims one into a fresh `.tonk/`" to "claims one into a fresh spot (`tonk join <url> --name <spot>`)".

- [ ] **Step 2: Guide (agent-facing)**

In `src/guide-index.md`, add a short section (near the orient material) titled `## Spots`:

```markdown
## Spots

Commands run against the selected *spot* (a named fact store), not
the cwd. Resolution order: `--spot <name>` > `TONK_SPOT` env >
`tonk use <name>` selection. In automation, always pin the spot
per-process (`TONK_SPOT=x tonk ...` or `--spot x`) — never rely on
`tonk use`, which is shared global state another session can change.
`tonk spot list` shows what's registered and what is current.
```

- [ ] **Step 3: Verify the guide still ships and commit**

Run: `cargo test -p tonk-cli && cargo run -q -p tonk-cli --bin tonk -- guide | head -40`
Expected: tests pass; the guide index prints and includes the Spots section.

```bash
git add rust/tonk-cli/README.md rust/tonk-cli/src/guide-index.md
git commit -m "docs(tonk-cli): document spots, registry, and automation pinning"
```

---

### Task 7: Full gate and final smoke

**Files:** none new.

- [ ] **Step 1: Workspace lint gate (matches CI)**

Run, from the workspace root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
Expected: both clean. Fix anything surfaced (wasm-gated helpers, dead code from the deleted discovery path) before proceeding.

- [ ] **Step 2: Full native test run**

Run: `cargo nextest run -p tonk-cli` (or `cargo test -p tonk-cli`)
Expected: all suites pass, including the new `spot` binary.

- [ ] **Step 3: Cold-start smoke, isolated store**

```bash
STATE=$(mktemp -d)
t() { env TONK_SPOTS_STATE="$STATE" cargo run -q -p tonk-cli --bin tonk -- "$@"; }
t query task                      # error: no spots registered → spot new hint
t spot new garden
t concept add task title:text     # works from any cwd
t assert task --title "hello"
t query task                      # one row
TONK_SPOT=garden t status         # spot: garden (env)
t --spot garden status            # spot: garden (flag)
t spot rm garden --delete
```
Expected: annotated behaviour; `rm --delete` leaves `$STATE/spots` empty.

- [ ] **Step 4: Commit any gate fixes**

```bash
git add -u
git commit -m "chore(tonk-cli): satisfy workspace lint gate for spot registry"
```
(Skip the commit if the gate was already clean.)
