//! `tonk migrate carry` — copy a `.carry/` directory into `.tonk/`.
//!
//! Carry and tonk share the same on-disk layout (a single
//! dialog repository named `main`, two branches, identity in the
//! platform profile dir). The migration is therefore a
//! file-level copy plus a sanity-check open: locate the source,
//! refuse if the destination exists, copy into a hidden sibling,
//! then open that stage to verify the dialog repository loads. The
//! verified directory is published with one rename. `--move`
//! removes its source only after publication.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::site::{self, TonkSite};
use crate::staged_directory::StagedDirectory;

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
    /// Copy, verify, and publish the destination before deleting the source.
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

/// Drive the migration: locate, refuse-on-conflict, copy into a sibling,
/// verify there, publish the new `.tonk/`, and only then clean up a move source.
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
    let source_identity = if mode == Mode::Move {
        Some(SourceDirectoryIdentity::capture(&source).with_context(|| {
            format!(
                "cannot safely move source {} because its directory identity could not be recorded; the source remains unchanged",
                source.display()
            )
        })?)
    } else {
        None
    };
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

    let stage = StagedDirectory::beside(&destination, "migrate-carry")?;
    copy_dir_recursive(&source, stage.path()).with_context(|| {
        format!(
            "failed to copy migration source {} into stage {} for destination {}; the source remains unchanged",
            source.display(),
            stage.path().display(),
            destination.display()
        )
    })?;

    let repo_did = open_for_verify(stage.path(), config).await.map_err(|error| {
        error.context(format!(
            "verification failed before publication; source remains at {} and canonical destination {} was not created",
            source.display(),
            destination.display()
        ))
    })?;
    let destination = stage.publish().with_context(|| {
        format!(
            "verified migration publication at {destination} did not settle cleanly; source remains at {source}; if the destination is absent, retry the migration; if it is present, never overwrite or delete it merely to retry—verify its repository subject before deciding whether it is the migrated copy or another operation's destination",
            destination = destination.display(),
            source = source.display()
        )
    })?;

    let moved = mode == Mode::Move;
    if moved {
        remove_source_after_publication(
            &StdFilesystem,
            &source,
            &destination,
            source_identity
                .as_ref()
                .expect("move mode captured a source identity"),
        )?;
    }

    Ok(MigrationOutcome {
        source,
        destination,
        repo_did,
        moved,
    })
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

trait Filesystem {
    fn directory_identity(&self, path: &Path) -> std::io::Result<SourceDirectoryIdentity>;

    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()>;
}

struct StdFilesystem;

impl Filesystem for StdFilesystem {
    fn directory_identity(&self, path: &Path) -> std::io::Result<SourceDirectoryIdentity> {
        SourceDirectoryIdentity::capture(path)
    }

    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_dir_all(path)
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl SourceDirectoryIdentity {
    fn capture(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt as _;

        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} is not a real directory", path.display()),
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

#[cfg(not(unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceDirectoryIdentity;

#[cfg(not(unix))]
impl SourceDirectoryIdentity {
    fn capture(path: &Path) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "directory identity is unavailable for {} on this platform",
                path.display()
            ),
        ))
    }
}

fn remove_source_after_publication(
    filesystem: &impl Filesystem,
    source: &Path,
    destination: &Path,
    expected_identity: &SourceDirectoryIdentity,
) -> Result<()> {
    let current_identity = filesystem.directory_identity(source).with_context(|| {
        format!(
            "verified destination is published at {}, but the move source {} could not be re-identified; it was not deleted",
            destination.display(),
            source.display()
        )
    })?;
    if current_identity != *expected_identity {
        anyhow::bail!(
            "verified destination is published at {}, but move source {} changed since migration began; the current source path was not deleted",
            destination.display(),
            source.display()
        );
    }
    filesystem.remove_dir_all(source).with_context(|| {
        format!(
            "verified destination is published at {}, but failed to remove move source {}; both copies remain, so inspect these paths and retry only the source cleanup",
            destination.display(),
            source.display()
        )
    })
}

