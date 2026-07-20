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
        std::fs::rename(&tmp, &path)
            .map_err(|e| SpotError::Io(format!("could not move {} into place: {e}", tmp.display())))
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
    let tail_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if head_ok && tail_ok {
        Ok(())
    } else {
        Err(SpotError::InvalidName(name.to_owned()))
    }
}

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
            let registry = registry_with(&[("a", "/s/a"), ("b", "/s/b"), ("c", "/s/c")], Some("c"));
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
            for name in [
                "", "Garden", "-lead", "_lead", "sp ace", "dot.", "sl/ash", "über",
            ] {
                assert!(validate_name(name).is_err(), "{name}");
            }
        }
    }
}
