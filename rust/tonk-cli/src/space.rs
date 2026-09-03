//! Space registry: named spaces, canonical storage, selection.
//!
//! A *space* is a named entry in `spaces.json` mapping to the *site*
//! directory that backs it (see [`crate::site`]). The registry and
//! the canonical site directories live under the platform data dir
//! (`~/Library/Application Support/tonk/` on macOS), next to
//! `telemetry.json` / `update.json`:
//!
//! ```text
//! tonk/
//!   spaces.json      registry: name → site path plus directory
//!                    bindings
//!   spaces/<name>/   canonical site dirs
//! ```
//!
//! Selection resolves `--space` > `TONK_SPACE` > a directory
//! binding (the nearest bound ancestor of the cwd). The flag and env
//! forms are per-invocation / per-process; bindings persist that
//! choice for sessions that live in a directory. A directory is only
//! ever a key into the registry — it never locates or contains space
//! data.
//!
//! `spaces.json` stores absolute, expanded paths so applications
//! built on tonk can resolve a name with zero path logic. Writes
//! go through a temp file + atomic rename. A corrupt registry is a
//! hard error naming the file — never silently recreated.
//!
//! A pre-rename registry and site root remain readable. The first command to
//! touch that layout converts the store in place (see [`SpaceStore::load`]).
//! Nothing writes the retired spelling back, so conversion happens once. An
//! older `tonk` sharing the same data directory then stops seeing these spaces,
//! but does not destroy them because its unknown-field sink round-trips what it
//! cannot read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_schema::{RepositoryName, prelude::DidExt as _};

/// Canonical environment variable naming the space to use.
pub const SPACE_ENV: &str = "TONK_SPACE";

/// Environment variable overriding the directory that holds
/// `spaces.json` and the canonical `spaces/` root, so tests can
/// isolate state (same pattern as `TONK_TELEMETRY_STATE`).
pub const STATE_ENV: &str = "TONK_SPACES_STATE";

/// File name of the registry inside the store directory.
const REGISTRY_FILE: &str = "spaces.json";

/// Pre-rename registry filename, read once and migrated away. See
/// [`SpaceStore::load`].
const LEGACY_REGISTRY_FILE: &str = "spots.json";

/// Directory name (inside the store) holding canonical site dirs.
const SPACES_DIRNAME: &str = "spaces";

/// Pre-rename canonical-site directory name, moved once on migration.
const LEGACY_SPACES_DIRNAME: &str = "spots";

/// On-disk registry: one entry per space plus directory bindings.
/// `BTreeMap` keeps listing and serialization order stable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    /// Compatibility sink for registries written before directory
    /// bindings replaced the machine-global selection. It is never
    /// consulted or written back.
    #[serde(default, rename = "current", skip_serializing)]
    legacy_current: Option<String>,
    /// Name → entry. Paths inside are absolute and expanded.
    ///
    /// The alias reads the pre-rename registry key. It must be an alias rather
    /// than a bare rename because [`Registry::extra`] would otherwise swallow
    /// the legacy field into the unknown-field sink and report a populated
    /// installation as empty.
    #[serde(default, alias = "spots")]
    pub spaces: BTreeMap<String, SpaceEntry>,
    /// Directories bound to a space, keyed by canonicalized absolute
    /// path. A top-level map rather than a list inside each entry: a
    /// key cannot repeat, so "one directory, one space" is structural
    /// rather than enforced.
    #[serde(
        default,
        alias = "attachments",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub bindings: BTreeMap<PathBuf, String>,
    /// The account this installation is currently signed into, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountRecord>,
    /// Fields this binary does not recognise. `spaces.json` is a public
    /// format other applications read and rewrite directly, and this
    /// binary is not necessarily the newest one touching it — an
    /// older `tonk` (stable channel, pre-`tonk update`) or a
    /// third-party writer must not silently drop a field it has never
    /// heard of just because it round-tripped the registry.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// One registered space.
///
/// A binding and nothing else. Which account owns a space is read from the
/// space's own roster, not recorded beside it: a tag here could drift from
/// the delegation chains the access service actually validates, with nothing
/// checking it, and a member device could never learn the owner of a space it
/// merely joined.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpaceEntry {
    /// Absolute path to the site directory backing this space.
    pub site: PathBuf,
}

impl SpaceEntry {
    /// An entry for `site`.
    pub fn at(site: impl Into<PathBuf>) -> Self {
        Self { site: site.into() }
    }
}

/// The one account this installation is signed into.
///
/// Tonk is signed into at most one account at a time: linking replaces this
/// record, logout clears it. Signing out touches nothing else — not replicas,
/// not the profile, not retained delegations — so every registered space
/// stays open, and the account only parameterizes account-service operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    /// Immutable account-root DID.
    pub root: String,
    /// Origin that hosted the most recent successful ceremony.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceremony_origin: Option<String>,
    /// Provider-matched default content endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_remote: Option<String>,
    /// Unknown forward-compatible fields.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl AccountRecord {
    /// A record for a freshly linked account root.
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            ceremony_origin: None,
            access_remote: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// Where a resolution's space name came from. Surfaced in status
/// and error output so a session can always tell what it is about
/// to touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `--space` flag.
    Flag,
    /// [`SPACE_ENV`] environment variable.
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

/// A successful resolution: which space, where its site lives, and
/// which selection mechanism picked it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The space's registry name.
    pub name: String,
    /// Absolute path to the site directory.
    pub site: PathBuf,
    /// Which selection mechanism named it.
    pub source: Source,
}

