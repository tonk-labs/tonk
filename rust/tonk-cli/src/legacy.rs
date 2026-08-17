//! Migrating a CSV export written by a pre-dialog-upgrade build.
//!
//! The dialog upgrade renamed the schema namespace and retired part of it,
//! so an export taken under the old build cannot be imported as it stands:
//! the new build reserves `dialog.*` against application writes and refuses
//! the whole transaction on the first such row.
//!
//! The rename is mechanical — `dialog.attribute/id` became `db.attribute/id`,
//! and so on for 49 of the 58 names a real legacy export carries. What did
//! *not* survive is dialog's own bookkeeping: the old rules system
//! (`dialog.effect/*`), and the handful of markers the new build regenerates
//! for itself. Those are dropped rather than translated, because the new
//! build writes its own.
//!
//! Remapping is preferred over re-deriving the schema from
//! `tonk schema` output. Both work, but this carries the data the spot
//! actually had — a schema that drifted from the standard library keeps its
//! drift — and a table cannot fail halfway through a re-evaluation.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The last release that can read pre-upgrade data.
///
/// Named rather than resolved: the point of a pinned release is that it does
/// not move, so an upgrade path that asked for "latest compatible" would be
/// asking a question with a changing answer.
pub const LEGACY_RELEASE: &str = "v0.6.7";

/// Download and unpack the legacy CLI into `into`, returning its path.
///
/// The published artifact, not a build from source: it is the same thing the
/// install instructions name, so a broken release surfaces here rather than
/// in someone's terminal.
pub fn fetch_legacy_cli(into: &Path) -> Result<PathBuf> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        (os, arch) => bail!(
            "no published {LEGACY_RELEASE} build for {os}/{arch}; \
             export on a machine that has one, then `tonk import` here"
        ),
    };
    let url = format!(
        "https://github.com/tonk-labs/tonk/releases/download/\
         {LEGACY_RELEASE}/tonk-{platform}.tar.gz"
    );
    std::fs::create_dir_all(into)
        .with_context(|| format!("failed to create {}", into.display()))?;
    let archive = into.join("legacy.tar.gz");
    run(Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive)
        .arg(&url))
    .with_context(|| format!("failed to download {LEGACY_RELEASE} from {url}"))?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(into))
    .context("failed to unpack the legacy CLI")?;
    Ok(into.join("tonk"))
}

/// Export a branch using the legacy CLI, which is the only build that can
/// read pre-upgrade data.
///
/// `site` is the spot's own directory. The legacy CLI resolves a spot
/// through its registry rather than from the working directory, so it is
/// named explicitly with `--spot` — a caller holding a directory should not
/// have to register it first just to read it.
pub fn legacy_export(cli: &Path, spot: &str, branch: &str, out: &Path) -> Result<()> {
    let mut command = Command::new(cli);
    command
        .arg("export")
        .arg("--spot")
        .arg(spot)
        .arg("--out")
        .arg(out);
    // `--branch` is newer than the build being driven here: the legacy CLI
    // exports `main` and knows no flag for anything else. Passing it would
    // fail outright, so a non-default branch is refused with an explanation
    // rather than silently exporting the wrong one.
    if branch != crate::site::BRANCH_NAME {
        bail!(
            "{LEGACY_RELEASE} can only export {:?}; branch {branch:?} predates \
             per-branch export and cannot be migrated with it",
            crate::site::BRANCH_NAME
        );
    }
    run(command
        // The old build measures usage; a migration is not a user session.
        .env("DO_NOT_TRACK", "1"))
    .with_context(|| format!("the legacy CLI could not export branch {branch:?}"))
}

/// What upgrading one branch did.
#[derive(Debug, Clone)]
pub struct Upgraded {
    /// The branch that was upgraded.
    pub branch: String,
    /// How the export was rewritten on the way through.
    pub migration: Migration,
    /// The migrated CSV, ready to import.
    pub csv: PathBuf,
}

/// Export a legacy branch and rewrite it into something this build imports.
///
/// Stops short of importing: the caller owns the destination, and a
/// migration that both reads and writes in one step gives no chance to
/// inspect what is about to land.
pub fn upgrade_branch(cli: &Path, spot: &str, branch: &str, workspace: &Path) -> Result<Upgraded> {
    let exported = workspace.join(format!("{branch}-legacy.csv"));
    legacy_export(cli, spot, branch, &exported)?;

    let migrated = workspace.join(format!("{branch}-migrated.csv"));
    let source = std::fs::File::open(&exported)
        .with_context(|| format!("failed to read {}", exported.display()))?;
    let sink = std::fs::File::create(&migrated)
        .with_context(|| format!("failed to write {}", migrated.display()))?;
    let migration = migrate_export(std::io::BufReader::new(source), sink)?;

    Ok(Upgraded {
        branch: branch.to_owned(),
        migration,
        csv: migrated,
    })
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status().context("could not run the command")?;
    if !status.success() {
        bail!("command failed: {status}");
    }
    Ok(())
}

/// The attribute column is the first field of a row, and an attribute name
/// never contains a comma or a quote — so the rename can be applied to the
/// line's leading token without parsing the rest, which may contain
/// embedded newlines inside quoted values.
fn split_attribute(line: &str) -> Option<(&str, &str)> {
    let end = line.find(',')?;
    Some((&line[..end], &line[end..]))
}

/// Attribute families the new build owns and regenerates. Importing them is
/// what trips the reserved-namespace refusal, and translating them would be
/// worse: they would then disagree with what the new build wrote itself.
const DROP_PREFIXES: &[&str] = &[
    // The old rules system, superseded by dialog's native induction.
    "dialog.effect/",
    // Native rules, written by dialog at commit time.
    "dialog.rule/",
];