/// Recursively copy `source` to `destination`, creating
/// directories as needed and copying every regular file. Skips
/// symlinks rather than following them to avoid escaping the
/// source tree (carry doesn't write any, so this is defensive).
fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
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

/// Open the staged `.tonk/`, capture its subject, then drop every repository
/// handle before the caller renames the directory.
async fn open_for_verify(stage: &Path, config: crate::site::SiteConfig) -> Result<String> {
    let site = TonkSite::open_with(stage, config).await?;
    let repo_did = site.repository.did().to_string();
    site.reactor.shutdown();
    drop(site);
    Ok(repo_did)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::io::{Error, ErrorKind};

    use super::*;

    #[cfg(unix)]
    struct RefusingFilesystem;

    #[cfg(unix)]
    impl Filesystem for RefusingFilesystem {
        fn directory_identity(&self, path: &Path) -> std::io::Result<SourceDirectoryIdentity> {
            SourceDirectoryIdentity::capture(path)
        }

        fn remove_dir_all(&self, _path: &Path) -> std::io::Result<()> {
            Err(Error::new(ErrorKind::PermissionDenied, "injected refusal"))
        }
    }

    #[cfg(unix)]
    struct MismatchingFilesystem {
        identity: SourceDirectoryIdentity,
    }

    #[cfg(unix)]
    impl Filesystem for MismatchingFilesystem {
        fn directory_identity(&self, _path: &Path) -> std::io::Result<SourceDirectoryIdentity> {
            Ok(self.identity)
        }

        fn remove_dir_all(&self, _path: &Path) -> std::io::Result<()> {
            panic!("an identity mismatch must fail before deletion")
        }
    }

    #[cfg(unix)]
    #[test]
    fn it_keeps_the_verified_destination_when_source_cleanup_fails() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join(".carry");
        let destination = temp.path().join(".tonk");
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&destination)?;
        std::fs::write(source.join("source"), b"source copy")?;
        std::fs::write(destination.join("verified"), b"verified copy")?;
        let identity = SourceDirectoryIdentity::capture(&source)?;

        let error =
            remove_source_after_publication(&RefusingFilesystem, &source, &destination, &identity)
                .expect_err("source cleanup is injected to fail");

        assert_eq!(std::fs::read(source.join("source"))?, b"source copy");
        assert_eq!(
            std::fs::read(destination.join("verified"))?,
            b"verified copy"
        );
        let rendered = format!("{error:#}");
        assert!(rendered.contains("both copies remain"), "{rendered}");
        assert!(
            rendered.contains(&source.display().to_string()),
            "{rendered}"
        );
        assert!(
            rendered.contains(&destination.display().to_string()),
            "{rendered}"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn it_refuses_to_delete_a_move_source_whose_identity_changed() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join(".carry");
        let destination = temp.path().join(".tonk");
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&destination)?;
        std::fs::write(source.join("replacement"), b"replacement source")?;
        std::fs::write(destination.join("verified"), b"verified copy")?;
        let expected = SourceDirectoryIdentity::capture(&source)?;
        let mismatched = SourceDirectoryIdentity {
            inode: expected.inode.wrapping_add(1),
            ..expected
        };

        let error = remove_source_after_publication(
            &MismatchingFilesystem {
                identity: mismatched,
            },
            &source,
            &destination,
            &expected,
        )
        .expect_err("a replaced source path must not be deleted");

        assert_eq!(
            std::fs::read(source.join("replacement"))?,
            b"replacement source"
        );
        assert_eq!(
            std::fs::read(destination.join("verified"))?,
            b"verified copy"
        );
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("changed since migration began"),
            "{rendered}"
        );
        assert!(
            rendered.contains(&source.display().to_string()),
            "{rendered}"
        );
        assert!(
            rendered.contains(&destination.display().to_string()),
            "{rendered}"
        );
        Ok(())
    }
}