/// Failure modes for registry access and resolution.
#[derive(Debug, Error)]
pub enum SpaceError {
    /// Spaces exist but neither the process nor cwd selects one.
    #[error(
        "no space active for this directory; run `tonk space use <name>`, \
         pass --space, or set TONK_SPACE"
    )]
    NoSelection,
    /// The registry has zero spaces — selection is moot; the fix is
    /// creating one.
    #[error(
        "no spaces registered; create one with `tonk space new <name>` \
         (add --site <path> to adopt an existing .tonk directory)"
    )]
    NothingRegistered,
    /// A name that isn't in the registry.
    #[error("unknown space '{name}'{}", unknown_hint(.available, .binding))]
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
    /// `space unbind` against a directory with no binding of its
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
    /// `space new` against a name that already exists. Re-pointing
    /// is an explicit `rm` + `new`, never an overwrite.
    #[error(
        "space '{0}' already exists; to re-point the name run \
         `tonk space rm {0} --keep-data` first, or `tonk space rm {0}` \
         to delete its data as well"
    )]
    Exists(String),
    /// The registry file exists but doesn't parse. Deliberately
    /// not self-healing: silently recreating it would orphan every
    /// registered space.
    #[error("corrupt space registry at {path}: {detail}")]
    Corrupt {
        /// Path to the offending `spaces.json`.
        path: PathBuf,
        /// The serde error text.
        detail: String,
    },
    /// A name outside the allowed slug. Canonical names become
    /// directory names, so the alphabet is conservative.
    #[error("invalid space name '{0}': use [a-z0-9][a-z0-9-_]*")]
    InvalidName(String),
    /// The platform reports no data directory (no home).
    #[error("could not determine the platform data directory")]
    NoDataDir,
    /// Site bootstrap (`space new`) failed inside the site layer.
    #[error("failed to initialize site: {0}")]
    Init(String),
    /// `space transplant` failed inside the site layer.
    #[error("transplant failed: {0}")]
    Transplant(String),
    /// Registry or site-directory I/O.
    #[error("{0}")]
    Io(String),
    /// Both the pre-rename and current canonical roots contain data.
    #[error(
        "cannot convert the legacy space store because both {legacy} and {current} exist; move or merge one root explicitly"
    )]
    ConflictingRoots {
        /// Pre-rename canonical-site root.
        legacy: PathBuf,
        /// Current canonical-site root.
        current: PathBuf,
    },
}

/// Hint suffix for [`SpaceError::Unknown`]: list what is registered,
/// or point at `space new` when nothing is; when the name came from a
/// directory binding, name the directory too and point at `space
/// unbind` — otherwise the error reads as coming from nowhere, and
/// there is no obvious way to clear it.
fn unknown_hint(available: &[String], binding: &Option<PathBuf>) -> String {
    let registered = if available.is_empty() {
        "; none registered (create one with `tonk space new <name>`)".to_string()
    } else {
        format!("; registered: {}", available.join(", "))
    };
    match binding {
        Some(directory) => format!(
            "{registered}; via binding at {directory} — clear it with `tonk space unbind {directory}`",
            directory = directory.display(),
        ),
        None => registered,
    }
}

/// Hint suffix for [`SpaceError::NotBound`]: name the bound ancestor,
/// so the fix is a `cd` away.
fn unbind_hint(ancestor: &Option<(PathBuf, String)>) -> String {
    match ancestor {
        Some((directory, name)) => {
            format!("; {} is bound to {name}", directory.display())
        }
        None => String::new(),
    }
}

/// Handle on the space state directory. All registry reads and
/// writes go through one of these; tests construct it with
/// [`SpaceStore::at`] over a tempdir so nothing touches the user's
/// real data dir and no test ever mutates process-global env.
#[derive(Debug, Clone)]
pub struct SpaceStore {
    dir: PathBuf,
}

impl SpaceStore {
    /// The real store: [`STATE_ENV`] override, else the platform
    /// data dir (`dirs::data_dir()/tonk`, the same base telemetry
    /// and update state use).
    pub fn open() -> Result<Self, SpaceError> {
        if let Ok(dir) = std::env::var(STATE_ENV)
            && !dir.is_empty()
        {
            return Ok(Self { dir: dir.into() });
        }
        let base = dirs::data_dir().ok_or(SpaceError::NoDataDir)?;
        Ok(Self {
            dir: base.join("tonk"),
        })
    }

    /// A store rooted at an explicit directory (tests).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Root directory containing this profile's space and account state.
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Path to `spaces.json` inside this store.
    pub fn registry_path(&self) -> PathBuf {
        self.dir.join(REGISTRY_FILE)
    }

    /// Dedicated account-system repository directory.
    ///
    /// It is a sibling of `spaces/`, never a registered space and never written
    /// to `spaces.json`.
    pub fn account_dir(&self) -> PathBuf {
        self.dir.join("account")
    }

    /// Canonical site directory for `name` inside this store.
    /// Purely a path computation — nothing is created.
    pub fn canonical_site(&self, name: &str) -> PathBuf {
        self.dir.join(SPACES_DIRNAME).join(name)
    }

    /// The root holding canonical site directories.
    pub fn spaces_root(&self) -> PathBuf {
        self.dir.join(SPACES_DIRNAME)
    }

