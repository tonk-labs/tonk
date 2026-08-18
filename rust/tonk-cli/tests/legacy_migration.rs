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

/// Runtime-owned fields are the migration boundary that the Vault depends on:
/// rewriting the CSV text is insufficient unless a current replica can use
/// the imported concept and see its locally injected identity facts.
#[tokio::test]
async fn migrated_runtime_fields_bind_the_current_replica() -> Result<()> {
    let legacy = "the,of,as,is,cause\n\
        dialog.attribute/cardinality,the:legacy-profile,text,one,\n\
        dialog.attribute/id,the:legacy-profile,text,dialog.origin/profile,\n\
        dialog.attribute/type,the:legacy-profile,text,Entity,\n\
        dialog.meta/description,the:legacy-profile,text,Legacy profile.,\n\
        dialog.attribute/cardinality,the:legacy-subject,text,one,\n\
        dialog.attribute/id,the:legacy-subject,text,dialog.origin/subject,\n\
        dialog.attribute/type,the:legacy-subject,text,Entity,\n\
        dialog.meta/description,the:legacy-subject,text,Legacy subject.,\n\
        dialog.concept.with/profile,tonk:migrated-runtime,entity,the:legacy-profile,\n\
        dialog.concept.with/subject,tonk:migrated-runtime,entity,the:legacy-subject,\n\
        dialog.meta/concept,tonk:migrated-runtime,entity,db:concept,\n\
        db.name/referent,id:migrated/runtime,entity,tonk:migrated-runtime,\n";
    let mut migrated = Vec::new();
    tonk_cli::legacy::migrate_export(legacy.as_bytes(), &mut migrated)?;

    let site = common::TestSite::new().await?;
    // Model a destination imported by the pre-fix migration: schema columns
    // moved into `db.*`, but the runtime selector values stayed at
    // `dialog.origin/*`. A corrected rerun must repair this state rather than
    // requiring the user to discover that the first attempt poisoned it.
    let pre_fix = legacy.replace("\ndialog.", "\ndb.");
    let path = site.tmp.path().join("pre-fix-runtime-fields.csv");
    std::fs::write(&path, pre_fix)?;
    tonk_cli::transfer::import(&site.site, &path).await?;
    let before = site
        .eval_inline("migrated/runtime:\n  subject: ?subject\n  profile: ?profile\n")
        .await?;
    assert!(
        before.response.matches_after[0].results.is_empty(),
        "the pre-fix fixture must reproduce the unbound runtime concept"
    );

    let path = site.tmp.path().join("runtime-fields.csv");
    std::fs::write(&path, migrated)?;
    tonk_cli::legacy::import_migrated_branch(&site.site, "main", &path).await?;

    let outcome = site
        .eval_inline("migrated/runtime:\n  subject: ?subject\n  profile: ?profile\n")
        .await?;
    let results = &outcome.response.matches_after[0].results;
    assert_eq!(
        results.len(),
        1,
        "a migrated runtime-bound concept must match this replica; saw:\n{}",
        outcome.stdout
    );
    Ok(())
}

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

/// The whole `tonk migrate --legacy` command, credentials included.
///
/// The test above drives the pieces directly; this one runs the command a
/// person would run, against a fixture that has an account attached. That
/// ordering is the thing under test: the account carries the authority every
/// migrated repository's chain terminates in, so a run that upgrades the data
/// and leaves the credentials behind produces a spot that reads locally and
/// cannot be pushed anywhere.
// Gated by `cfg` rather than `cfg_attr(.., ignore)`: `dialog_common::test`
// does not carry a trailing attribute through, so an `ignore` written the
// way the test above writes it is silently dropped and the test runs in CI,
// where it fails for want of a fixture it downloads.
#[cfg(feature = "legacy-migration")]
#[dialog_common::test]
async fn it_migrates_credentials_before_repositories(
    env: tonk_access_service::helpers::AccessServiceAddress,
) -> Result<()> {
    let endpoint = env.access_service_url.trim_end_matches('/').to_owned();
    // Every `tonk` invocation below is a blocking subprocess, and the access
    // service it pushes to is running on this runtime's thread. Block that
    // thread directly and the push can never be answered.
    tokio::task::spawn_blocking(move || migrate_and_publish(&endpoint))
        .await
        .context("the migration steps join")?
}

