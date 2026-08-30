//! Crash-safe construction of a directory beside its final path.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use anyhow::{Context, Result, anyhow, bail};

const STAGE_PREFIX: &str = ".tonk-stage-";
const MARKER_MAGIC: &[u8] = b"tonk-staged-directory-v1\0";

/// A uniquely named sibling directory that is removed unless it is published.
pub(crate) struct StagedDirectory {
    destination: PathBuf,
    stage: PathBuf,
    marker: PathBuf,
    marker_contents: Vec<u8>,
    identity: DirectoryIdentity,
    published: bool,
}

impl StagedDirectory {
    /// Create a hidden staging directory on the destination filesystem.
    pub(crate) fn beside(destination: &Path, label: &str) -> Result<Self> {
        if label.is_empty()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            bail!(
                "invalid staged-directory label '{label}' for {}",
                destination.display()
            );
        }
        let parent = destination.parent().ok_or_else(|| {
            anyhow!(
                "cannot stage destination without a parent directory: {}",
                destination.display()
            )
        })?;
        if !parent.exists() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent {} for destination {}",
                    parent.display(),
                    destination.display()
                )
            })?;
        }
        let metadata = std::fs::symlink_metadata(parent).with_context(|| {
            format!(
                "failed to inspect parent {} for destination {}",
                parent.display(),
                destination.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "refusing to stage destination {} beside symlink or non-directory parent {}",
                destination.display(),
                parent.display()
            );
        }

        let prefix = format!("{STAGE_PREFIX}{label}-");
        let temporary = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in(parent)
            .with_context(|| {
                format!(
                    "failed to create a staged directory beside destination {}",
                    destination.display()
                )
            })?;
        let stage = temporary.path().to_path_buf();
        let marker = marker_path(&stage)?;
        let marker_contents = marker_contents(&stage, destination)?;
        let marker_result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)
                .with_context(|| format!("failed to create stage marker {}", marker.display()))?;
            file.write_all(&marker_contents)
                .with_context(|| format!("failed to write stage marker {}", marker.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync stage marker {}", marker.display()))?;
            sync_directory(parent)
        })();
        if let Err(error) = marker_result {
            let _ = std::fs::remove_file(&marker);
            return Err(error).with_context(|| {
                format!(
                    "failed to mark staged directory {} for destination {}",
                    stage.display(),
                    destination.display()
                )
            });
        }
        let identity =
            DirectoryIdentity::from_metadata(&std::fs::symlink_metadata(&stage).with_context(
                || format!("failed to inspect staged directory {}", stage.display()),
            )?);
        let stage = temporary.keep();
        Ok(Self {
            destination: destination.to_path_buf(),
            stage,
            marker,
            marker_contents,
            identity,
            published: false,
        })
    }

    /// Directory into which the caller builds and verifies the complete value.
    pub(crate) fn path(&self) -> &Path {
        &self.stage
    }

    /// Sync and rename this stage to its still-absent destination.
    pub(crate) fn publish(mut self) -> Result<PathBuf> {
        self.validate_owned_stage().with_context(|| {
            format!(
                "refusing to publish unmarked or replaced stage {} at {}; destination was not changed",
                self.stage.display(),
                self.destination.display()
            )
        })?;
        match std::fs::symlink_metadata(&self.destination) {
            Ok(_) => {
                bail!(
                    "refusing to replace existing destination {} with staged directory {}; destination was not changed",
                    self.destination.display(),
                    self.stage.display()
                )
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect destination {} before publishing staged directory {}; destination was not changed",
                        self.destination.display(),
                        self.stage.display()
                    )
                });
            }
        }

        sync_tree(&self.stage).with_context(|| {
            format!(
                "failed to sync staged directory {} before publishing at {}; destination was not changed",
                self.stage.display(),
                self.destination.display()
            )
        })?;
        rename_noreplace(&self.stage, &self.destination).with_context(|| {
            format!(
                "failed to publish staged directory {} at {}; destination was not changed",
                self.stage.display(),
                self.destination.display()
            )
        })?;
        self.published = true;

        std::fs::remove_file(&self.marker).with_context(|| {
            format!(
                "published staged directory at {}, but failed to remove its marker {}; the complete destination remains in place",
                self.destination.display(),
                self.marker.display()
            )
        })?;

        let parent = self
            .destination
            .parent()
            .expect("beside accepted a destination with a parent");
        sync_directory(parent).with_context(|| {
            format!(
                "published staged directory at {}, but failed to sync its parent {}; the complete destination remains in place",
                self.destination.display(),
                parent.display()
            )
        })?;
        Ok(self.destination.clone())
    }

    fn validate_owned_stage(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.stage)
            .with_context(|| format!("failed to inspect stage {}", self.stage.display()))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || !self.identity.matches(&metadata)
        {
            bail!(
                "stage path no longer identifies the directory Tonk created: {}",
                self.stage.display()
            );
        }
        let marker = std::fs::read(&self.marker)
            .with_context(|| format!("failed to read stage marker {}", self.marker.display()))?;
        if marker != self.marker_contents {
            bail!(
                "stage marker no longer matches destination {}: {}",
                self.destination.display(),
                self.marker.display()
            );
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn rename_noreplace(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-replace directory publication is unsupported on this platform",
    ))
}

