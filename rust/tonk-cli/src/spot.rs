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
//!   spots.json      registry: name → site path plus directory
//!                   bindings
//!   spots/<name>/   canonical site dirs
//! ```
//!
//! Selection resolves `--spot` > `TONK_SPOT` > a directory
//! binding (the nearest bound ancestor of the cwd). The flag and env
//! forms are per-invocation / per-process; bindings persist that
//! choice for sessions that live in a directory. A directory is only
//! ever a key into the registry — it never locates or contains spot
//! data.
//!
//! `spots.json` stores absolute, expanded paths so applications
//! built on tonk can resolve a name with zero path logic. Writes
//! go through a temp file + atomic rename. A corrupt registry is a
//! hard error naming the file — never silently recreated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dialog_query::{Output as _, Query, Term};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_schema::RepositoryName;
use tonk_schema::domain::repo;
use tonk_schema::prelude::DidExt as _;

/// Environment variable naming the spot to use, beaten only by the
/// `--spot` flag. Automation (agents, bench, CI) should always set
/// this (or pass `--spot`) to override a directory binding.
pub const SPOT_ENV: &str = "TONK_SPOT";

/// Environment variable overriding the directory that holds
/// `spots.json` and the canonical `spots/` root, so tests can
/// isolate state (same pattern as `TONK_TELEMETRY_STATE`).
pub const STATE_ENV: &str = "TONK_SPOTS_STATE";

/// File name of the registry inside the store directory.
const REGISTRY_FILE: &str = "spots.json";

/// Directory name (inside the store) holding canonical site dirs.
const SPOTS_DIRNAME: &str = "spots";

