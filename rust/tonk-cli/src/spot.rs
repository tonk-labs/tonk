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
//!   spots.json      registry: name → site path
//!   spots/<name>/   canonical site dirs
//! ```
//!
//! # One selector
//!
//! A command's spot comes from exactly one place: the *reference*
//! it was given. A reference is a registry name (`garden`) or a
//! path to a site directory (`~/proj/.tonk`) — [`classify`] tells
//! them apart by the name slug, so a bare word is always a name and
//! anything else is always a path. Named references are looked up in
//! the global registry; path references are used as-is.
//!
//! [`SpotStore::resolve`] reads exactly one thing: [`SPOT_ENV`].
//! Not a flag, not a machine-wide default, and not the working
//! directory — a `.tonk` you happened to `cd` next to is not a
//! decision you made. With the variable unset, every command is an
//! error naming the ways forward, never a guess.
//!
//! So a spot is acquired in exactly two ways, both of them things
//! you typed:
//!
//! - `tonk spot enter <ref>` exports [`SPOT_ENV`] into a subshell,
//!   so a terminal keeps its spot (and shows it in the prompt — see
//!   [`crate::shell`]). Two terminals hold different spots with
//!   nothing shared between them.
//! - `TONK_SPOT=<ref> tonk <cmd>` sets it for one process. This is
//!   what agents and CI use, since they can't live in a subshell.
//!
//! Both set the same variable, so there is one mechanism and no
//! second spelling that could disagree with the first.
//!
//! `tonk spot enter` with no argument is the one place a `.tonk`
//! directory is consulted — running it is itself the explicit act,
//! and what it exports is the resolved absolute path.
//!
//! `spots.json` stores absolute, expanded paths so applications
//! built on tonk can resolve a name with zero path logic. Writes
//! go through a temp file + atomic rename. A corrupt registry is a
//! hard error naming the file — never silently recreated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment variable holding the spot reference to use — the
/// only override there is. `tonk spot enter` exports it into a
/// subshell; agents and CI set it per-command. Same variable, same
/// meaning, no separate automation path and no flag form.
pub const SPOT_ENV: &str = "TONK_SPOT";

/// Environment variable recording which spot a `tonk spot enter`
/// shell was opened on, exported beside [`SPOT_ENV`].
///
/// It exists to answer a question [`SPOT_ENV`] alone cannot: an
/// inherited variable and one set for a single command arrive at
/// the child process byte-identical, so nothing can tell
/// `TONK_SPOT=x tonk …` from `export TONK_SPOT=x`. Comparing the
/// two variables can: equal means this invocation is riding the
/// shell's choice rather than making one. See
/// [`inherited_from_session`].
pub const SESSION_ENV: &str = "TONK_SPOT_SESSION";

/// Environment variable overriding the directory that holds
/// `spots.json` and the canonical `spots/` root, so tests can
/// isolate state (same pattern as `TONK_TELEMETRY_STATE`).
pub const STATE_ENV: &str = "TONK_SPOTS_STATE";

/// Whether this invocation took its spot from the surrounding
/// `tonk spot enter` shell instead of naming one.
///
/// True only when both variables are set and agree. A different
/// [`SPOT_ENV`] means the caller overrode the session deliberately,
/// which is the explicit act we want. An absent [`SESSION_ENV`]
/// means we are not in an entered shell and cannot tell — someone
/// exporting `TONK_SPOT` from their own shell profile looks exactly
/// like someone setting it for this command, and no amount of
/// inspection separates them.
pub fn inherited_from_session(spot: Option<&str>, session: Option<&str>) -> bool {
    matches!((spot, session), (Some(spot), Some(session)) if spot == session)
}

/// File name of the registry inside the store directory.
const REGISTRY_FILE: &str = "spots.json";

/// Directory name (inside the store) holding canonical site dirs.
const SPOTS_DIRNAME: &str = "spots";

/// On-disk registry: one entry per spot, and nothing else.
/// `BTreeMap` keeps listing and serialization order stable.
///
/// Deliberately holds no "current" selection. A machine-wide
/// default is ambient state every terminal shares and none of them
/// display — the thing that made it possible to run a command
/// against a spot you had forgotten another window selected.
/// Sessions carry their own reference in [`SPOT_ENV`] instead.
/// A `current` key left by an older tonk parses and is dropped on
/// the next write; serde ignores unknown fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
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

/// Where a resolution's reference came from. Printed by every
/// command before it does anything, so a session can always tell
/// what it is about to touch and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Named directly as an argument to `tonk spot enter`. Never a
    /// resolution tier — only how `enter` labels the reference it
    /// was handed.
    Argument,
    /// [`SPOT_ENV`] environment variable, as `tonk spot enter`
    /// exports it and agents set it per-command. The only way a
    /// command ever resolves a spot.
    Env,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Argument => "argument",
            Source::Env => "env",
        })
    }
}

