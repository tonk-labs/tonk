//! Upgrading a spot written by the pre-dialog-upgrade build, end to end.
//!
//! Every step runs for real: the published `v0.6.7` binary is downloaded,
//! handed a spot directory that binary itself created, and asked to export
//! it — because only it can read that format. The export is remapped, the
//! current build imports it, and the note the old build wrote is queried
//! back.
//!
//! Gated behind the `legacy-migration` feature rather than the usual
//! integration one: it reaches the network for a release archive, which the
//! ordinary test run should not depend on.

mod common;

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The release the migration instructions name, and the last before the
/// dialog upgrade — it pins dialog `rev = e8bbe462`.
const LEGACY_RELEASE: &str = "v0.6.7";

/// Download and unpack the published legacy CLI, returning its path.
///
/// The real published artifact rather than a local build of the old ref:
/// what the migration instructions tell a user to install is exactly what
/// this exercises, so a missing or broken release fails here instead of in
/// someone's terminal.
fn legacy_binary(into: &Path) -> Result<PathBuf> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        (os, arch) => bail!("no published legacy CLI for {os}/{arch}"),
    };
    let url = format!(
        "https://github.com/tonk-labs/tonk/releases/download/\
         {LEGACY_RELEASE}/tonk-{platform}.tar.gz"
    );
    let archive = into.join("legacy.tar.gz");
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive)
        .arg(&url)
        .status()
        .context("could not run curl")?;
    if !status.success() {
        bail!("failed to download the legacy CLI from {url}");
    }
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(into)
        .status()
        .context("could not run tar")?;
    if !status.success() {
        bail!("failed to unpack the legacy CLI");
    }
    Ok(into.join("tonk"))
}

/// Run the legacy CLI against the fixture's isolated home.
///
/// `HOME` is overridden because the spot registry resolves through
/// `dirs::data_dir()`, which on macOS ignores any tonk-specific variable and
/// would otherwise write into the developer's real registry.
fn legacy_run(cli: &Path, home: &Path, work: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new(cli)
        .args(args)
        .current_dir(work)
        .env("HOME", home)
        .env("TONK_UNSAFE_ALLOW_DEVICE_ROOT", "1")
        // A test run is not a user to be measured.
        .env("DO_NOT_TRACK", "1")
        .status()
        .with_context(|| format!("could not run the legacy CLI: {args:?}"))?;
    if !status.success() {
        bail!("legacy CLI failed: {args:?}");
    }
    Ok(())
}

/// A spot written by the old build, upgraded and still answering as itself.
///
/// The identity half is the point. A migrated spot that answers with the
/// right values under *new* entities is a copy, not an upgrade: its peers
/// would treat it as a different spot. So this asserts the exact DID the old
/// build minted, not merely that a row with the right title came back.
#[tokio::test]
#[cfg_attr(not(feature = "legacy-migration"), ignore)]
async fn it_upgrades_a_legacy_spot_end_to_end() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let legacy_cli = legacy_binary(workspace.path())?;

    // The fixture is the old build's own on-disk database. This build cannot
    // read it — opening its branch fails with `missing field 'branch'` —
    // which is precisely why the export must run under the old binary.
    let home = workspace.path().join("legacy-home");
    let spots = home.join("Library/Application Support/tonk/spots");
    std::fs::create_dir_all(&spots)?;
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-spot-v0.6.7.tar.gz");
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&fixture)
        .arg("-C")
        .arg(&spots)
        .status()?;
    if !status.success() {
        bail!("failed to unpack the legacy spot fixture");
    }

    // The registry that names the spot. It is written here rather than
    // committed because it stores absolute paths, which would be this
    // machine's and no one else's.
    let spot = spots.join("legacy");
    let registry = serde_json::json!({
        "spots": { "legacy": { "site": spot } },
        "bindings": {},
    });
    std::fs::write(
        spots
            .parent()
            .context("spots directory has no parent")?
            .join("spots.json"),
        serde_json::to_vec_pretty(&registry)?,
    )?;

    // Export with the old binary: the step nothing else can perform.
    let work = workspace.path().join("work");
    std::fs::create_dir_all(&work)?;
    let export = workspace.path().join("legacy.csv");
    // `tonk use`, not `tonk spot use`: binding a directory to a spot is a
    // top-level verb in this release.
    legacy_run(&legacy_cli, &home, &work, &["use", "legacy"])?;
    legacy_run(
        &legacy_cli,
        &home,
        &work,
        &[
            "export",
            "--out",
            export.to_str().context("non-UTF-8 path")?,
        ],
    )?;

    // Remap the schema namespace the upgrade renamed.
    let source = std::fs::File::open(&export)?;
    let mut migrated = Vec::new();
    let report = tonk_cli::legacy::migrate_export(BufReader::new(source), &mut migrated)?;
    assert!(
        report.remapped > report.dropped * 10,
        "the rename carries the schema and the drop list is dialog's own \
         bookkeeping; remapped {} dropped {}",
        report.remapped,
        report.dropped
    );

    // Import under the current build, and ask the old build's note for itself.
    let site = common::TestSite::new().await?;
    let path = site.tmp.path().join("migrated.csv");
    std::fs::write(&path, &migrated)?;
    tonk_cli::transfer::import(&site.site, &path).await?;

    let rows = site
        .eval_inline("note:\n  this: ?this\n  title: ?title\n")
        .await?
        .stdout;
    assert!(
        rows.contains("written by the old build"),
        "the upgraded spot must still answer the legacy note query; saw:\n{rows}"
    );
    assert!(
        rows.contains("did:key:z6Mk3VY17HUDh9rW6UpiDdtF9BGmdqfsYC2ZGzk4rAJadk2H"),
        "the note must keep the entity the old build minted, or this is a \
         copy rather than an upgrade; saw:\n{rows}"
    );
    Ok(())
}