impl Drop for StagedDirectory {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        match self.validate_owned_stage() {
            Ok(()) => {}
            Err(error) => {
                if matches!(
                    std::fs::symlink_metadata(&self.stage),
                    Err(ref missing) if missing.kind() == ErrorKind::NotFound
                ) && std::fs::read(&self.marker).ok().as_deref()
                    == Some(self.marker_contents.as_slice())
                {
                    let _ = std::fs::remove_file(&self.marker);
                    return;
                }
                eprintln!(
                    "warning: preserving stage {} because Tonk could not prove it still owns this directory for destination {}: {error:#}",
                    self.stage.display(),
                    self.destination.display()
                );
                return;
            }
        }
        match std::fs::remove_dir_all(&self.stage) {
            Ok(()) => {
                if let Err(error) = std::fs::remove_file(&self.marker)
                    && error.kind() != ErrorKind::NotFound
                {
                    eprintln!(
                        "warning: cleaned unpublished Tonk stage {}, but failed to remove marker {}: {error}",
                        self.stage.display(),
                        self.marker.display()
                    );
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let _ = std::fs::remove_file(&self.marker);
            }
            Err(error) => eprintln!(
                "warning: failed to clean unpublished Tonk stage {} for destination {}: {error}; the destination was not changed, and retry is safe after removing this stage",
                self.stage.display(),
                self.destination.display()
            ),
        }
    }
}

/// Whether a directory and sibling marker have the shape this module creates.
pub(crate) fn is_staged_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(marked) = name.strip_prefix(STAGE_PREFIX) else {
        return false;
    };
    let Some((label, random)) = marked.rsplit_once('-') else {
        return false;
    };
    let name_is_valid = !label.is_empty()
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && random.len() >= 6
        && random.bytes().all(|byte| byte.is_ascii_alphanumeric());
    if !name_is_valid {
        return false;
    }
    let Ok(marker) = marker_path(path).and_then(|marker| {
        std::fs::read(&marker).with_context(|| format!("failed to read {}", marker.display()))
    }) else {
        return false;
    };
    let Some(declaration) = marker.strip_prefix(MARKER_MAGIC) else {
        return false;
    };
    let Some(separator) = declaration.iter().position(|byte| *byte == 0) else {
        return false;
    };
    &declaration[..separator] == path.file_name().expect("name checked").as_encoded_bytes()
        && !declaration[separator + 1..].is_empty()
        && !declaration[separator + 1..].contains(&0)
}

fn marker_path(stage: &Path) -> Result<PathBuf> {
    let name = stage.file_name().ok_or_else(|| {
        anyhow!(
            "staged directory has no file name for its marker: {}",
            stage.display()
        )
    })?;
    let mut marker_name = name.to_os_string();
    marker_name.push(".marker");
    Ok(stage.with_file_name(marker_name))
}

fn marker_contents(stage: &Path, destination: &Path) -> Result<Vec<u8>> {
    let stage_name = stage.file_name().ok_or_else(|| {
        anyhow!(
            "staged directory has no file name for its marker: {}",
            stage.display()
        )
    })?;
    let destination_name = destination.file_name().ok_or_else(|| {
        anyhow!(
            "destination has no file name for its stage marker: {}",
            destination.display()
        )
    })?;
    let mut marker = Vec::with_capacity(
        MARKER_MAGIC.len()
            + stage_name.as_encoded_bytes().len()
            + destination_name.as_encoded_bytes().len()
            + 1,
    );
    marker.extend_from_slice(MARKER_MAGIC);
    marker.extend_from_slice(stage_name.as_encoded_bytes());
    marker.push(0);
    marker.extend_from_slice(destination_name.as_encoded_bytes());
    Ok(marker)
}

#[cfg(unix)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl DirectoryIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn matches(&self, metadata: &std::fs::Metadata) -> bool {
        self.device == metadata.dev() && self.inode == metadata.ino()
    }
}

#[cfg(not(unix))]
struct DirectoryIdentity;

#[cfg(not(unix))]
impl DirectoryIdentity {
    fn from_metadata(_metadata: &std::fs::Metadata) -> Self {
        Self
    }