/// What a spot reference names. A bare name-slug word is a registry
/// name; anything else — `./site`, `~/proj/.tonk`, an absolute path
/// — is a path. The split is total and needs no guessing: see
/// [`validate_name`] for the slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference<'a> {
    /// A registered spot name, looked up in the global registry.
    Name(&'a str),
    /// A site directory, used as given.
    Path(&'a str),
}

/// Classify a reference string. Never fails: whatever isn't a legal
/// registry name is treated as a path, and a path that doesn't
/// exist produces [`SpotError::NoSiteAt`] at resolution time rather
/// than a confusing "unknown spot".
pub fn classify(reference: &str) -> Reference<'_> {
    if validate_name(reference).is_ok() {
        Reference::Name(reference)
    } else {
        Reference::Path(reference)
    }
}

/// Expand a leading `~` against the home directory. Shells do this
/// before tonk sees an argument, but [`SPOT_ENV`] can carry a
/// tilde through a config file or a quoted assignment.
fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

/// A successful resolution: which spot, where its site lives, and
/// which selection mechanism picked it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The spot's registry name, or — for a path reference and for
    /// the local-directory tier, which have no registry entry — the
    /// site path.
    pub name: String,
    /// Absolute path to the site directory.
    pub site: PathBuf,
    /// Which selection mechanism named it.
    pub source: Source,
}

impl Resolved {
    /// The one-line "which spot am I on" every command prints:
    /// name, source, and site. Collapsed to a single mention when
    /// the name *is* the site path, so a path reference doesn't
    /// echo the same string twice.
    pub fn describe(&self) -> String {
        let site = self.site.display().to_string();
        if self.name == site {
            format!("{site} (via {source})", source = self.source)
        } else {
            format!(
                "{name} (via {source}) {site}",
                name = self.name,
                source = self.source,
            )
        }
    }
}