    /// Canonical site directories that no registry entry names.
    ///
    /// These are left by `tonk space rm --keep-data` (and by a
    /// hand-edited `spaces.json`). They are invisible to every other
    /// command yet still occupy their names: `tonk join --name x` and
    /// `tonk account space pull --name x` both refuse to write over
    /// one. Listing them is what makes that state recoverable instead
    /// of merely confusing.
    ///
    /// A store whose `spaces/` directory cannot be read has no
    /// orphans to report — this is a diagnostic, never a reason to
    /// fail a command that was otherwise fine.
    pub fn orphaned_sites(&self, registry: &Registry) -> Vec<PathBuf> {
        let registered: Vec<PathBuf> = registry
            .spaces
            .values()
            .map(|entry| canonical(&entry.site))
            .collect();
        let Ok(entries) = std::fs::read_dir(self.spaces_root()) else {
            return Vec::new();
        };
        // Canonicalized on the way out, not just for the comparison:
        // these paths get printed as `--site` arguments, and a path
        // that doesn't compare equal to the one `space new` would
        // register is a path that re-creates this same confusion.
        let mut orphans: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| canonical(&entry.path()))
            .filter(|path| !registered.contains(path))
            .collect();
        orphans.sort();
        orphans
    }

    /// Path to the pre-rename registry, read once by [`Self::load`].
    fn legacy_registry_path(&self) -> PathBuf {
        self.dir.join(LEGACY_REGISTRY_FILE)
    }

    /// The pre-rename root of canonical site directories.
    fn legacy_spaces_root(&self) -> PathBuf {
        self.dir.join(LEGACY_SPACES_DIRNAME)
    }

    /// Load the registry. A missing file is an empty registry (the
    /// pre-first-use state); an unparseable one is
    /// [`SpaceError::Corrupt`].
    ///
    /// An installation still on the pre-rename layout is converted here
    /// first — see [`Self::adopt_legacy_layout`] — because this is the
    /// first place any command touches the registry, and a store that
    /// reported itself empty would read as "no spaces registered" to
    /// someone holding a full one.
    pub fn load(&self) -> Result<Registry, SpaceError> {
        let path = self.registry_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return self.adopt_legacy_layout();
            }
            Err(e) => {
                return Err(SpaceError::Io(format!(
                    "could not read {}: {e}",
                    path.display()
                )));
            }
        };
        serde_json::from_str(&text).map_err(|e| SpaceError::Corrupt {
            path,
            detail: e.to_string(),
        })
    }

    /// Convert a pre-rename registry and site root to the current layout,
    /// returning the registry either way.
    ///
    /// Only reached when `spaces.json` is absent, so a converted store
    /// never pays for this again. The order is chosen so every
    /// interruption converges on a re-run: the site directory moves
    /// first, the rewritten registry lands atomically second, and the
    /// legacy registry is removed last. A crash before the write leaves the
    /// old registry naming directories that have moved, which the next run
    /// fixes because the rewrite is a prefix substitution that does not
    /// consult the filesystem.
    ///
    /// Failure is an error rather than an empty registry: silently
    /// reporting zero spaces to someone who has ten is the one outcome
    /// worse than refusing to run.
    fn adopt_legacy_layout(&self) -> Result<Registry, SpaceError> {
        let legacy_path = self.legacy_registry_path();
        let text = match std::fs::read_to_string(&legacy_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let current = self.registry_path();
                return match std::fs::read_to_string(&current) {
                    Ok(text) => serde_json::from_str(&text).map_err(|e| SpaceError::Corrupt {
                        path: current,
                        detail: e.to_string(),
                    }),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
                    Err(e) => Err(SpaceError::Io(format!(
                        "could not read {}: {e}",
                        current.display()
                    ))),
                };
            }
            Err(e) => {
                return Err(SpaceError::Io(format!(
                    "could not read {}: {e}",
                    legacy_path.display()
                )));
            }
        };
        let mut registry: Registry =
            serde_json::from_str(&text).map_err(|e| SpaceError::Corrupt {
                path: legacy_path.clone(),
                detail: e.to_string(),
            })?;

        // Reading remains useful on a mounted or permissioned read-only
        // data directory. Leave the legacy paths intact and retry the
        // conversion on a later writable invocation.
        if std::fs::metadata(&self.dir)
            .map(|metadata| metadata.permissions().readonly())
            .unwrap_or(false)
        {
            return Ok(registry);
        }

        let (legacy_root, root) = (self.legacy_spaces_root(), self.spaces_root());
        if legacy_root.is_dir() && root.exists() {
            return Err(SpaceError::ConflictingRoots {
                legacy: legacy_root,
                current: root,
            });
        }
        if legacy_root.is_dir() && !root.exists() {
            std::fs::rename(&legacy_root, &root).map_err(|e| {
                SpaceError::Io(format!(
                    "could not move {} to {}: {e}",
                    legacy_root.display(),
                    root.display()
                ))
            })?;
        }
        for entry in registry.spaces.values_mut() {
            if let Ok(relative) = entry.site.strip_prefix(&legacy_root) {
                entry.site = root.join(relative);
            }
        }

        self.save(&registry)?;
        // Best-effort: the store is already correct without it, and a
        // read-only data dir is not a reason to fail every command. What
        // it costs is a stale file an older `tonk` would still write to.
        let _ = std::fs::remove_file(&legacy_path);
        Ok(registry)
    }

    /// Persist the registry atomically: write a sibling temp file,
    /// then rename over `spaces.json` so concurrent readers never
    /// observe a torn write.
    pub fn save(&self, registry: &Registry) -> Result<(), SpaceError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| SpaceError::Io(format!("could not create {}: {e}", self.dir.display())))?;
        let path = self.registry_path();
        let tmp = self.dir.join(format!("{REGISTRY_FILE}.tmp"));
        let text = serde_json::to_string_pretty(registry)
            .map_err(|e| SpaceError::Io(format!("could not serialize registry: {e}")))?;
        std::fs::write(&tmp, text)
            .map_err(|e| SpaceError::Io(format!("could not write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            SpaceError::Io(format!("could not move {} into place: {e}", tmp.display()))
        })
    }

    /// The account this installation is signed into, if any.
    pub fn account(&self) -> Result<Option<AccountRecord>, SpaceError> {
        Ok(self.load()?.account)
    }

    /// Record (or clear) the signed-in account.
    pub fn set_account(&self, account: Option<AccountRecord>) -> Result<(), SpaceError> {
        let mut registry = self.load()?;
        registry.account = account;
        self.save(&registry)
    }

    /// Resolve the space a command should operate on.
    ///
    /// Strict precedence: `flag` (`--space`) > `env` ([`SPACE_ENV`],
    /// already read and empty-filtered by the caller) > a directory
    /// binding at or above `cwd`.
    ///
    /// `cwd` is passed in rather than read here so nothing depends on
    /// process-global state, and it is only ever a key into the
    /// registry: the directory never locates site data.
    ///
    /// `SPACE_ENV` outranks bindings deliberately. A harness that
    /// pinned a space for the process must not be overridden by
    /// whatever directory it happened to launch in.
    pub fn resolve(
        &self,
        flag: Option<&str>,
        env: Option<&str>,
        cwd: Option<&Path>,
    ) -> Result<Resolved, SpaceError> {
        let registry = self.load()?;
        let (name, source) = if let Some(name) = flag {
            (name.to_owned(), Source::Flag)
        } else if let Some(name) = env {
            (name.to_owned(), Source::Env)
        } else if let Some((directory, name)) =
            cwd.and_then(|cwd| directory_binding(&registry, cwd))
        {
            (name, Source::Directory(directory))
        } else if registry.spaces.is_empty() {
            return Err(SpaceError::NothingRegistered);
        } else {
            return Err(SpaceError::NoSelection);
        };
        match registry.spaces.get(&name) {
            // Resolution never consults the signed-in account. Editing a
            // replica this device holds is unrestricted; the only enforcement
            // that is real happens at the service boundary, against the
            // space's own delegation chain.
            Some(entry) => Ok(Resolved {
                name,
                site: entry.site.clone(),
                source,
            }),
            None => {
                // Name the bound directory in the error too, when
                // that is where the name came from — otherwise an
                // orphaned binding (the registered space was
                // removed by hand or by `space rm` on another
                // machine) reads as an unexplained failure with no
                // way to clear it.
                let binding = match &source {
                    Source::Directory(directory) => Some(directory.clone()),
                    _ => None,
                };
                Err(SpaceError::Unknown {
                    name,
                    available: registry.spaces.keys().cloned().collect(),
                    binding,
                })
            }
        }
    }
}