/// On-disk registry: one entry per spot plus directory bindings.
/// `BTreeMap` keeps listing and serialization order stable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    /// Compatibility sink for registries written before directory
    /// bindings replaced the machine-global selection. It is never
    /// consulted or written back.
    #[serde(default, rename = "current", skip_serializing)]
    legacy_current: Option<String>,
    /// Name → entry. Paths inside are absolute and expanded.
    #[serde(default)]
    pub spots: BTreeMap<String, SpotEntry>,
    /// Directories bound to a spot, keyed by canonicalized absolute
    /// path. A top-level map rather than a list inside each entry: a
    /// key cannot repeat, so "one directory, one spot" is structural
    /// rather than enforced.
    #[serde(
        default,
        alias = "attachments",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub bindings: BTreeMap<PathBuf, String>,
    /// Fields this binary does not recognise. `spots.json` is a public
    /// format other applications read and rewrite directly, and this
    /// binary is not necessarily the newest one touching it — an
    /// older `tonk` (stable channel, pre-`tonk update`) or a
    /// third-party writer must not silently drop a field it has never
    /// heard of just because it round-tripped the registry.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `--spot` flag.
    Flag,
    /// [`SPOT_ENV`] environment variable.
    Env,
    /// A binding on the cwd or one of its ancestors. Carries the
    /// bound directory, so output can say *which* one answered.
    Directory(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Flag => f.write_str("flag"),
            Source::Env => f.write_str("env"),
            Source::Directory(directory) => write!(f, "directory {}", directory.display()),
        }
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
    /// Spots exist but neither the process nor cwd selects one.
    #[error(
        "no spot active for this directory; run `tonk use <name>`, \
         pass --spot, or set TONK_SPOT"
    )]
    NoSelection,
    /// The registry has zero spots — selection is moot; the fix is
    /// creating one.
    #[error(
        "no spots registered; create one with `tonk spot new <name>` \
         (add --site <path> to adopt an existing .tonk directory)"
    )]
    NothingRegistered,
    /// A name that isn't in the registry.
    #[error("unknown spot '{name}'{}", unknown_hint(.available, .binding))]
    Unknown {
        /// The name that failed to resolve.
        name: String,
        /// Every registered name, for the error hint.
        available: Vec<String>,
        /// The bound directory that produced this name, when
        /// resolution got here via [`Source::Directory`]. `None` for
        /// the flag/env cases, which have no directory to blame and
        /// no `unbind` to suggest.
        binding: Option<PathBuf>,
    },
    /// `spot unbind` against a directory with no binding of its
    /// own. Matching is exact on purpose — unbinding a whole project
    /// because someone typed `unbind` three levels down inside it is
    /// not a recoverable surprise — so the ancestor that *is*
    /// bound goes in the message instead.
    #[error("no binding at {directory}{}", unbind_hint(.ancestor))]
    NotBound {
        /// The directory that was asked about.
        directory: PathBuf,
        /// The nearest bound ancestor, for the error hint.
        ancestor: Option<(PathBuf, String)>,
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

/// Hint suffix for [`SpotError::Unknown`]: list what is registered,
/// or point at `spot new` when nothing is; when the name came from a
/// directory binding, name the directory too and point at `spot
/// unbind` — otherwise the error reads as coming from nowhere, and
/// there is no obvious way to clear it.
fn unknown_hint(available: &[String], binding: &Option<PathBuf>) -> String {
    let registered = if available.is_empty() {
        "; none registered (create one with `tonk spot new <name>`)".to_string()
    } else {
        format!("; registered: {}", available.join(", "))
    };
    match binding {
        Some(directory) => format!(
            "{registered}; via binding at {directory} — clear it with `tonk spot unbind {directory}`",
            directory = directory.display(),
        ),
        None => registered,
    }
}

/// Hint suffix for [`SpotError::NotBound`]: name the bound ancestor,
/// so the fix is a `cd` away.
fn unbind_hint(ancestor: &Option<(PathBuf, String)>) -> String {
    match ancestor {
        Some((directory, name)) => {
            format!("; {} is bound to {name}", directory.display())
        }
        None => String::new(),
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

    /// Dedicated account-system repository directory.
    ///
    /// It is a sibling of `spots/`, never a registered spot and never written
    /// to `spots.json`.
    pub fn account_dir(&self) -> PathBuf {
        self.dir.join("account")
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
    /// already read and empty-filtered by the caller) > a directory
    /// binding at or above `cwd`.
    ///
    /// `cwd` is passed in rather than read here so nothing depends on
    /// process-global state, and it is only ever a key into the
    /// registry: the directory never locates site data.
    ///
    /// `SPOT_ENV` outranks bindings deliberately. A harness that
    /// pinned a spot for the process must not be overridden by
    /// whatever directory it happened to launch in.
    pub fn resolve(
        &self,
        flag: Option<&str>,
        env: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<Resolved, SpotError> {
        let registry = self.load()?;
        let (name, source) = if let Some(name) = flag {
            (name.to_owned(), Source::Flag)
        } else if let Some(name) = env {
            (name.to_owned(), Source::Env)
        } else if let Some((directory, name)) =
            cwd.and_then(|cwd| directory_binding(&registry, cwd))
        {
            (name, Source::Directory(directory))
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
            None => {
                // Name the bound directory in the error too, when
                // that is where the name came from — otherwise an
                // orphaned binding (the registered spot was
                // removed by hand or by `spot rm` on another
                // machine) reads as an unexplained failure with no
                // way to clear it.
                let binding = match &source {
                    Source::Directory(directory) => Some(directory.clone()),
                    _ => None,
                };
                Err(SpotError::Unknown {
                    name,
                    available: registry.spots.keys().cloned().collect(),
                    binding,
                })
            }
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

/// Canonicalize a path for use as a binding key, falling back to
/// the path as given when the filesystem refuses (most often: the
/// directory has been deleted). A key that cannot be canonicalized
/// simply never matches a canonicalized cwd, which is the right
/// outcome for a directory that no longer exists — the binding
/// tier is skipped and resolution falls through.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The nearest binding at or above `cwd`: start at the directory
/// itself and climb to the root, taking the first hit, so a nested
/// directory overrides its parent.
fn directory_binding(registry: &Registry, cwd: &Path) -> Option<(PathBuf, String)> {
    if registry.bindings.is_empty() {
        return None;
    }
    let cwd = canonical(cwd);
    cwd.ancestors().find_map(|dir| {
        registry
            .bindings
            .get_key_value(dir)
            .map(|(path, name)| (path.clone(), name.clone()))
    })
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
    /// Directories that were bound to this spot and are no longer
    /// bound to anything.
    pub unbound: Vec<PathBuf>,
}

/// Rows for `tonk spot list` plus the resolved active selection
/// (None when nothing resolves — empty registry or dangling name).
#[derive(Debug, Clone)]
pub struct Listing {
    /// `(name, site)` per registered spot, in name order.
    pub rows: Vec<(String, PathBuf)>,
    /// `(directory, spot)` per binding, in path order.
    pub bindings: Vec<(PathBuf, String)>,
    /// The spot a bare command would hit right now, with source.
    pub active: Option<Resolved>,
}

/// Register an already-mounted canonical site without binding any directory.
///
/// The registry is loaded immediately before the atomic save so a concurrent
/// name claim is never silently overwritten.
pub fn register_existing_unbound(
    store: &SpotStore,
    name: &str,
    site: &Path,
) -> Result<(), SpotError> {
    validate_name(name)?;
    let site = site.canonicalize().map_err(|error| {
        SpotError::Io(format!(
            "could not canonicalize {}: {error}",
            site.display()
        ))
    })?;
    let canonical = store.canonical_site(name).canonicalize().map_err(|error| {
        SpotError::Io(format!(
            "account spot is not mounted at canonical site {}: {error}",
            store.canonical_site(name).display()
        ))
    })?;
    if site != canonical {
        return Err(SpotError::Io(format!(
            "account spot must be mounted at canonical site {}",
            canonical.display()
        )));
    }

    let mut registry = store.load()?;
    if registry.spots.contains_key(name) {
        return Err(SpotError::Exists(name.to_owned()));
    }
    registry.spots.insert(name.to_owned(), SpotEntry { site });
    store.save(&registry)
}

/// Create (or adopt) a spot: initialize the site, register the name,
/// and optionally bind a directory to it. The site lands in the
/// store's canonical `spots/<name>/` unless `site_override` names
/// another directory; because [`crate::site::TonkSite::init_at_with`]
/// is idempotent, an override pointing at existing site storage adopts it.
pub async fn create(
    store: &SpotStore,
    name: &str,
    site_override: Option<&Path>,
    binding_directory: Option<&Path>,
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
    ensure_repository_name(&site, name).await?;

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
    if let Some(directory) = binding_directory {
        registry
            .bindings
            .insert(canonical(directory), name.to_owned());
    }
    store.save(&registry)?;
    Ok(outcome)
}

/// Give a newly-created CLI repository the same self-describing content fact
/// that browser-created repositories carry. Adopted sites keep an existing
/// content name; only a missing identity is filled from the registry name.
async fn ensure_repository_name(site: &crate::site::TonkSite, name: &str) -> Result<(), SpotError> {
    let session = site
        .branch()
        .await
        .map_err(|error| SpotError::Init(format!("acquire content branch: {error}")))?;
    let subject = site.repository.did().this();
    let existing: Vec<RepositoryName> = session
        .handle()
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(subject.clone()),
            name: Term::var("name"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|error| SpotError::Init(format!("read repository identity: {error}")))?;
    if existing.is_empty() {
        session
            .handle()
            .transaction()
            .assert(RepositoryName {
                this: subject,
                name: repo::Name(name.to_owned()),
            })
            .commit()
            .perform(&site.operator)
            .await
            .map_err(|error| SpotError::Init(format!("stamp repository identity: {error}")))?;
    }
    Ok(())
}

/// Outcome of [`bind`].
#[derive(Debug, Clone)]
pub struct BindOutcome {
    /// The canonicalized directory now bound.
    pub directory: PathBuf,
    /// The spot it resolves to.
    pub name: String,
    /// The spot it was bound to before, when it was already bound.
    pub previous: Option<String>,
}

/// Outcome of [`unbind`].
#[derive(Debug, Clone)]
pub struct UnbindOutcome {
    /// The directory that is no longer bound.
    pub directory: PathBuf,
    /// The spot it used to resolve to.
    pub name: String,
}

/// Bind `directory` to `name`.
/// Rebinding an already-bound directory overwrites and reports
/// what it replaced: unlike `spot new`, nothing is destroyed, so
/// there is no reason to demand an unbind first.
pub fn bind(store: &SpotStore, name: &str, directory: &Path) -> Result<BindOutcome, SpotError> {
    let mut registry = store.load()?;
    if !registry.spots.contains_key(name) {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
            binding: None,
        });
    }
    let directory = canonical(directory);
    let previous = registry.bindings.insert(directory.clone(), name.to_owned());
    store.save(&registry)?;
    Ok(BindOutcome {
        directory,
        name: name.to_owned(),
        previous,
    })
}

/// Unbind `directory`. Exact match only — see
/// [`SpotError::NotBound`].
pub fn unbind(store: &SpotStore, directory: &Path) -> Result<UnbindOutcome, SpotError> {
    let mut registry = store.load()?;
    let key = canonical(directory);
    let Some(name) = registry.bindings.remove(&key) else {
        return Err(SpotError::NotBound {
            directory: key.clone(),
            // The exact lookup just missed, so any hit here is a
            // strict ancestor.
            ancestor: directory_binding(&registry, &key),
        });
    };
    store.save(&registry)?;
    Ok(UnbindOutcome {
        directory: key,
        name,
    })
}

/// Everything `tonk spot list` needs in one read: the rows plus
/// what a bare command would currently resolve to (honouring the
/// same `flag`/`env` precedence, so `tonk --spot x spot list`
/// marks `x`).
pub fn listing(
    store: &SpotStore,
    flag: Option<&str>,
    env: Option<&str>,
    cwd: Option<&Path>,
) -> Result<Listing, SpotError> {
    let registry = store.load()?;
    let rows = registry
        .spots
        .iter()
        .map(|(name, entry)| (name.clone(), entry.site.clone()))
        .collect();
    let bindings = registry
        .bindings
        .iter()
        .map(|(directory, name)| (directory.clone(), name.clone()))
        .collect();
    Ok(Listing {
        rows,
        bindings,
        active: store.resolve(flag, env, cwd).ok(),
    })
}

/// Remove `name` from the registry. Site data stays on disk unless
/// `delete` — the registry is the authority on names, not a lifecycle
/// manager for storage it didn't necessarily create.
pub fn remove(store: &SpotStore, name: &str, delete: bool) -> Result<RemoveOutcome, SpotError> {
    let mut registry = store.load()?;
    let Some(entry) = registry.spots.remove(name) else {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
            binding: None,
        });
    };
    // A binding naming an unregistered spot would resolve to a
    // bare "unknown spot" on the next command, so drop them with the
    // entry.
    let unbound: Vec<PathBuf> = registry
        .bindings
        .iter()
        .filter(|(_, spot)| spot.as_str() == name)
        .map(|(directory, _)| directory.clone())
        .collect();
    for directory in &unbound {
        registry.bindings.remove(directory);
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
        unbound,
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
            legacy_current: current.map(str::to_owned),
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
            bindings: BTreeMap::new(),
            extra: serde_json::Map::new(),
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
        fn it_keeps_account_storage_outside_spots_and_the_registry() {
            let (tmp, store) = store();
            assert_eq!(store.account_dir(), tmp.path().join("account"));
            assert_ne!(store.account_dir(), store.canonical_site("account"));
            assert!(store.load().unwrap().spots.is_empty());
        }

        #[test]
        fn it_round_trips_the_registry() {
            let (_tmp, store) = store();
            let registry = registry_with(&[("garden", "/tmp/garden")], None);
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

        #[dialog_common::test]
        fn it_preserves_a_field_it_does_not_recognise_across_a_save() {
            let (_tmp, store) = store();
            std::fs::create_dir_all(store.registry_path().parent().unwrap()).unwrap();
            std::fs::write(
                store.registry_path(),
                r#"{
                    "current": "garden",
                    "spots": { "garden": { "site": "/tmp/garden" } },
                    "futureField": { "some": "value" }
                }"#,
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(registry.legacy_current.as_deref(), Some("garden"));
            assert_eq!(
                registry.spots.get("garden").map(|e| &e.site),
                Some(&PathBuf::from("/tmp/garden"))
            );

            // Unknown fields still round-trip, while the obsolete
            // global selection is deliberately dropped.
            store.save(&registry).expect("save");

            let reloaded: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(store.registry_path()).unwrap())
                    .expect("parse");
            assert_eq!(reloaded["futureField"]["some"], "value");
            assert!(reloaded.get("current").is_none());
            assert_eq!(reloaded["spots"]["garden"]["site"], "/tmp/garden");
        }

        #[dialog_common::test]
        fn it_migrates_attachments_and_drops_the_global_selection() {
            let (_tmp, store) = store();
            std::fs::create_dir_all(store.registry_path().parent().unwrap()).unwrap();
            std::fs::write(
                store.registry_path(),
                r#"{
                    "current": "garden",
                    "attachments": { "/project": "garden" },
                    "spots": { "garden": { "site": "/tmp/garden" } }
                }"#,
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(
                registry.bindings.get(Path::new("/project")),
                Some(&"garden".to_owned())
            );
            store.save(&registry).expect("save");

            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(store.registry_path()).unwrap())
                    .expect("parse");
            assert!(value.get("current").is_none(), "{value}");
            assert!(value.get("attachments").is_none(), "{value}");
            assert_eq!(value["bindings"]["/project"], "garden");
        }

        #[dialog_common::test]
        fn it_serializes_without_an_extra_or_bindings_key_when_neither_is_used() {
            let (_tmp, store) = store();
            let registry = registry_with(&[("garden", "/tmp/garden")], None);
            store.save(&registry).expect("save");

            let text = std::fs::read_to_string(store.registry_path()).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).expect("parse");
            let object = value.as_object().expect("object");
            assert!(!object.contains_key("bindings"), "{text}");
            for key in object.keys() {
                assert!(
                    matches!(key.as_str(), "spots"),
                    "unexpected key {key}: {text}"
                );
            }
        }
    }

    mod resolving {
        use super::*;

        #[test]
        fn it_prefers_flag_over_env() {
            let (_tmp, store) = store();
            let registry = registry_with(&[("a", "/s/a"), ("b", "/s/b"), ("c", "/s/c")], Some("c"));
            store.save(&registry).expect("save");

            let flag = store.resolve(Some("a"), Some("b"), None).expect("flag");
            assert_eq!(flag.name, "a");
            assert_eq!(flag.source, Source::Flag);

            let env = store.resolve(None, Some("b"), None).expect("env");
            assert_eq!(env.name, "b");
            assert_eq!(env.source, Source::Env);

            let err = store.resolve(None, None, None).expect_err("no binding");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
        }

        #[test]
        fn it_errors_when_nothing_is_selected() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            let err = store.resolve(None, None, None).expect_err("no selection");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
        }

        #[test]
        fn it_hints_spot_new_when_the_registry_is_empty() {
            let (_tmp, store) = store();
            let err = store.resolve(None, None, None).expect_err("empty");
            assert!(matches!(err, SpotError::NothingRegistered), "{err}");
            assert!(err.to_string().contains("tonk spot new"), "{err}");
        }

        #[test]
        fn it_errors_on_an_unknown_name_listing_available() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            let err = store
                .resolve(Some("nope"), None, None)
                .expect_err("unknown");
            assert!(err.to_string().contains("registered: a"), "{err}");
        }

        /// A registry with two spots and `/proj` bound to `a`.
        fn bound_registry() -> Registry {
            let mut registry = registry_with(&[("a", "/s/a"), ("b", "/s/b")], None);
            registry
                .bindings
                .insert(PathBuf::from("/proj"), "a".to_owned());
            registry
        }

        #[dialog_common::test]
        fn it_resolves_a_directory_binding() {
            let (_tmp, store) = store();
            store.save(&bound_registry()).expect("save");

            let resolved = store
                .resolve(None, None, Some(Path::new("/proj/sub/deep")))
                .expect("bound");
            assert_eq!(resolved.name, "a");
            assert_eq!(resolved.source, Source::Directory(PathBuf::from("/proj")));
        }

        #[dialog_common::test]
        fn it_takes_the_deepest_binding() {
            let (_tmp, store) = store();
            let mut registry = bound_registry();
            registry
                .bindings
                .insert(PathBuf::from("/proj/sub"), "b".to_owned());
            store.save(&registry).expect("save");

            let resolved = store
                .resolve(None, None, Some(Path::new("/proj/sub/deep")))
                .expect("bound");
            assert_eq!(resolved.name, "b");
            assert_eq!(
                resolved.source,
                Source::Directory(PathBuf::from("/proj/sub"))
            );
        }

        #[dialog_common::test]
        fn it_errors_outside_any_binding() {
            let (_tmp, store) = store();
            store.save(&bound_registry()).expect("save");

            let err = store
                .resolve(None, None, Some(Path::new("/elsewhere")))
                .expect_err("no binding");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
        }

        #[dialog_common::test]
        fn it_prefers_the_environment_over_a_binding() {
            let (_tmp, store) = store();
            store.save(&bound_registry()).expect("save");

            let resolved = store
                .resolve(None, Some("b"), Some(Path::new("/proj")))
                .expect("env");
            assert_eq!(resolved.name, "b");
            assert_eq!(resolved.source, Source::Env);
        }

        #[dialog_common::test]
        fn it_prefers_the_flag_over_a_binding() {
            let (_tmp, store) = store();
            store.save(&bound_registry()).expect("save");

            let resolved = store
                .resolve(Some("b"), None, Some(Path::new("/proj")))
                .expect("flag");
            assert_eq!(resolved.name, "b");
            assert_eq!(resolved.source, Source::Flag);
        }

        #[dialog_common::test]
        fn it_blames_the_binding_for_an_orphaned_name() {
            let (_tmp, store) = store();
            // `a` is bound at `/proj` but was never registered —
            // the hand-edit-the-file scenario `spot rm` normally
            // prevents by pruning bindings alongside the entry.
            let mut registry = registry_with(&[("b", "/s/b")], Some("b"));
            registry
                .bindings
                .insert(PathBuf::from("/proj"), "a".to_owned());
            store.save(&registry).expect("save");

            let err = store
                .resolve(None, None, Some(Path::new("/proj")))
                .expect_err("orphaned binding");
            let SpotError::Unknown { binding, .. } = &err else {
                panic!("{err}");
            };
            assert_eq!(binding.as_deref(), Some(Path::new("/proj")));
            assert!(err.to_string().contains("/proj"), "{err}");
            assert!(err.to_string().contains("spot unbind"), "{err}");
        }

        #[dialog_common::test]
        fn it_leaves_the_unknown_hint_unchanged_for_the_flag_case() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");

            let err = store
                .resolve(Some("nope"), None, None)
                .expect_err("unknown flag selection");
            let SpotError::Unknown { binding, .. } = &err else {
                panic!("{err}");
            };
            assert_eq!(*binding, None);
            assert!(!err.to_string().contains("spot unbind"), "{err}");
        }
    }

    mod binding {
        use super::*;

        #[dialog_common::test]
        fn it_binds_a_directory_and_reports_the_previous_binding() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")], None))
                .expect("save");

            let first = bind(&store, "a", Path::new("/proj")).expect("bind");
            assert_eq!(first.name, "a");
            assert_eq!(first.previous, None);
            assert_eq!(first.directory, PathBuf::from("/proj"));

            let second = bind(&store, "b", Path::new("/proj")).expect("rebind");
            assert_eq!(second.previous.as_deref(), Some("a"));
            assert_eq!(
                store.load().expect("load").bindings.get(Path::new("/proj")),
                Some(&"b".to_owned())
            );
        }

        #[dialog_common::test]
        fn it_refuses_to_bind_an_unknown_spot() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");

            let err = bind(&store, "nope", Path::new("/proj")).expect_err("unknown");
            assert!(matches!(err, SpotError::Unknown { .. }), "{err}");
            assert!(
                store.load().expect("load").bindings.is_empty(),
                "a failed bind must not write"
            );
        }

        #[dialog_common::test]
        fn it_unbinds_only_an_exact_match_and_names_the_ancestor() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            bind(&store, "a", Path::new("/proj")).expect("bind");

            let err = unbind(&store, Path::new("/proj/sub")).expect_err("not bound here");
            assert!(err.to_string().contains("/proj is bound to a"), "{err}");
            assert!(
                !store.load().expect("load").bindings.is_empty(),
                "a subdirectory unbind must not unbind the parent"
            );

            let outcome = unbind(&store, Path::new("/proj")).expect("unbind");
            assert_eq!(outcome.name, "a");
            assert!(store.load().expect("load").bindings.is_empty());
        }

        #[dialog_common::test]
        fn it_prunes_bindings_when_the_spot_is_removed() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")], None))
                .expect("save");
            bind(&store, "a", Path::new("/proj")).expect("bind a");
            bind(&store, "b", Path::new("/other")).expect("bind b");

            let outcome = remove(&store, "a", false).expect("remove");
            assert_eq!(outcome.unbound, vec![PathBuf::from("/proj")]);

            let bindings = store.load().expect("load").bindings;
            assert_eq!(bindings.get(Path::new("/other")), Some(&"b".to_owned()));
            assert!(!bindings.contains_key(Path::new("/proj")));
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
