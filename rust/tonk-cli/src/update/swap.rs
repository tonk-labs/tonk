//! Replacing the running binary.
//!
//! Everything is prepared on a temp file in the target's own
//! directory — extracted, permissioned, and smoke-tested — and the
//! `rename()` happens last. A failure at any
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
    ///
    /// npm owns npm-installed binaries. Its default is the final release;
    /// `@next` is the explicit prerelease channel.
    pub fn remedy(self) -> &'static str {
        match self {
            ForeignInstall::Nix => "update it through nix (e.g. `nix flake update`)",
            ForeignInstall::Npm => {
                "reinstall it through npm (`npm i -g @tonk/cli`, or `npm i -g \
                 @tonk/cli@next` for the prerelease channel)"
            }
        }
    }

    /// How this install is described in the refusal message.
    pub fn label(self) -> &'static str {
        match self {
            ForeignInstall::Nix => "a nix store path",
            ForeignInstall::Npm => "an npm install",
        }
    }

    /// The message shown when refusing to touch `target`.
    ///
    /// Shared by the early guard in `update::run` and by [`install`]'s
    /// own defense-in-depth check, so the wording lives in one place.
    pub fn refusal(self, target: &Path) -> String {
        format!(
            "{} is {} — tonk will not overwrite it; {}",
            target.display(),
            self.label(),
            self.remedy()
        )
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
/// The archive integrity gate, independent of the platform signature.
/// Mismatch is fatal.
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
            entry
                .read_to_end(&mut bytes)
                .context("could not read tonk from archive")?;
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
        bail!("{}", foreign.refusal(target));
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

    std::fs::rename(temp, target).with_context(|| format!("could not replace {}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.tar.gz` holding one entry named `tonk` with `body`.
    fn archive_with_bytes(body: &[u8]) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        header.set_path("tonk").expect("path");
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();

        let mut tar = tar::Builder::new(Vec::new());
        tar.append(&header, body).expect("append");
        let tar = tar.into_inner().expect("finish");

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).expect("gz");
        encoder.finish().expect("gz finish")
    }

    fn archive_with(body: &str) -> Vec<u8> {
        archive_with_bytes(body.as_bytes())
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
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
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
            foreign_install(Path::new(
                "/home/x/node_modules/@tonk/cli-linux-x64/bin/tonk"
            )),
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

        assert!(
            std::fs::read_to_string(&target)
                .expect("read")
                .contains("0.5.0")
        );
    }

    #[cfg(target_os = "macos")]
    #[dialog_common::test]
    fn it_preserves_signed_binary_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("tonk");
        let signed = std::fs::read("/usr/bin/true").expect("read signed system binary");
        let archive = archive_with_bytes(&signed);

        install(&archive, &sha_of(&archive), &target).expect("install");

        assert_eq!(
            Sha256::digest(std::fs::read(&target).expect("read installed binary")),
            Sha256::digest(&signed),
            "install must not replace the embedded Apple signature"
        );
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
    fn it_shares_the_refusal_message_between_the_helper_and_install() {
        let target = Path::new("/nix/store/abc/bin/tonk");
        let foreign = foreign_install(target).expect("foreign");
        let message = foreign.refusal(target);
        assert!(message.contains("will not overwrite"));
        assert!(message.contains("nix flake update"));

        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let sha = sha_of(&archive);
        let err = install(&archive, &sha, target).expect_err("must refuse");
        assert_eq!(err.to_string(), message);
    }

    #[dialog_common::test]
    fn it_refuses_to_overwrite_a_foreign_install() {
        let archive = archive_with("#!/bin/sh\necho 'tonk 0.5.0'\n");
        let sha = sha_of(&archive);
        let err =
            install(&archive, &sha, Path::new("/nix/store/abc/bin/tonk")).expect_err("must refuse");
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