/// Validate a space name against the canonical slug:
/// `[a-z0-9][a-z0-9-_]*`. Names become directory names under
/// `spaces/`, so the alphabet stays conservative.
pub fn validate_name(name: &str) -> Result<(), SpaceError> {
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let tail_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if head_ok && tail_ok {
        Ok(())
    } else {
        Err(SpaceError::InvalidName(name.to_owned()))
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

/// Outcome of [`create`]: the registered space and the DID of the
/// repository backing it.
#[derive(Debug, Clone)]
pub struct CreateOutcome {
    /// Registry name.
    pub name: String,
    /// Absolute site directory.
    pub site: PathBuf,
    /// The site repository's DID.
    pub did: String,
    /// Whether site data was already at the target and got adopted
    /// rather than created.
    ///
    /// Adoption is the point of `--site`, but it also happens
    /// silently at the canonical path after `tonk space rm
    /// --keep-data` — the same name picks its old data back up. That
    /// is usually what someone wants and never what they expect, so
    /// the caller says which one happened.
    pub adopted: bool,
}

/// What [`remove`] should do with the space's site data.
///
/// Explicit rather than a `bool` because the two arms are not
/// variations on one operation: deleting is irreversible and
/// keeping leaves data behind that no registry entry names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Data {
    /// Delete the site directory from disk.
    Delete,
    /// Leave the site directory where it is. The data then belongs
    /// to no registered space — see [`SpaceStore::orphaned_sites`].
    Keep,
}

/// What happened to the site directory during [`remove`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deletion {
    /// The directory was deleted.
    Deleted,
    /// [`Data::Delete`] was asked for but nothing was there — the
    /// registry named a directory that had already been removed by
    /// hand. Distinguished from `Deleted` so output can say the
    /// data was already gone rather than claim to have destroyed it.
    AlreadyGone,
    /// [`Data::Keep`]: the directory is still on disk.
    Kept,
}

/// Outcome of [`remove`].
#[derive(Debug, Clone)]
pub struct RemoveOutcome {
    /// The removed registry name.
    pub name: String,
    /// Where the site lived (still lives, when `data` is
    /// [`Deletion::Kept`]).
    pub site: PathBuf,
    /// What became of the site directory.
    pub data: Deletion,
    /// Directories that were bound to this space and are no longer
    /// bound to anything.
    pub unbound: Vec<PathBuf>,
}

/// Rows for `tonk space` plus the resolved active selection
/// (None when nothing resolves — empty registry or dangling name).
#[derive(Debug, Clone)]
pub struct Listing {
    /// `(name, site)` per registered space, in name order.
    pub rows: Vec<(String, PathBuf)>,
    /// `(directory, space)` per binding, in path order.
    pub bindings: Vec<(PathBuf, String)>,
    /// The space a bare command would hit right now, with source.
    pub active: Option<Resolved>,
    /// Site data under `spaces/` that no entry names. See
    /// [`SpaceStore::orphaned_sites`].
    pub orphans: Vec<PathBuf>,
}

/// Register an already-mounted canonical site without binding any directory.
///
/// The registry is loaded immediately before the atomic save so a concurrent
/// name claim is never silently overwritten.
pub fn register_existing_unbound(
    store: &SpaceStore,
    name: &str,
    site: &Path,
) -> Result<(), SpaceError> {
    validate_name(name)?;
    let site = site.canonicalize().map_err(|error| {
        SpaceError::Io(format!(
            "could not canonicalize {}: {error}",
            site.display()
        ))
    })?;
    let canonical = store.canonical_site(name).canonicalize().map_err(|error| {
        SpaceError::Io(format!(
            "account space is not mounted at canonical site {}: {error}",
            store.canonical_site(name).display()
        ))
    })?;
    if site != canonical {
        return Err(SpaceError::Io(format!(
            "account space must be mounted at canonical site {}",
            canonical.display()
        )));
    }

    let mut registry = store.load()?;
    if registry.spaces.contains_key(name) {
        return Err(SpaceError::Exists(name.to_owned()));
    }
    registry
        .spaces
        .insert(name.to_owned(), SpaceEntry::at(site));
    store.save(&registry)
}