/// Failure modes for registry access and resolution.
#[derive(Debug, Error)]
pub enum SpotError {
    /// Spots exist but this session never named one. Never falls
    /// back to a default — the whole point is that a command can
    /// only touch a spot you pointed it at.
    #[error(
        "no spot selected; TONK_SPOT is not set. Open a shell on one with \
         `tonk spot enter <name>` (`tonk spot list` shows what is registered), \
         or name it for a single command: TONK_SPOT=<name> tonk <cmd>."
    )]
    NoSelection,
    /// The registry has zero spots — selection is moot; the fix is
    /// creating one.
    #[error(
        "no spots registered; create one with `tonk spot new <name>` \
         (add --site <path> to adopt an existing .tonk directory)"
    )]
    NothingRegistered,
    /// A path reference pointing at something that isn't a
    /// directory. Distinct from [`SpotError::Unknown`] so a typo'd
    /// path never reads as a typo'd name.
    #[error("no site directory at {}", .0.display())]
    NoSiteAt(PathBuf),
    /// `tonk spot enter` with no argument in a directory that has no
    /// `.tonk`. Names the directory it actually looked in — the
    /// walk-up that would have hidden this is deliberately absent.
    #[error(
        "no .tonk directory in {}; pass a name (`tonk spot enter <name>`) or \
         create one here with `tonk spot new <name> --site .tonk`",
        .0.display()
    )]
    NoLocalSite(PathBuf),
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

    /// Resolve one reference — a name against the registry, a path
    /// as given — tagging the result with where the reference came
    /// from.
    pub fn resolve_reference(
        &self,
        reference: &str,
        source: Source,
    ) -> Result<Resolved, SpotError> {
        match classify(reference) {
            Reference::Name(name) => {
                let registry = self.load()?;
                match registry.spots.get(name) {
                    Some(entry) => Ok(Resolved {
                        name: name.to_owned(),
                        site: entry.site.clone(),
                        source,
                    }),
                    None => Err(SpotError::Unknown {
                        name: name.to_owned(),
                        available: registry.spots.keys().cloned().collect(),
                    }),
                }
            }
            Reference::Path(path) => {
                let site = expand_home(path);
                if !site.is_dir() {
                    // Anchor the report: a relative reference that
                    // doesn't exist is only diagnosable if the
                    // error says which directory it was relative
                    // to. `absolute` is pure path arithmetic, so it
                    // works on a path that isn't there.
                    return Err(SpotError::NoSiteAt(
                        std::path::absolute(&site).unwrap_or(site),
                    ));
                }
                // Canonicalize so the echoed line and any later
                // comparison name one path, not `./x` and `/a/b/x`.
                let site = site.canonicalize().unwrap_or(site);
                Ok(Resolved {
                    name: site.display().to_string(),
                    site,
                    source,
                })
            }
        }
    }

    /// The site directory a bare `tonk spot enter` would open: `.tonk`
    /// in `cwd` itself. Never a parent — a spot you can't see with
    /// `ls` is exactly the surprise this design refuses.
    pub fn local_site(cwd: &Path) -> Option<PathBuf> {
        let candidate = cwd.join(crate::site::SITE_DIRNAME);
        candidate.is_dir().then_some(candidate)
    }

    /// Resolve the spot a command should operate on.
    ///
    /// One place: `env` ([`SPOT_ENV`], already read and
    /// empty-filtered by the caller). No flag form, no
    /// machine-wide default, and — deliberately — no consulting the
    /// working directory.
    ///
    /// A `.tonk` beside you is *not* a selection. It would mean a
    /// command silently working in a directory you happened to `cd`
    /// into, which is the one failure mode this whole design exists
    /// to remove; `tonk spot enter` can start from it because
    /// typing that is an explicit act, but running `tonk query`
    /// somewhere is not. So: the variable is set, or the command
    /// errors.
    pub fn resolve(&self, env: Option<&str>) -> Result<Resolved, SpotError> {
        match env {
            Some(reference) => self.resolve_reference(reference, Source::Env),
            None if self.load()?.spots.is_empty() => Err(SpotError::NothingRegistered),
            None => Err(SpotError::NoSelection),
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

/// Rows for `tonk spot list` plus whatever this session currently
/// resolves to (None when nothing does — no reference anywhere, or
/// one that dangles).
#[derive(Debug, Clone)]
pub struct Listing {
    /// `(name, site)` per registered spot, in name order.
    pub rows: Vec<(String, PathBuf)>,
    /// The spot a command would hit right now, with its source.
    /// Session-scoped, not a property of the registry.
    pub active: Option<Resolved>,
}

/// Create (or adopt) a spot: initialize the site and register the
/// name. The site lands in the store's canonical `spots/<name>/`
/// unless `site_override` names another directory; because
/// [`crate::site::TonkSite::init_at_with`] is idempotent, an
/// override pointing at existing site storage adopts it — the
/// migration path for pre-registry `.tonk/` dirs.
///
/// Creating does not select: the caller's shell is unchanged, and
/// `tonk spot enter <name>` is the separate, visible act that puts a
/// session on it.
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
    store.save(&registry)?;
    Ok(outcome)
}

/// Everything `tonk spot list` needs in one read: the rows plus
/// what this session currently resolves to, through the same
/// env/cwd order every other command uses.
pub fn listing(store: &SpotStore, env: Option<&str>) -> Result<Listing, SpotError> {
    let registry = store.load()?;
    let rows = registry
        .spots
        .iter()
        .map(|(name, entry)| (name.clone(), entry.site.clone()))
        .collect();
    Ok(Listing {
        rows,
        active: store.resolve(env).ok(),
    })
}

/// Remove `name` from the registry. Site data stays on disk unless
/// `delete` — the registry is the authority on names, not a
/// lifecycle manager for storage it didn't necessarily create.
pub fn remove(store: &SpotStore, name: &str, delete: bool) -> Result<RemoveOutcome, SpotError> {
    let mut registry = store.load()?;
    let Some(entry) = registry.spots.remove(name) else {
        return Err(SpotError::Unknown {
            name: name.to_owned(),
            available: registry.spots.keys().cloned().collect(),
        });
    };
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

    fn registry_with(names: &[(&str, &str)]) -> Registry {
        Registry {
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
            let registry = registry_with(&[("garden", "/tmp/garden")]);
            store.save(&registry).expect("save");
            assert_eq!(store.load().expect("load"), registry);
        }

        /// A registry written by a tonk that still had a global
        /// selection must keep loading — the entries are what
        /// matter, and `current` is simply dropped on the next
        /// write rather than failing the parse.
        #[test]
        fn it_loads_a_legacy_registry_that_still_carries_current() {
            let (_tmp, store) = store();
            std::fs::create_dir_all(store.registry_path().parent().unwrap()).unwrap();
            std::fs::write(
                store.registry_path(),
                r#"{"current":"garden","spots":{"garden":{"site":"/tmp/garden"}}}"#,
            )
            .unwrap();
            let registry = store.load().expect("legacy registry loads");
            assert_eq!(registry, registry_with(&[("garden", "/tmp/garden")]));
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
        fn it_prefers_the_environment_over_the_local_directory() {
            let (_tmp, store) = store();
            store
                .save(&registry_with(&[("a", "/s/a"), ("b", "/s/b")]))
                .expect("save");
            let env = store.resolve(Some("b")).expect("env");
            assert_eq!((env.name.as_str(), env.source), ("b", Source::Env));
        }

        /// The working directory is not a selector. A `.tonk` right
        /// next to you — the strongest case there is for consulting
        /// the cwd — must still resolve to nothing, or a command can
        /// act on a directory you merely walked into.
        #[test]
        fn it_never_resolves_a_local_dot_tonk() {
            let (tmp, store) = store();
            store.save(&registry_with(&[("a", "/s/a")])).expect("save");
            std::fs::create_dir_all(tmp.path().join(crate::site::SITE_DIRNAME))
                .expect("mkdir .tonk beside us");

            let err = store.resolve(None).expect_err("cwd is not a selector");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
        }

        #[test]
        fn it_errors_when_nothing_names_a_spot() {
            let (_tmp, store) = store();
            store.save(&registry_with(&[("a", "/s/a")])).expect("save");
            let err = store.resolve(None).expect_err("no selection");
            assert!(matches!(err, SpotError::NoSelection), "{err}");
            assert!(err.to_string().contains("tonk spot enter"), "{err}");
            assert!(err.to_string().contains("TONK_SPOT"), "{err}");
        }

        #[test]
        fn it_hints_spot_new_when_the_registry_is_empty() {
            let (_tmp, store) = store();
            let err = store.resolve(None).expect_err("empty");
            assert!(matches!(err, SpotError::NothingRegistered), "{err}");
            assert!(err.to_string().contains("tonk spot new"), "{err}");
        }

        #[test]
        fn it_errors_on_an_unknown_name_listing_available() {
            let (_tmp, store) = store();
            store.save(&registry_with(&[("a", "/s/a")])).expect("save");
            let err = store.resolve(Some("nope")).expect_err("unknown");
            assert!(err.to_string().contains("registered: a"), "{err}");
        }

        /// A path reference resolves to itself without touching the
        /// registry, so an unregistered site directory is usable by
        /// path alone.
        #[test]
        fn it_resolves_a_path_reference_without_the_registry() {
            let (tmp, store) = store();
            let site = tmp.path().join("proj").join(crate::site::SITE_DIRNAME);
            std::fs::create_dir_all(&site).expect("mkdir site");

            let resolved = store
                .resolve(Some(site.to_str().expect("utf-8")))
                .expect("path reference");
            assert_eq!(resolved.source, Source::Env);
            assert_eq!(resolved.site, site.canonicalize().expect("canonicalize"));
        }

        /// A mistyped path must not read as a mistyped name — the
        /// two failures have completely different fixes.
        #[test]
        fn it_distinguishes_a_missing_path_from_an_unknown_name() {
            let (tmp, store) = store();
            store.save(&registry_with(&[("a", "/s/a")])).expect("save");
            let missing = tmp.path().join("nope").join(".tonk");

            let err = store
                .resolve(Some(missing.to_str().expect("utf-8")))
                .expect_err("missing path");
            assert!(matches!(err, SpotError::NoSiteAt(_)), "{err}");
            assert!(!err.to_string().contains("unknown spot"), "{err}");
        }
    }

    mod spotting_an_inherited_session {
        use super::*;

        #[test]
        fn it_reports_inheritance_when_the_spot_is_the_sessions_own() {
            assert!(inherited_from_session(Some("garden"), Some("garden")));
        }

        /// A different value is the caller overriding the shell on
        /// purpose — the explicit act the whole design is asking
        /// for. Warning about it would punish the right behaviour.
        #[test]
        fn it_stays_quiet_when_the_caller_overrode_the_session() {
            assert!(!inherited_from_session(Some("beta"), Some("garden")));
        }

        /// Outside an entered shell there is no session to compare
        /// against, and an exported `TONK_SPOT` from someone's shell
        /// profile is indistinguishable from one set for this
        /// command. Undetectable, so unclaimed.
        #[test]
        fn it_claims_nothing_without_a_session_to_compare() {
            assert!(!inherited_from_session(Some("garden"), None));
            assert!(!inherited_from_session(None, Some("garden")));
            assert!(!inherited_from_session(None, None));
        }
    }

    mod classifying_references {
        use super::*;

        #[test]
        fn it_reads_bare_slug_words_as_registry_names() {
            for reference in ["garden", "work-2", "a_b", "0day"] {
                assert_eq!(classify(reference), Reference::Name(reference));
            }
        }

        #[test]
        fn it_reads_everything_path_shaped_as_a_path() {
            for reference in ["./site", "~/proj/.tonk", "/abs/site", ".tonk", "../up"] {
                assert_eq!(classify(reference), Reference::Path(reference));
            }
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