fn migrate_and_publish(endpoint: &str) -> Result<()> {
    let workspace = tempfile::tempdir()?;

    // A fixture written by the old build *with an account linked*, which is
    // the half `legacy-spot-v0.6.7` lacks: it carries the certificate
    // directory as well as the repositories.
    let home = workspace.path().join("home");
    let state = home.join("Library/Application Support/tonk");
    std::fs::create_dir_all(&state)?;
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-account-v0.6.7.tar.gz");
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&fixture)
        .arg("-C")
        .arg(&state)
        .status()?;
    if !status.success() {
        bail!("failed to unpack the legacy account fixture");
    }

    // Both repositories in the fixture must be unreadable to this build
    // before anything runs, or the migration under test is a no-op and the
    // assertions below would pass without it having done anything.
    for repo in ["linked/main", ".tonk/main"] {
        let revision = state.join(repo).join("memory/branch/main/revision");
        let bytes = std::fs::read(&revision)
            .with_context(|| format!("fixture is missing {}", revision.display()))?;
        assert_eq!(
            tonk_account::readability(Some(&bytes)),
            tonk_account::Readability::Legacy,
            "{repo} must start in the pre-upgrade format"
        );
    }

    // The fixture was produced by a test harness that isolates its profile
    // under the spot directory; the shipped CLI reads its profile from
    // `dialog/<name>`. Move it there so the command sees these credentials
    // rather than quietly minting a fresh, empty profile.
    let certificates = home.join("Library/Application Support/dialog/tonk");
    std::fs::create_dir_all(
        certificates
            .parent()
            .context("profile directory has no parent")?,
    )?;
    std::fs::rename(state.join("_profile/tonk"), &certificates)
        .context("the fixture must carry a profile named `tonk`")?;
    let before = count_files(&certificates)?;
    assert!(before > 0, "the fixture must carry account certificates");

    // Credentials first, and separately: this is the account root the old
    // build minted, which every migrated repository's authority chain has to
    // terminate in.
    const ACCOUNT_ROOT: &str = "did:key:z6MkhFDyBYNT1Y1jNj8RJKVc7CWurCVPmrnGEGmbYxvwHJkX";

    // The registry naming the fixture's data repository, written here because
    // it stores absolute paths that belong to this run alone.
    let registry = serde_json::json!({
        "spots": { "linked": { "site": state.join("linked") } },
        "bindings": {},
    });
    std::fs::write(
        state.join("spots.json"),
        serde_json::to_vec_pretty(&registry)?,
    )?;

    // Run the command itself, as a person would.
    //
    // `--site` names the source to export from; the destination is whichever
    // spot is active here, so the run needs one of this build's own making
    // to import into.
    let work = workspace.path().join("work");
    std::fs::create_dir_all(&work)?;
    // Credentials first. The fixture's certificate store is in the old
    // format, so until this runs the build cannot see an account at all --
    // `spot new` below refuses outright with "A Tonk account is required".
    // That refusal is why the account step has to lead.
    let migrated_account = Command::new(env!("CARGO_BIN_EXE_tonk"))
        .args(["account", "migrate"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("DO_NOT_TRACK", "1")
        .output()
        .context("running tonk account migrate failed")?;
    if !migrated_account.status.success() {
        bail!(
            "tonk account migrate failed: {}",
            String::from_utf8_lossy(&migrated_account.stderr)
        );
    }
    let account_report = String::from_utf8_lossy(&migrated_account.stdout).into_owned();
    assert!(
        !account_report.contains("migrated 0 certificates"),
        "the account migration must move the fixture's certificates, not \
         start from an empty profile; saw:\n{account_report}"
    );

    // Migrating the credentials is what makes the account visible at all.
    let status = Command::new(env!("CARGO_BIN_EXE_tonk"))
        .args(["account", "status"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("DO_NOT_TRACK", "1")
        .output()
        .context("running tonk account status failed")?;
    let status = String::from_utf8_lossy(&status.stdout).into_owned();
    assert!(
        status.contains(ACCOUNT_ROOT),
        "after migrating, the profile must answer with the account root the \
         old build minted; saw:\n{status}"
    );

    // `spot new` registers and binds in one step.
    let created = Command::new(env!("CARGO_BIN_EXE_tonk"))
        .args(["spot", "new", "upgraded"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("DO_NOT_TRACK", "1")
        .output()
        .context("running tonk spot new failed")?;
    if !created.status.success() {
        bail!(
            "tonk spot new failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_tonk"))
        .args(["migrate", "--legacy", "--site", "linked"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("DO_NOT_TRACK", "1")
        .output()
        .context("running tonk migrate failed")?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "tonk migrate --legacy failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // Credentials first: the run must say so, and it must say so before it
    // reports any repository work. Ordering is the invariant, not merely
    // that both happened.
    let account_at = stdout
        .find("account:")
        .with_context(|| format!("no account migration in the output:\n{stdout}"))?;
    if let Some(branch_at) = stdout.find("main") {
        assert!(
            account_at < branch_at,
            "credentials must migrate before repositories; saw:\n{stdout}"
        );
    }

    // The certificates survived the run rather than being dropped by it.
    assert!(
        count_files(&certificates)? >= before,
        "the migration must not discard account certificates"
    );

    // The data half: the old build's note, answering under the entity the
    // old build minted. A row with the right title under a fresh entity
    // would be a copy rather than an upgrade.
    std::fs::write(
        work.join("q.tonk"),
        "note:\n  this: ?this\n  title: ?title\n",
    )?;
    let queried = Command::new(env!("CARGO_BIN_EXE_tonk"))
        .args(["eval", "q.tonk"])
        .current_dir(&work)
        .env("HOME", &home)
        .env("DO_NOT_TRACK", "1")
        .output()
        .context("running tonk eval failed")?;
    let rows = String::from_utf8_lossy(&queried.stdout).into_owned();
    assert!(
        rows.contains("written by the old build with an account"),
        "the migrated spot must answer the legacy note query; saw:\n{rows}"
    );
    assert!(
        rows.contains("did:key:z6Mk68TsruaH39SRJqbBn5PJNzEuCBxAmePdWqSz9hqPxpfn"),
        "the note must keep the entity the old build minted; saw:\n{rows}"
    );

    // Publishing the result is the part that proves the credentials came
    // across intact: the push is authorized by the migrated account, so a
    // migration that upgraded only the data would fail right here.
    let tonk = |args: &[&str]| -> Result<std::process::Output> {
        Command::new(env!("CARGO_BIN_EXE_tonk"))
            .args(args)
            .current_dir(&work)
            .env("HOME", &home)
            .env("DO_NOT_TRACK", "1")
            .output()
            .with_context(|| format!("running tonk {args:?} failed"))
    };
    let added = tonk(&["remote", "add", "origin", endpoint])?;
    if !added.status.success() {
        bail!(
            "tonk remote add failed: {}",
            String::from_utf8_lossy(&added.stderr)
        );
    }
    let upstream = tonk(&["remote", "set-upstream", "origin"])?;
    if !upstream.status.success() {
        bail!(
            "tonk remote set-upstream failed: {}",
            String::from_utf8_lossy(&upstream.stderr)
        );
    }
    let pushed = tonk(&["push"])?;
    assert!(
        pushed.status.success(),
        "the migrated spot must publish\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pushed.stdout),
        String::from_utf8_lossy(&pushed.stderr)
    );

    Ok(())
}

/// Number of regular files beneath a directory, recursively.
fn count_files(root: &Path) -> Result<usize> {
    let mut total = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                total += 1;
            }
        }
    }
    Ok(total)
}