/// Create (or adopt) a space: initialize the site, register the name,
/// and optionally bind a directory to it. The site lands in the
/// store's canonical `spaces/<name>/` unless `site_override` names
/// another directory; because [`crate::site::TonkSite::init_at_with`]
/// is idempotent, an override pointing at existing site storage adopts it.
pub async fn create(
    store: &SpaceStore,
    name: &str,
    site_override: Option<&Path>,
    binding_directory: Option<&Path>,
    config: crate::site::SiteConfig,
) -> Result<CreateOutcome, SpaceError> {
    validate_name(name)?;
    let mut registry = store.load()?;
    if registry.spaces.contains_key(name) {
        return Err(SpaceError::Exists(name.to_owned()));
    }

    let target = site_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.canonical_site(name));
    // Asked before init, which is idempotent and so cannot tell us
    // afterwards whether it created or adopted.
    let adopted = crate::site::has_site_data(&target);
    let site = crate::site::TonkSite::init_at_with(&target, config)
        .await
        .map_err(|e| SpaceError::Init(format!("{e:#}")))?;

    // A fresh CLI space must carry the same self-identifying row the worker's
    // create path writes. Invite validation uses this fact to distinguish a
    // usable repository from an arbitrary branch that happens to have data.
    // Adoption preserves the existing repository's name instead of treating
    // the local registry alias as a rename request.
    if !adopted {
        site.branch()
            .await
            .map_err(|e| SpaceError::Init(format!("failed to open main branch: {e}")))?
            .handle()
            .transaction()
            .assert(RepositoryName {
                this: site.repository.did().this(),
                name: tonk_schema::domain::repo::Name(name.to_owned()),
            })
            .commit()
            .perform(&site.operator)
            .await
            .map_err(|e| SpaceError::Init(format!("failed to stamp repository identity: {e}")))?;
    }

    let outcome = CreateOutcome {
        name: name.to_owned(),
        site: site.root.clone(),
        did: site.repository.did().to_string(),
        adopted,
    };
    registry
        .spaces
        .insert(name.to_owned(), SpaceEntry::at(outcome.site.clone()));
    if let Some(directory) = binding_directory {
        registry
            .bindings
            .insert(canonical(directory), name.to_owned());
    }
    store.save(&registry)?;
    Ok(outcome)
}

/// Outcome of [`transplant`].
#[derive(Debug, Clone)]
pub struct TransplantOutcome {
    /// The registered name, unchanged by the transplant.
    pub name: String,
    /// Absolute path of the (re-rooted) site directory.
    pub site: PathBuf,
    /// The subject the history was adopted from.
    pub origin: String,
    /// The fresh subject DID.
    pub did: String,
    /// Where the pre-transplant copy of the site was kept, unless the
    /// transplant ran in place.
    pub backup: Option<PathBuf>,
}

/// Re-root a registered space under a freshly minted subject, keeping
/// its data and history in place — the recovery when the space's keys
/// are lost, or the retirement of a subject on purpose.
///
/// The registry entry is untouched: the name still points at the same
/// site directory, whose identity file is what changed. By default the
/// whole site is first copied aside to `<site>.pre-transplant` so the
/// origin-keyed store survives as a fallback; `in_place` skips the
/// copy.
pub async fn transplant(
    store: &SpaceStore,
    name: &str,
    in_place: bool,
    config: crate::site::SiteConfig,
) -> Result<TransplantOutcome, SpaceError> {
    let resolved = store.resolve(Some(name), None, None)?;
    let backup = if in_place {
        None
    } else {
        let backup = backup_path(&resolved.site);
        if backup.exists() {
            return Err(SpaceError::Transplant(format!(
                "a pre-transplant copy already exists at {}; move it aside or pass --in-place",
                backup.display()
            )));
        }
        crate::migrate::copy_dir_recursive(&resolved.site, &backup)
            .map_err(|e| SpaceError::Transplant(format!("failed to copy the site aside: {e:#}")))?;
        Some(backup)
    };
    let transplanted = crate::site::transplant_at_with(&resolved.site, name, config)
        .await
        .map_err(|e| SpaceError::Transplant(format!("{e:#}")))?;
    Ok(TransplantOutcome {
        name: name.to_owned(),
        site: resolved.site,
        did: transplanted.site.repository.did().to_string(),
        origin: transplanted.origin,
        backup,
    })
}

/// The sibling directory a transplant copies the origin-keyed site to.
fn backup_path(site: &Path) -> PathBuf {
    let mut file_name = site
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    file_name.push(".pre-transplant");
    site.with_file_name(file_name)
}

/// Outcome of [`bind`].
#[derive(Debug, Clone)]
pub struct BindOutcome {
    /// The canonicalized directory now bound.
    pub directory: PathBuf,
    /// The space it resolves to.
    pub name: String,
    /// The space it was bound to before, when it was already bound.
    pub previous: Option<String>,
}

/// Outcome of [`unbind`].
#[derive(Debug, Clone)]
pub struct UnbindOutcome {
    /// The directory that is no longer bound.
    pub directory: PathBuf,
    /// The space it used to resolve to.
    pub name: String,
}