/// Individual attributes with the same story as [`DROP_PREFIXES`].
const DROP_EXACT: &[&str] = &[
    "dialog.concept/transient",
    "dialog.db/revision",
    "dialog.meta/effect",
];

/// What a migration did, so a caller can report it rather than guess.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Rows carried into the new export.
    pub kept: usize,
    /// Rows whose attribute moved from `dialog.` to `db.`.
    pub remapped: usize,
    /// Rows dropped because the new build owns that attribute.
    pub dropped: usize,
}

/// Rewrite a legacy CSV export into one the current build can import.
///
/// Streams rather than buffering: an export of a real spot is arbitrarily
/// large, and there is no reason to hold it in memory to rewrite one column.
pub fn migrate_export<R: BufRead, W: Write>(source: R, mut out: W) -> Result<Migration> {
    let mut migration = Migration::default();
    let mut lines = source.lines();
    let header = lines
        .next()
        .transpose()
        .context("failed to read the legacy export")?
        .context("legacy export has no header row")?;
    writeln!(out, "{header}").context("failed to write the header row")?;

    // A quoted value may span lines, so a row is not a line. Only a line
    // that begins a row carries the attribute column; the rest are its
    // continuation and must travel with it — including when the row is
    // dropped, or its tail would be left behind as a malformed record.
    //
    // Oddness of the running quote count decides which is which: inside a
    // quoted value the count so far is odd, and the line that closes it
    // makes it even again.
    let mut inside_quotes = false;
    let mut dropping = false;
    for line in lines {
        let line = line.context("failed to read a legacy row")?;
        let starts_row = !inside_quotes;
        inside_quotes ^= line.matches('"').count() % 2 == 1;

        if !starts_row {
            if !dropping {
                writeln!(out, "{line}").context("failed to write a migrated row")?;
            }
            continue;
        }

        let Some((the, rest)) = split_attribute(&line) else {
            dropping = false;
            writeln!(out, "{line}").context("failed to write a migrated row")?;
            continue;
        };
        if DROP_PREFIXES.iter().any(|p| the.starts_with(p)) || DROP_EXACT.contains(&the) {
            migration.dropped += 1;
            dropping = true;
            continue;
        }
        dropping = false;
        match rename(the) {
            Some(renamed) => {
                migration.remapped += 1;
                writeln!(out, "{renamed}{rest}").context("failed to write a migrated row")?;
            }
            None => writeln!(out, "{line}").context("failed to write a migrated row")?,
        }
        migration.kept += 1;
    }
    out.flush().context("failed to flush the migrated export")?;
    Ok(migration)
}

/// `dialog.<rest>` becomes `db.<rest>`; anything else is left alone.
fn rename(value: &str) -> Option<String> {
    value
        .strip_prefix("dialog.")
        .map(|rest| format!("db.{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_renames_the_schema_namespace() {
        assert_eq!(
            rename("dialog.attribute/id").as_deref(),
            Some("db.attribute/id")
        );
        assert_eq!(
            rename("dialog.concept.with/name").as_deref(),
            Some("db.concept.with/name")
        );
        // An application attribute is not dialog's to rename.
        assert_eq!(rename("xyz.tonk.view/model"), None);
    }

    /// A quoted value may span lines, so a row is not a line. Treating each
    /// line as a row let the line *after* a multi-line value be read as a
    /// fresh record — which slipped a reserved `dialog.*` attribute past the
    /// filter and made a real legacy import fail at commit.
    #[test]
    fn it_carries_multi_line_values_with_their_row() {
        let legacy = "the,of,as,is,cause\n\
             db.meta/description,concept:A,text,\"first line\n\
             second line\",\n\
             dialog.concept.with/profile,concept:B,entity,the:C,\n";
        let mut out = Vec::new();
        let migration = migrate_export(legacy.as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(
            text.contains("second line"),
            "the continuation must travel with its row; got:\n{text}"
        );
        assert!(
            !text.contains("dialog."),
            "a row after a multi-line value is still a row, and this one \
             renames; got:\n{text}"
        );
        assert_eq!(migration.remapped, 1, "only the second row is `dialog.*`");
    }

    /// A dropped row takes its continuation lines with it, or the tail is
    /// left behind as a record with no attribute column.
    #[test]
    fn it_drops_a_multi_line_row_whole() {
        let legacy = "the,of,as,is,cause\n\
             dialog.effect/source,rule:A,text,\"kept\n\
             orphan\",\n\
             xyz.app/title,thing:B,text,fine,\n";
        let mut out = Vec::new();
        let migration = migrate_export(legacy.as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(
            !text.contains("orphan"),
            "a dropped row must not leave its tail behind; got:\n{text}"
        );
        assert!(
            text.contains("xyz.app/title"),
            "the next row still survives"
        );
        assert_eq!(migration.dropped, 1);
        assert_eq!(migration.kept, 1);
    }

    /// Dialog's own bookkeeping is dropped, not translated: the new build
    /// writes its own, and a translated copy would contradict it.
    #[test]
    fn it_drops_what_the_new_build_owns() {
        let legacy = "the,of,as,is,cause\n\
             dialog.effect/on,concept:A,entity,db:concept,\n\
             dialog.rule/source,rule:B,bytes,00,\n\
             dialog.attribute/id,the:C,text,name,\n";
        let mut out = Vec::new();
        let migration = migrate_export(legacy.as_bytes(), &mut out).unwrap();

        assert_eq!(
            migration.dropped, 2,
            "both effect and rule rows are dialog's"
        );
        assert_eq!(migration.kept, 1);
        assert_eq!(migration.remapped, 1);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("db.attribute/id"));
        assert!(!text.contains("dialog."));
    }
}
