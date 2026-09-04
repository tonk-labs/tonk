//! `tonk migrate carry` — copy a `.carry/` directory into `.tonk/`.
//!
//! Carry and tonk share the same on-disk layout (a single
//! dialog repository named `main`, two branches, identity in the
//! platform profile dir). The migration is therefore a
//! file-level copy plus a sanity-check open: locate the source,
//! refuse if the destination exists, copy or move, then open
//! the new `.tonk/` to verify the dialog repository loads. A
//! verification failure rolls back the partial destination so
//! the caller can retry without manual cleanup.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use dialog_repository::Repository;

use crate::site::{self, TonkSite};

/// Default name of the source directory carry writes to.
const CARRY_DIRNAME: &str = ".carry";

/// Outcome of a successful migration.
#[derive(Debug)]
pub struct MigrationOutcome {
    /// Absolute path to the source `.carry/`.
    pub source: PathBuf,
    /// Absolute path to the migrated `.tonk/`.
    pub destination: PathBuf,
    /// Repository DID surfaced by opening the migrated `.tonk/`.
    pub repo_did: String,
    /// `true` if `--move` was honoured (the source was removed).
    pub moved: bool,
}

/// Migration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Copy the source to the destination; leave the source in place.
    Copy,
    /// Atomically rename the source to the destination when they
    /// share a filesystem; otherwise copy then delete the source.
    Move,
}

/// [`run`] with a caller-supplied [`crate::site::SiteConfig`], so the
/// verification open resolves against the caller's profile and store
/// rather than the install defaults — what isolated tests need.
pub async fn run_with(
    start: &Path,
    source_override: Option<&Path>,
    mode: Mode,
    config: crate::site::SiteConfig,
) -> Result<MigrationOutcome> {
    run_inner(start, source_override, mode, config).await
}

/// Drive the migration: locate, refuse-on-conflict, copy/move,
/// verify by opening the new `.tonk/`, roll back on verification
/// failure.
pub async fn run(
    start: &Path,
    source_override: Option<&Path>,
    mode: Mode,
) -> Result<MigrationOutcome> {
    run_inner(start, source_override, mode, crate::site::default_config()?).await
}

async fn run_inner(
    start: &Path,
    source_override: Option<&Path>,
    mode: Mode,
    config: crate::site::SiteConfig,
) -> Result<MigrationOutcome> {
    let source = resolve_source(start, source_override)?;
    let destination = source
        .parent()
        .ok_or_else(|| anyhow!("source has no parent directory: {}", source.display()))?
        .join(site::SITE_DIRNAME);

    if destination.exists() {
        return Err(anyhow!(
            "refusing to overwrite existing destination: {} (remove or rename it first)",
            destination.display()
        ));
    }

    let moved = perform_transfer(&source, &destination, mode)?;

    // Verify by opening the migrated site. If the repo handle
    // loads, the migration is sound.
    match open_for_verify(&destination, config).await {
        Ok(repository) => Ok(MigrationOutcome {
            source,
            destination,
            repo_did: repository.did().to_string(),
            moved,
        }),
        Err(verify_err) => {
            // Roll back: remove the partial destination so the
            // user can retry without manually cleaning up.
            let _ = std::fs::remove_dir_all(&destination);
            Err(verify_err.context(format!(
                "verification failed; rolled back {}",
                destination.display()
            )))
        }
    }
}

/// Walk up from `start` looking for a `.carry/` directory. With
/// `source_override` supplied, takes that path verbatim (after
/// canonicalisation).
fn resolve_source(start: &Path, source_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = source_override {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", path.display()))?;
        if !canonical.is_dir() {
            return Err(anyhow!("{} is not a directory", canonical.display()));
        }
        return Ok(canonical);
    }
    let mut current: PathBuf = start
        .canonicalize()
        .with_context(|| format!("could not canonicalize {}", start.display()))?;
    loop {
        let candidate = current.join(CARRY_DIRNAME);
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !current.pop() {
            return Err(anyhow!(
                "no .carry/ found above {}; pass --from to point at one explicitly",
                start.display()
            ));
        }
    }
}

/// Move (`Mode::Move`) or copy (`Mode::Copy`) `source` to
/// `destination`. Returns `true` if the source no longer exists
/// after the call.
///
/// `Mode::Move` first tries `std::fs::rename` — atomic on the same
/// filesystem. If that fails (typically EXDEV cross-device), falls
/// back to copy-then-delete.
fn perform_transfer(source: &Path, destination: &Path, mode: Mode) -> Result<bool> {
    match mode {
        Mode::Move => {
            if std::fs::rename(source, destination).is_ok() {
                return Ok(true);
            }
            // Cross-device or other rename failure — fall through
            // to copy + delete. The copy step's errors surface
            // unwrapped so the caller sees the real reason rather
            // than the rename error we swallowed.
            copy_dir_recursive(source, destination).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
            std::fs::remove_dir_all(source).with_context(|| {
                format!("failed to remove source {} after copy", source.display())
            })?;
            Ok(true)
        }
        Mode::Copy => {
            copy_dir_recursive(source, destination).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
            Ok(false)
        }
    }
}

/// Recursively copy `source` to `destination`, creating
/// directories as needed and copying every regular file. Skips
/// symlinks rather than following them to avoid escaping the
/// source tree (carry doesn't write any, so this is defensive).
pub(crate) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&entry_path, &target)?;
        } else if file_type.is_file() {
            std::fs::copy(&entry_path, &target).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry_path.display(),
                    target.display()
                )
            })?;
        }
        // Symlinks intentionally skipped.
    }
    Ok(())
}

/// Open the migrated `.tonk/` and return the repository handle.
/// Used as the verification step — if this fails, the migration
/// is rolled back.
async fn open_for_verify(
    destination: &Path,
    config: crate::site::SiteConfig,
) -> Result<Repository> {
    let site = TonkSite::open_with(destination, config).await?;
    Ok(site.repository)
}