/// Bind `directory` to `name`.
/// Rebinding an already-bound directory overwrites and reports
/// what it replaced: unlike `space new`, nothing is destroyed, so
/// there is no reason to demand an unbind first.
pub fn bind(store: &SpaceStore, name: &str, directory: &Path) -> Result<BindOutcome, SpaceError> {
    let mut registry = store.load()?;
    if !registry.spaces.contains_key(name) {
        return Err(SpaceError::Unknown {
            name: name.to_owned(),
            available: registry.spaces.keys().cloned().collect(),
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
/// [`SpaceError::NotBound`].
pub fn unbind(store: &SpaceStore, directory: &Path) -> Result<UnbindOutcome, SpaceError> {
    let mut registry = store.load()?;
    let key = canonical(directory);
    let Some(name) = registry.bindings.remove(&key) else {
        return Err(SpaceError::NotBound {
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

/// Everything `tonk space` needs in one read: the rows plus
/// what a bare command would currently resolve to (honouring the
/// same `flag`/`env` precedence, so `tonk --space x space`
/// marks `x`).
pub fn listing(
    store: &SpaceStore,
    flag: Option<&str>,
    env: Option<&str>,
    cwd: Option<&Path>,
) -> Result<Listing, SpaceError> {
    let registry = store.load()?;
    let rows = registry
        .spaces
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
        orphans: store.orphaned_sites(&registry),
    })
}

/// Remove `name` from the registry, deleting its site data or
/// leaving it behind per `data`.
///
/// The data is deleted *before* the registry is saved. The reverse
/// order looks more cautious and is worse: a delete that fails after
/// the entry is already gone leaves data on disk that nothing names,
/// which is precisely the state that later blocks `tonk join` and
/// `tonk account space pull` on the same name. Deleting first means a
/// failure leaves the space fully registered and the command safe to
/// retry.
///
/// The residual risk runs the other way — data deleted, registry save
/// fails — and is benign: the entry points at an empty path, `tonk
/// space` shows it, and re-running `rm` clears it.
pub fn remove(store: &SpaceStore, name: &str, data: Data) -> Result<RemoveOutcome, SpaceError> {
    let mut registry = store.load()?;
    let Some(entry) = registry.spaces.remove(name) else {
        return Err(SpaceError::Unknown {
            name: name.to_owned(),
            available: registry.spaces.keys().cloned().collect(),
            binding: None,
        });
    };
    // A binding naming an unregistered space would resolve to a
    // bare "unknown space" on the next command, so drop them with the
    // entry.
    let unbound: Vec<PathBuf> = registry
        .bindings
        .iter()
        .filter(|(_, space)| space.as_str() == name)
        .map(|(directory, _)| directory.clone())
        .collect();
    for directory in &unbound {
        registry.bindings.remove(directory);
    }

    let deletion = match data {
        Data::Keep => Deletion::Kept,
        Data::Delete => match std::fs::remove_dir_all(&entry.site) {
            Ok(()) => Deletion::Deleted,
            // The registry outlived the directory. Nothing to
            // destroy and nothing to warn about beyond saying so:
            // dropping the entry is still the right outcome.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Deletion::AlreadyGone,
            Err(e) => {
                return Err(SpaceError::Io(format!(
                    "could not delete {}: {e}; space '{name}' is still registered",
                    entry.site.display()
                )));
            }
        },
    };
    store.save(&registry)?;

    Ok(RemoveOutcome {
        name: name.to_owned(),
        site: entry.site,
        data: deletion,
        unbound,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SpaceStore) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SpaceStore::at(tmp.path());
        (tmp, store)
    }

    fn registry_with(names: &[(&str, &str)], current: Option<&str>) -> Registry {
        Registry {
            legacy_current: current.map(str::to_owned),
            spaces: names
                .iter()
                .map(|(name, site)| ((*name).to_owned(), SpaceEntry::at(PathBuf::from(site))))
                .collect(),
            bindings: BTreeMap::new(),
            account: None,
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
        fn it_keeps_account_storage_outside_spaces_and_the_registry() {
            let (tmp, store) = store();
            assert_eq!(store.account_dir(), tmp.path().join("account"));
            assert_ne!(store.account_dir(), store.canonical_site("account"));
            assert!(store.load().unwrap().spaces.is_empty());
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
            assert!(matches!(err, SpaceError::Corrupt { .. }), "{err}");
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
                    "spaces": { "garden": { "site": "/tmp/garden" } },
                    "futureField": { "some": "value" }
                }"#,
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(registry.legacy_current.as_deref(), Some("garden"));
            assert_eq!(
                registry.spaces.get("garden").map(|e| &e.site),
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
            assert_eq!(reloaded["spaces"]["garden"]["site"], "/tmp/garden");
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
                    "spaces": { "garden": { "site": "/tmp/garden" } }
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
                    matches!(key.as_str(), "spaces"),
                    "unexpected key {key}: {text}"
                );
            }
        }
    }

    mod adopting_the_pre_rename_layout {
        use super::*;

        /// Build the registry and site layout a pre-rename `tonk` left behind.
        fn legacy_store(names: &[&str]) -> (tempfile::TempDir, SpaceStore) {
            let (tmp, store) = store();
            let root = tmp.path().join("spots");
            let mut spots = serde_json::Map::new();
            for name in names {
                std::fs::create_dir_all(root.join(name)).unwrap();
                std::fs::write(root.join(name).join("marker"), name).unwrap();
                spots.insert(
                    (*name).to_owned(),
                    serde_json::json!({ "site": root.join(name) }),
                );
            }
            std::fs::write(
                tmp.path().join("spots.json"),
                serde_json::to_string_pretty(&serde_json::json!({ "spots": spots })).unwrap(),
            )
            .unwrap();
            (tmp, store)
        }

        #[dialog_common::test]
        fn it_reads_spaces_from_the_legacy_registry_key() {
            let (_tmp, store) = legacy_store(&["garden", "work"]);
            let registry = store.load().expect("load");
            assert_eq!(
                registry.spaces.keys().collect::<Vec<_>>(),
                vec!["garden", "work"]
            );
        }

        #[dialog_common::test]
        fn it_moves_canonical_site_data_under_the_new_root() {
            let (tmp, store) = legacy_store(&["garden"]);
            store.load().expect("load");

            assert!(!tmp.path().join("spots").exists());
            assert_eq!(
                std::fs::read_to_string(store.canonical_site("garden").join("marker")).unwrap(),
                "garden"
            );
        }

        #[dialog_common::test]
        fn it_rewrites_entry_paths_that_pointed_into_the_old_root() {
            let (_tmp, store) = legacy_store(&["garden"]);
            let registry = store.load().expect("load");
            assert_eq!(
                registry.spaces.get("garden").map(|entry| &entry.site),
                Some(&store.canonical_site("garden"))
            );
        }

        /// A `--site` elsewhere was never under the store's own root, so
        /// the prefix rewrite must leave it exactly where it is.
        #[dialog_common::test]
        fn it_leaves_a_site_outside_the_old_root_alone() {
            let (tmp, store) = store();
            let elsewhere = tmp.path().join("project").join(".tonk");
            std::fs::create_dir_all(&elsewhere).unwrap();
            std::fs::write(
                tmp.path().join("spots.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "spots": { "proj": { "site": elsewhere } }
                }))
                .unwrap(),
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(
                registry.spaces.get("proj").map(|entry| &entry.site),
                Some(&elsewhere)
            );
        }

        #[dialog_common::test]
        fn it_carries_the_account_slot_and_bindings_across_but_drops_entry_ownership() {
            let (tmp, store) = store();
            let bound = tmp.path().join("work");
            std::fs::create_dir_all(&bound).unwrap();
            std::fs::write(
                tmp.path().join("spots.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "spots": { "garden": { "site": "/tmp/garden", "account": "did:key:owner" } },
                    "bindings": { bound.to_str().unwrap(): "garden" },
                    "account": { "root": "did:key:root" }
                }))
                .unwrap(),
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(
                registry.account.as_ref().map(|a| a.root.as_str()),
                Some("did:key:root")
            );
            assert_eq!(
                registry.bindings.get(&bound).map(String::as_str),
                Some("garden")
            );
            let written: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(store.registry_path()).expect("read converted registry"),
            )
            .expect("parse converted registry");
            assert!(
                written["spaces"]["garden"].get("account").is_none(),
                "ownership comes from the roster, not a registry tag: {written}"
            );
        }

        #[dialog_common::test]
        fn it_writes_the_converted_registry_under_the_new_name_and_key() {
            let (tmp, store) = legacy_store(&["garden"]);
            store.load().expect("load");

            assert!(!tmp.path().join("spots.json").exists());
            let written: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(store.registry_path()).unwrap())
                    .expect("parse");
            assert!(written.get("spots").is_none(), "{written}");
            assert!(written["spaces"]["garden"].is_object(), "{written}");
        }

        /// The conversion is only ever reached with no `spaces.json`, so
        /// a second run must be a plain read that touches nothing.
        #[dialog_common::test]
        fn it_converges_when_run_twice() {
            let (tmp, store) = legacy_store(&["garden"]);
            let first = store.load().expect("first load");
            let second = store.load().expect("second load");
            assert_eq!(first, second);
            assert!(!tmp.path().join("spots").exists());
        }

        /// Interrupted between the directory move and the registry write: the
        /// old root is gone while the legacy registry still names it. The
        /// rewrite is a prefix substitution that never consults the filesystem,
        /// so the re-run still lands on the right paths.
        #[dialog_common::test]
        fn it_recovers_when_the_move_landed_but_the_registry_write_did_not() {
            let (tmp, store) = legacy_store(&["garden"]);
            std::fs::rename(tmp.path().join("spots"), tmp.path().join("spaces")).unwrap();

            let registry = store.load().expect("load");
            assert_eq!(
                registry.spaces.get("garden").map(|entry| &entry.site),
                Some(&store.canonical_site("garden"))
            );
        }

        #[dialog_common::test]
        fn it_refuses_to_guess_when_both_site_roots_exist() {
            let (tmp, store) = legacy_store(&["garden"]);
            std::fs::create_dir_all(tmp.path().join("spaces").join("other")).unwrap();

            let error = store.load().expect_err("conflicting roots");
            assert!(format!("{error}").contains("both"), "{error}");
            assert!(tmp.path().join("spots").join("garden").exists());
            assert!(tmp.path().join("spaces").join("other").exists());
            assert!(!tmp.path().join("spaces.json").exists());
        }

        #[dialog_common::test]
        fn it_refuses_to_report_an_unparseable_legacy_registry_as_empty() {
            let (tmp, store) = store();
            std::fs::write(tmp.path().join("spots.json"), "{ not json").unwrap();

            let error = store.load().expect_err("corrupt");
            assert!(
                matches!(&error, SpaceError::Corrupt { path, .. } if path.ends_with("spots.json")),
                "{error:?}"
            );
        }

        #[dialog_common::test]
        fn it_ignores_a_legacy_registry_once_the_store_has_been_converted() {
            let (tmp, store) = store();
            store
                .save(&registry_with(&[("garden", "/tmp/garden")], None))
                .expect("save");
            std::fs::write(
                tmp.path().join("spots.json"),
                r#"{ "spots": { "stale": { "site": "/tmp/stale" } } }"#,
            )
            .unwrap();

            let registry = store.load().expect("load");
            assert_eq!(registry.spaces.keys().collect::<Vec<_>>(), vec!["garden"]);
        }

        /// A second process can enter conversion after observing no
        /// `spaces.json`, then lose the race while the first process writes it
        /// and removes the legacy registry. It must read the winner's registry,
        /// not return an empty baseline that a subsequent write can save.
        #[dialog_common::test]
        fn it_rechecks_the_current_registry_when_the_legacy_file_disappears() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("garden", "/tmp/garden")], None))
                .expect("save winner");

            let registry = store.adopt_legacy_layout().expect("losing converter");
            assert_eq!(registry.spaces.keys().collect::<Vec<_>>(), vec!["garden"]);
        }

        #[cfg(unix)]
        #[dialog_common::test]
        fn it_reads_a_legacy_store_when_the_data_directory_is_read_only() {
            use std::os::unix::fs::PermissionsExt as _;

            let (tmp, store) = legacy_store(&["garden"]);
            let mut writable = std::fs::metadata(tmp.path()).unwrap().permissions();
            let mut read_only = writable.clone();
            read_only.set_mode(0o555);
            std::fs::set_permissions(tmp.path(), read_only).unwrap();

            let loaded = store.load();

            writable.set_mode(0o755);
            std::fs::set_permissions(tmp.path(), writable).unwrap();
            let registry = loaded.expect("read-only legacy store remains readable");
            assert_eq!(registry.spaces.keys().collect::<Vec<_>>(), vec!["garden"]);
            assert!(tmp.path().join("spots.json").exists());
            assert!(!tmp.path().join("spaces.json").exists());
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
            assert!(matches!(err, SpaceError::NoSelection), "{err}");
        }

        #[test]
        fn it_errors_when_nothing_is_selected() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");
            let err = store.resolve(None, None, None).expect_err("no selection");
            assert!(matches!(err, SpaceError::NoSelection), "{err}");
        }

        #[test]
        fn it_hints_space_new_when_the_registry_is_empty() {
            let (_tmp, store) = store();
            let err = store.resolve(None, None, None).expect_err("empty");
            assert!(matches!(err, SpaceError::NothingRegistered), "{err}");
            assert!(err.to_string().contains("tonk space new"), "{err}");
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

        /// A registry with two spaces and `/proj` bound to `a`.
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
            assert!(matches!(err, SpaceError::NoSelection), "{err}");
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
            // the hand-edit-the-file scenario `space rm` normally
            // prevents by pruning bindings alongside the entry.
            let mut registry = registry_with(&[("b", "/s/b")], Some("b"));
            registry
                .bindings
                .insert(PathBuf::from("/proj"), "a".to_owned());
            store.save(&registry).expect("save");

            let err = store
                .resolve(None, None, Some(Path::new("/proj")))
                .expect_err("orphaned binding");
            let SpaceError::Unknown { binding, .. } = &err else {
                panic!("{err}");
            };
            assert_eq!(binding.as_deref(), Some(Path::new("/proj")));
            assert!(err.to_string().contains("/proj"), "{err}");
            assert!(err.to_string().contains("space unbind"), "{err}");
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
            let SpaceError::Unknown { binding, .. } = &err else {
                panic!("{err}");
            };
            assert_eq!(*binding, None);
            assert!(!err.to_string().contains("space unbind"), "{err}");
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
        fn it_refuses_to_bind_an_unknown_space() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a")], None))
                .expect("save");

            let err = bind(&store, "nope", Path::new("/proj")).expect_err("unknown");
            assert!(matches!(err, SpaceError::Unknown { .. }), "{err}");
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
        fn it_prunes_bindings_when_the_space_is_removed() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")], None))
                .expect("save");
            bind(&store, "a", Path::new("/proj")).expect("bind a");
            bind(&store, "b", Path::new("/other")).expect("bind b");

            let outcome = remove(&store, "a", Data::Keep).expect("remove");
            assert_eq!(outcome.unbound, vec![PathBuf::from("/proj")]);

            let bindings = store.load().expect("load").bindings;
            assert_eq!(bindings.get(Path::new("/other")), Some(&"b".to_owned()));
            assert!(!bindings.contains_key(Path::new("/proj")));
        }
    }

    mod removal {
        use super::*;

        /// A failed delete must leave the space registered. The
        /// alternative — name unregistered, data still on disk — is
        /// the exact state this command exists to prevent, and
        /// reaching it by accident is worse than failing loudly.
        #[dialog_common::test]
        fn it_keeps_the_entry_registered_when_the_delete_fails() {
            let (tmp, store) = store();
            // A regular file where a site directory should be:
            // `remove_dir_all` refuses it, and not with NotFound.
            let site = tmp.path().join("not-a-directory");
            std::fs::write(&site, b"").expect("write");
            store
                .save(&registry_with(
                    &[("a", site.to_str().expect("utf-8 path"))],
                    None,
                ))
                .expect("save");

            let err = remove(&store, "a", Data::Delete).expect_err("delete fails");
            assert!(err.to_string().contains("still registered"), "{err}");
            assert!(
                store.load().expect("load").spaces.contains_key("a"),
                "the entry survives a failed delete"
            );
        }

        #[dialog_common::test]
        fn it_counts_only_unregistered_site_directories_as_orphans() {
            let (tmp, store) = store();
            let live = store.canonical_site("live");
            let kept = store.canonical_site("kept");
            std::fs::create_dir_all(&live).expect("live");
            std::fs::create_dir_all(&kept).expect("kept");
            let registry = registry_with(&[("live", live.to_str().expect("utf-8 path"))], None);
            store.save(&registry).expect("save");

            assert_eq!(
                store.orphaned_sites(&registry),
                vec![kept.canonicalize().expect("canonicalize kept")]
            );
            // A store whose `spaces/` root does not exist yet has
            // nothing to report rather than something to fail on.
            let empty = SpaceStore::at(tmp.path().join("nowhere"));
            assert!(empty.orphaned_sites(&registry).is_empty());
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