    fn matches(&self, _metadata: &std::fs::Metadata) -> bool {
        // Publication is unsupported on these platforms, and without a
        // replacement-resistant identity Drop cannot prove this path still
        // names Tonk's directory. Preserve it rather than risk deleting data.
        false
    }
}

fn sync_tree(path: &Path) -> Result<()> {
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("failed to read staged directory {}", path.display()))?
    {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            sync_tree(&entry.path())?;
        } else if metadata.is_file() {
            File::open(entry.path())
                .and_then(|file| file.sync_all())
                .with_context(|| {
                    format!("failed to sync staged file {}", entry.path().display())
                })?;
        }
    }
    sync_directory(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("failed to sync directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};

    use super::*;

    #[test]
    fn publish_never_replaces_an_existing_destination() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("space");
        std::fs::create_dir(&destination)?;
        std::fs::write(destination.join("kept"), b"original")?;

        let staged = StagedDirectory::beside(&destination, "test")?;
        std::fs::write(staged.path().join("new"), b"replacement")?;
        let error = staged
            .publish()
            .expect_err("an existing destination must never be replaced");

        assert!(
            error.to_string().contains("refusing to replace"),
            "{error:#}"
        );
        assert_eq!(std::fs::read(destination.join("kept"))?, b"original");
        assert!(!destination.join("new").exists());
        Ok(())
    }

    #[test]
    fn the_publish_primitive_never_replaces_an_existing_empty_directory() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let staged = temp.path().join("staged");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&staged)?;
        std::fs::create_dir(&destination)?;

        rename_noreplace(&staged, &destination)
            .expect_err("no-replace publication must reject even an empty destination");

        assert!(staged.is_dir(), "the unpublished stage is preserved");
        assert!(
            destination.is_dir(),
            "the competing destination is preserved"
        );
        Ok(())
    }

    #[test]
    fn a_returned_build_error_cleans_the_unpublished_stage() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("space");
        let mut staged_path = None;
        let mut marker_path = None;

        let error: anyhow::Error = (|| -> Result<()> {
            let staged = StagedDirectory::beside(&destination, "test")?;
            staged_path = Some(staged.path().to_path_buf());
            marker_path = Some(staged.marker.clone());
            std::fs::write(staged.path().join("partial"), b"partial")?;
            bail!("injected build failure")
        })()
        .expect_err("the build fails before publication");

        assert!(error.to_string().contains("injected build failure"));
        assert!(!destination.exists());
        assert!(!staged_path.expect("stage was created").exists());
        assert!(!marker_path.expect("marker was created").exists());
        Ok(())
    }

    #[test]
    fn publish_renames_a_hidden_sibling_into_place() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("space");
        let staged = StagedDirectory::beside(&destination, "test")?;
        let staged_path = staged.path().to_path_buf();
        let marker_path = staged.marker.clone();

        assert_eq!(staged_path.parent(), destination.parent());
        assert!(
            staged_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tonk-stage-"))
        );
        assert!(is_staged_directory(staged.path()));
        std::fs::write(staged.path().join("complete"), b"complete")?;

        assert_eq!(staged.publish()?, destination);
        assert!(!staged_path.exists());
        assert!(!marker_path.exists());
        assert_eq!(std::fs::read(destination.join("complete"))?, b"complete");
        Ok(())
    }

    #[test]
    fn a_tampered_marker_preserves_the_directory_instead_of_deleting_it() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("space");
        let staged = StagedDirectory::beside(&destination, "test")?;
        let staged_path = staged.path().to_path_buf();
        let marker_path = staged.marker.clone();
        std::fs::write(staged.path().join("unknown"), b"do not delete")?;
        std::fs::write(&marker_path, b"not a Tonk marker")?;

        drop(staged);

        assert_eq!(
            std::fs::read(staged_path.join("unknown"))?,
            b"do not delete"
        );
        assert!(marker_path.exists());
        assert!(!destination.exists());
        Ok(())
    }

    #[test]
    fn a_losing_concurrent_stage_never_deletes_the_published_winner() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("space");
        let winner = StagedDirectory::beside(&destination, "account-pull")?;
        let loser = StagedDirectory::beside(&destination, "account-pull")?;
        let loser_path = loser.path().to_path_buf();
        std::fs::write(winner.path().join("copy"), b"winner")?;
        std::fs::write(loser.path().join("copy"), b"loser")?;

        winner.publish()?;
        loser
            .publish()
            .expect_err("the second publisher loses the canonical-name race");

        assert_eq!(std::fs::read(destination.join("copy"))?, b"winner");
        assert!(!loser_path.exists(), "only the losing stage is cleaned");
        Ok(())
    }
}
