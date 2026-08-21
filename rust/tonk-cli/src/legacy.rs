//! Migrating a CSV export written by a pre-dialog-upgrade build.
//!
//! The dialog upgrade renamed the schema namespace and retired part of it,
//! so an export taken under the old build cannot be imported as it stands:
//! the new build reserves `dialog.*` against application writes and refuses
//! the whole transaction on the first such row.
//!
//! Most renames are mechanical — `dialog.attribute/id` became
//! `db.attribute/id`, and so on for 49 of the 58 names a real legacy export
//! carries. Runtime-injected replica attributes also changed from
//! `dialog.origin/*` to `dialog.replica/*`; those names live inside schema
//! rows and are rewritten explicitly. What did *not* survive is dialog's own
//! bookkeeping: the old rules system (`dialog.effect/*`) and command markers.
//! Those reserved rows cannot travel through ordinary CSV import. They are
//! excluded from the application CSV, then translated after import into
//! native `dialog.rule/*` facts and current boolean transient markers.
//!
//! Remapping is preferred over re-deriving the schema from
//! `tonk schema` output. Both work, but this carries the data the space
//! actually had — a schema that drifted from the standard library keeps its
//! drift — and a table cannot fail halfway through a re-evaluation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use dialog_artifacts::{Entity, Value};
use dialog_csv::CsvImporter;
use dialog_query::{Output as _, Query, Term};
use futures_util::StreamExt as _;
use serde_json::Value as JsonValue;
use tonk_schema::meta::{AnonymousAttribute, attribute};

/// The last release that can read pre-upgrade data.
///
/// Named rather than resolved: the point of a pinned release is that it does
/// not move, so an upgrade path that asked for "latest compatible" would be
/// asking a question with a changing answer.
pub const LEGACY_RELEASE: &str = "v0.6.7";

/// How [`LEGACY_RELEASE`] spells `--space`.
///
/// That build predates the rename and knows only `--spot`, so this is the
/// argument of an external program rather than a name this crate is free to
/// keep in step with its own vocabulary.
const LEGACY_SPACE_FLAG: &str = "--spot";

/// How [`LEGACY_RELEASE`] spells `TONK_SPACES_STATE`.
const LEGACY_STATE_ENV: &str = "TONK_SPOTS_STATE";

/// The throwaway registry directory inside a migration workspace.
fn legacy_state(workspace: &Path) -> PathBuf {
    workspace.join("legacy-state")
}

/// Give [`LEGACY_RELEASE`] a registry it can resolve `space` through.
///
/// That build reads `spots.json` keyed on `spots`, which a store converted to
/// the current layout no longer has. Rather than keep the old file around for
/// it, this writes a registry naming exactly the one space being migrated
/// into the workspace and points the child at it — so the child resolves the
/// name it was given without reading, or being able to damage, the real
/// store.
pub fn prepare_legacy_registry(workspace: &Path, space: &str, site: &Path) -> Result<()> {
    let dir = legacy_state(workspace);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let registry = serde_json::json!({ "spots": { space: { "site": site } } });
    std::fs::write(
        dir.join("spots.json"),
        serde_json::to_string_pretty(&registry).context("failed to encode the legacy registry")?,
    )
    .with_context(|| format!("failed to write the legacy registry in {}", dir.display()))
}

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
/// `site` is the space's own directory. The legacy CLI resolves a space
/// through its registry rather than from the working directory, so it is
/// named explicitly with [`LEGACY_SPACE_FLAG`] — a caller holding a directory
/// should not have to register it first just to read it.
pub fn legacy_export(
    cli: &Path,
    space: &str,
    branch: &str,
    out: &Path,
    workspace: &Path,
) -> Result<()> {
    let mut command = Command::new(cli);
    command
        .arg("export")
        .arg(LEGACY_SPACE_FLAG)
        .arg(space)
        .arg("--out")
        .arg(out)
        .env(LEGACY_STATE_ENV, legacy_state(workspace));
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
    /// The original export, retained to translate legacy runtime metadata.
    pub legacy_csv: PathBuf,
    /// Content-addressed blobs referenced by the migrated facts.
    pub blobs: Vec<Entity>,
}

/// Find every valid `blob:<hash>` entity referenced by an export.
///
/// Blob references may be subjects, entity-valued facts, or embedded in
/// rendered text such as HTML, so discovery scans the complete CSV payload
/// rather than only one column.
pub fn blob_references<R: BufRead>(mut source: R) -> Result<Vec<Entity>> {
    let mut export = String::new();
    source
        .read_to_string(&mut export)
        .context("failed to read the export while finding blobs")?;

    let mut references = BTreeSet::new();
    for (start, _) in export.match_indices("blob:") {
        let candidate: String = export[start + "blob:".len()..]
            .chars()
            .take_while(|character| {
                matches!(
                    character,
                    '1'..='9'
                        | 'A'..='H'
                        | 'J'..='N'
                        | 'P'..='Z'
                        | 'a'..='k'
                        | 'm'..='z'
                )
            })
            .collect();
        let Ok(entity) = format!("blob:{candidate}").parse::<Entity>() else {
            continue;
        };
        if entity.blob_hash().is_some() {
            references.insert(entity);
        }
    }
    Ok(references.into_iter().collect())
}

/// Export a legacy branch and rewrite it into something this build imports.
///
/// Stops short of importing: the caller owns the destination, and a
/// migration that both reads and writes in one step gives no chance to
/// inspect what is about to land.
pub fn upgrade_branch(cli: &Path, space: &str, branch: &str, workspace: &Path) -> Result<Upgraded> {
    let exported = workspace.join(format!("{branch}-legacy.csv"));
    legacy_export(cli, space, branch, &exported, workspace)?;

    let migrated = workspace.join(format!("{branch}-migrated.csv"));
    let source = std::fs::File::open(&exported)
        .with_context(|| format!("failed to read {}", exported.display()))?;
    let sink = std::fs::File::create(&migrated)
        .with_context(|| format!("failed to write {}", migrated.display()))?;
    let migration = migrate_export(std::io::BufReader::new(source), sink)?;
    let migrated_source = std::fs::File::open(&migrated)
        .with_context(|| format!("failed to read {}", migrated.display()))?;
    let blobs = blob_references(std::io::BufReader::new(migrated_source))?;

    Ok(Upgraded {
        branch: branch.to_owned(),
        migration,
        csv: migrated,
        legacy_csv: exported,
        blobs,
    })
}

/// What copying the referenced blob payloads into a current branch did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlobMigration {
    /// Payloads copied and verified by their content address.
    pub copied: usize,
    /// Total payload bytes copied.
    pub bytes: u64,
}

/// Copy legacy blob payloads into a current branch without inventing facts.
///
/// The legacy CLI is the reader because the current build cannot open the
/// source branch. Each payload is streamed to a temporary file, attached to
/// the destination, and accepted only when the resulting content address is
/// exactly the entity referenced by the migrated facts.
pub async fn migrate_blobs(
    cli: &Path,
    space: &str,
    branch: &str,
    blobs: &[Entity],
    destination: &crate::site::TonkSite,
    workspace: &Path,
) -> Result<BlobMigration> {
    let mut migration = BlobMigration::default();
    for (index, expected) in blobs.iter().enumerate() {
        let payload = workspace.join(format!("legacy-blob-{index}"));
        let output = std::fs::File::create(&payload)
            .with_context(|| format!("failed to create {}", payload.display()))?;
        let status = Command::new(cli)
            .arg("blob")
            .arg("cat")
            .arg(expected.as_str())
            .arg(LEGACY_SPACE_FLAG)
            .arg(space)
            .env(LEGACY_STATE_ENV, legacy_state(workspace))
            .env("DO_NOT_TRACK", "1")
            .stdout(Stdio::from(output))
            .status()
            .with_context(|| format!("could not read legacy blob {expected}"))?;
        if !status.success() {
            bail!("legacy blob {expected} could not be read: {status}");
        }

        let attached = crate::blob::attach(destination, branch, &payload)
            .await
            .with_context(|| format!("failed to attach legacy blob {expected}"))?;
        if attached.entity != *expected {
            bail!(
                "legacy blob content address mismatch: expected {expected}, got {}",
                attached.entity
            );
        }
        migration.copied += 1;
        migration.bytes += attached.size;

        let completed = index + 1;
        if completed == 1 || completed % 25 == 0 || completed == blobs.len() {
            eprintln!("blobs: {completed}/{} verified", blobs.len());
        }
    }
    Ok(migration)
}

/// Import a rewritten branch and repair runtime attribute IDs left by an
/// earlier, pre-fix migration attempt.
///
/// CSV import is additive. If a destination already contains
/// `dialog.origin/*` values, importing their corrected `dialog.replica/*`
/// counterparts does not retract the stale cardinality-one claims. Explicitly
/// retracting those claims makes a corrected migration safe to rerun and lets
/// identity-bound concepts such as `vault/tree` resolve again.
pub async fn import_migrated_branch(
    destination: &crate::site::TonkSite,
    branch: &str,
    csv: &PathBuf,
) -> Result<usize> {
    crate::transfer::import_branch(destination, branch, csv)
        .await
        .with_context(|| format!("failed to import migrated branch {branch:?}"))?;
    repair_runtime_attribute_ids(destination, branch).await
}

/// Runtime-owned behavior restored after importing a legacy application tree.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeRepair {
    /// Schema attribute IDs repaired from `dialog.origin/*` to `dialog.replica/*`.
    pub runtime_attributes: usize,
    /// Legacy command concepts marked transient in the current representation.
    pub transient_concepts: usize,
    /// Legacy effects installed as content-addressed native dialog rules.
    pub native_rules: usize,
    /// Unmarked source/polarity debris ignored from the legacy effect index.
    pub ignored_effects: usize,
}

/// Import a rewritten branch and translate the runtime metadata that cannot
/// travel through CSV under the current reserved `dialog.*` namespace.
///
/// A legacy export stores command semantics in two runtime-owned families:
/// `dialog.concept/transient` marks a concept as dispatch-only, and
/// `dialog.effect/*` stores the rule fired by that command. Dropping those
/// rows imports the visible application data but leaves event handlers inert.
pub async fn import_upgraded_branch(
    destination: &crate::site::TonkSite,
    branch: &str,
    csv: &PathBuf,
    legacy_csv: &Path,
) -> Result<RuntimeRepair> {
    // Decode first: an incompatible real effect should fail before any
    // application rows are committed, leaving the destination untouched.
    eprintln!("runtime: decoding legacy commands and effects …");
    let runtime = legacy_runtime(legacy_csv).await?;
    eprintln!("runtime: importing migrated application rows …");
    crate::transfer::import_branch(destination, branch, csv)
        .await
        .with_context(|| format!("failed to import migrated branch {branch:?}"))?;
    eprintln!("runtime: repairing replica attribute bindings …");
    let runtime_attributes = repair_runtime_attribute_ids(destination, branch).await?;

    if runtime.transient_concepts.is_empty() && runtime.rules.is_empty() {
        return Ok(RuntimeRepair {
            runtime_attributes,
            ignored_effects: runtime.ignored_effects,
            ..RuntimeRepair::default()
        });
    }

    let session = destination
        .named_branch(branch)
        .await
        .with_context(|| format!("failed to open migrated branch {branch:?}"))?;
    let transient_count = runtime.transient_concepts.len();
    let rule_count = runtime.rules.len();
    eprintln!("runtime: installing {transient_count} commands and {rule_count} native rules …");
    let mut transaction = session.handle().transaction();
    for concept in runtime.transient_concepts {
        transaction = transaction.assert(dialog_repository::Transient(concept));
    }
    for rule in runtime.rules.into_values() {
        transaction = transaction.assert(rule);
    }
    transaction
        .commit()
        .perform(&destination.operator)
        .await
        .with_context(|| format!("failed to restore legacy runtime behavior on {branch:?}"))?;
    eprintln!("runtime: legacy command behavior restored");

    Ok(RuntimeRepair {
        runtime_attributes,
        transient_concepts: transient_count,
        native_rules: rule_count,
        ignored_effects: runtime.ignored_effects,
    })
}

#[derive(Default)]
struct LegacyEffect {
    source: Option<String>,
    polarity: Option<String>,
}

#[derive(Default)]
struct LegacyRuntime {
    transient_concepts: BTreeSet<Entity>,
    rules: BTreeMap<Entity, dialog_query::InductiveRule>,
    ignored_effects: usize,
}

async fn legacy_runtime(legacy_csv: &Path) -> Result<LegacyRuntime> {
    let path = legacy_csv.to_owned();
    let runtime_csv = tokio::task::spawn_blocking(move || {
        let source = std::fs::File::open(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut runtime = Vec::new();
        extract_legacy_runtime(std::io::BufReader::new(source), &mut runtime)?;
        Ok::<_, anyhow::Error>(runtime)
    })
    .await
    .context("legacy runtime scan task failed")??;
    let mut artifacts = CsvImporter::from(Cursor::new(runtime_csv));
    let mut transient_concepts = BTreeSet::new();
    let mut effects = BTreeMap::<Entity, LegacyEffect>::new();
    let mut marked_effects = BTreeSet::new();

    while let Some(artifact) = artifacts.next().await {
        let artifact = artifact.with_context(|| {
            format!(
                "failed to decode legacy runtime metadata from {}",
                legacy_csv.display()
            )
        })?;
        match artifact.the.to_string().as_str() {
            "dialog.concept/transient" => {
                let Value::Entity(marker) = artifact.is else {
                    bail!(
                        "legacy command {} has a non-entity transient marker",
                        artifact.of
                    );
                };
                if marker.as_str() != "db:transient" {
                    bail!(
                        "legacy command {} has unexpected transient marker {marker}",
                        artifact.of
                    );
                }
                transient_concepts.insert(artifact.of);
            }
            "dialog.meta/effect" => {
                let Value::Entity(marker) = artifact.is else {
                    bail!("legacy effect {} has a non-entity marker", artifact.of);
                };
                if marker.as_str() != "db:effect" {
                    bail!(
                        "legacy effect {} has unexpected marker {marker}",
                        artifact.of
                    );
                }
                marked_effects.insert(artifact.of);
            }
            "dialog.effect/source" => {
                let Value::String(source) = artifact.is else {
                    bail!("legacy effect {} has a non-text source", artifact.of);
                };
                effects.entry(artifact.of).or_default().source = Some(source);
            }
            "dialog.effect/polarity" => {
                let Value::String(polarity) = artifact.is else {
                    bail!("legacy effect {} has a non-text polarity", artifact.of);
                };
                effects.entry(artifact.of).or_default().polarity = Some(polarity);
            }
            _ => {}
        }
    }

    let mut rules = BTreeMap::new();
    for effect in marked_effects {
        let stored = effects.remove(&effect).unwrap_or_default();
        let source = stored
            .source
            .with_context(|| format!("legacy effect {effect} has no source"))?;
        let polarity = stored
            .polarity
            .with_context(|| format!("legacy effect {effect} has no polarity"))?;
        let source = repair_runtime_attribute_names(&source);
        let mut rule: dialog_query::InductiveRule =
            serde_json::from_str(&source).with_context(|| {
                format!("legacy effect {effect} could not compile as a native rule")
            })?;
        rule = match polarity.as_str() {
            "assert" => rule,
            "retract" => rule.with_polarity(dialog_query::rule::inductive::Polarity::Retract),
            unknown => bail!("legacy effect {effect} has unknown polarity {unknown:?}"),
        };
        rules.insert(rule.this(), rule);
    }
    let ignored_effects = effects.len();
    Ok(LegacyRuntime {
        transient_concepts,
        rules,
        ignored_effects,
    })
}

const LEGACY_RUNTIME_ATTRIBUTES: &[&str] = &[
    "dialog.concept/transient",
    "dialog.meta/effect",
    "dialog.effect/source",
    "dialog.effect/polarity",
];

const LEGACY_DEDUCTIVE_RULE_ATTRIBUTES: &[&str] =
    &["dialog.meta/rule", "db.rule/source", "db.rule/conclusion"];

/// Copy only runtime metadata into a current-decoder-compatible CSV stream.
///
/// A complete old export can contain value encodings retired by dialog. The
/// application rows do not need decoding here — `migrate_export` carries
/// those byte-for-byte — so asking the current CSV importer to parse all of
/// them adds an unrelated failure mode. This scanner preserves complete CSV
/// records, including multiline effect JSON, and selects only the text and
/// entity-shaped attributes this translation consumes. Legacy deductive rules
/// are rejected here, before the application import can mutate the destination.
fn extract_legacy_runtime<R: BufRead, W: Write>(source: R, mut out: W) -> Result<()> {
    let mut lines = source.lines();
    let header = lines
        .next()
        .transpose()
        .context("failed to read the legacy export")?
        .context("legacy export has no header row")?;
    writeln!(out, "{header}").context("failed to write the runtime header")?;

    let mut inside_quotes = false;
    let mut keeping = false;
    for line in lines {
        let line = line.context("failed to read a legacy runtime row")?;
        let starts_row = !inside_quotes;
        inside_quotes ^= line.matches('"').count() % 2 == 1;
        if starts_row {
            let attribute = split_attribute(&line).map(|(the, _)| the);
            if attribute.is_some_and(|the| LEGACY_DEDUCTIVE_RULE_ATTRIBUTES.contains(&the)) {
                bail!(
                    "legacy deductive rules are not supported by this migration; \
                     export and recreate them before retrying"
                );
            }
            keeping = attribute.is_some_and(|the| LEGACY_RUNTIME_ATTRIBUTES.contains(&the));
        }
        if keeping {
            writeln!(out, "{line}").context("failed to write a legacy runtime row")?;
        }
    }
    out.flush().context("failed to flush legacy runtime rows")?;
    Ok(())
}

fn repair_runtime_attribute_names(source: &str) -> String {
    let Ok(mut source) = serde_json::from_str::<JsonValue>(source) else {
        return source.to_owned();
    };
    repair_runtime_attributes(&mut source);
    serde_json::to_string(&source).unwrap_or_else(|_| source.to_string())
}

fn repair_runtime_attributes(value: &mut JsonValue) {
    match value {
        JsonValue::Array(values) => {
            for value in values {
                repair_runtime_attributes(value);
            }
        }
        JsonValue::Object(fields) => {
            if let Some(JsonValue::String(the)) = fields.get_mut("the")
                && let Some((_, replacement)) = RUNTIME_ATTRIBUTE_RENAMES
                    .iter()
                    .find(|(attribute, _)| *attribute == the)
            {
                *the = (*replacement).to_owned();
            }
            for value in fields.values_mut() {
                repair_runtime_attributes(value);
            }
        }
        _ => {}
    }
}

async fn repair_runtime_attribute_ids(
    destination: &crate::site::TonkSite,
    branch: &str,
) -> Result<usize> {
    let session = destination
        .named_branch(branch)
        .await
        .with_context(|| format!("failed to open migrated branch {branch:?}"))?;
    let mut stale = Vec::new();
    for &(old, new) in RUNTIME_ATTRIBUTE_RENAMES {
        let attributes: Vec<AnonymousAttribute> = session
            .handle()
            .query()
            .select(Query::<AnonymousAttribute> {
                this: Term::var("attribute"),
                id: Term::from(attribute::Id(old.to_owned())),
                r#type: Term::var("type"),
                cardinality: Term::var("cardinality"),
                description: Term::var("description"),
            })
            .perform(&destination.operator)
            .try_vec()
            .await
            .with_context(|| format!("failed to find stale runtime attribute {old:?}"))?;
        stale.extend(
            attributes
                .into_iter()
                .map(|attribute| (attribute.this, old, new)),
        );
    }
    if stale.is_empty() {
        return Ok(0);
    }

    let repaired = stale.len();
    let mut transaction = session.handle().transaction();
    for (entity, old, new) in stale {
        transaction = transaction
            .retract(attribute::Id::of(entity.clone()).is(old.to_owned()))
            .assert(attribute::Id::of(entity).is(new.to_owned()));
    }
    transaction
        .commit()
        .perform(&destination.operator)
        .await
        .with_context(|| format!("failed to repair runtime attributes on branch {branch:?}"))?;
    Ok(repaired)
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

/// Runtime-owned attribute IDs that changed meaning without moving into the
/// `db.*` schema namespace. They appear as values of `db.attribute/id` rows,
/// so renaming only the CSV's first column leaves concepts bound to an
/// attribute the current runtime never injects.
const RUNTIME_ATTRIBUTE_RENAMES: &[(&str, &str)] = &[
    ("dialog.origin/subject", "dialog.replica/subject"),
    ("dialog.origin/profile", "dialog.replica/profile"),
];

fn rename_runtime_attribute_id(the: &str, rest: &str) -> Option<String> {
    if !matches!(the, "dialog.attribute/id" | "db.attribute/id") {
        return None;
    }
    RUNTIME_ATTRIBUTE_RENAMES.iter().find_map(|(old, new)| {
        let old = format!(",text,{old},");
        rest.contains(&old)
            .then(|| rest.replacen(&old, &format!(",text,{new},"), 1))
    })
}

/// Attribute families the new build owns. Importing them is what trips the
/// reserved-namespace refusal. Legacy effects are translated separately into
/// native rules; already-native rule rows belong to the destination runtime.
const DROP_PREFIXES: &[&str] = &[
    // The old rules system, superseded by dialog's native induction.
    "dialog.effect/",
    // Native rules, written by dialog at commit time.
    "dialog.rule/",
];

/// Individual reserved attributes excluded from ordinary CSV import.
/// Transient markers are translated separately into the current boolean form.
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
    /// Rows whose schema or runtime attribute name was remapped.
    pub remapped: usize,
    /// Rows dropped because the new build owns that attribute.
    pub dropped: usize,
}

/// Rewrite a legacy CSV export into one the current build can import.
///
/// Streams rather than buffering: an export of a real space is arbitrarily
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
        let renamed = rename(the);
        let remapped_id = rename_runtime_attribute_id(the, rest);
        if renamed.is_some() || remapped_id.is_some() {
            migration.remapped += 1;
        }
        let output_the = renamed.as_deref().unwrap_or(the);
        let output_rest = remapped_id.as_deref().unwrap_or(rest);
        writeln!(out, "{output_the}{output_rest}").context("failed to write a migrated row")?;
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
    fn it_finds_blob_references_anywhere_in_an_export_once() {
        let first = Entity::from_blob(&[1; 32]).unwrap();
        let second = Entity::from_blob(&[2; 32]).unwrap();
        let third = Entity::from_blob(&[3; 32]).unwrap();
        let csv = format!(
            "the,of,as,is,cause\n\
             app/file,{first},entity,{second},\n\
             app/view,id:page,text,\"<img src='{third}'> {first}\",\n\
             app/note,id:note,text,blob:not-a-content-hash,\n"
        );

        assert_eq!(
            blob_references(csv.as_bytes()).unwrap(),
            vec![first, second, third]
        );
    }

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

    #[test]
    fn it_updates_runtime_attribute_ids_inside_schema_rows() {
        let legacy = "the,of,as,is,cause\n\
            dialog.attribute/id,the:subject,text,dialog.origin/subject,\n\
            dialog.attribute/id,the:profile,text,dialog.origin/profile,\n";
        let mut out = Vec::new();

        migrate_export(legacy.as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains(",text,dialog.replica/subject,"), "{text}");
        assert!(text.contains(",text,dialog.replica/profile,"), "{text}");
        assert!(!text.contains("dialog.origin/"), "{text}");
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

    /// Dialog's own bookkeeping is excluded from the application CSV. The
    /// migration translates the legacy effect source separately; native rule
    /// rows are regenerated from that source rather than copied verbatim.
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

    #[test]
    fn it_extracts_only_legacy_runtime_rows_with_multiline_sources() {
        let legacy = "the,of,as,is,cause\n\
            dialog.concept/transient,concept:A,entity,db:transient,\n\
            app/binary,id:data,bytes,not-valid-current-base58,\n\
            dialog.effect/source,effect:A,text,\"first line\n\
            second line\",\n\
            dialog.effect/polarity,effect:A,text,assert,\n";
        let mut runtime = Vec::new();

        extract_legacy_runtime(legacy.as_bytes(), &mut runtime).unwrap();
        let text = String::from_utf8(runtime).unwrap();

        assert!(text.contains("dialog.concept/transient"));
        assert!(text.contains("first line\nsecond line"));
        assert!(text.contains("dialog.effect/polarity"));
        assert!(!text.contains("app/binary"));
    }

    #[test]
    fn it_repairs_runtime_names_inside_legacy_rule_sources() {
        let source = r#"{
            "the":"dialog.origin/profile",
            "description":"dialog.origin/profile",
            "nested":{"the":"dialog.origin/subject"}
        }"#;
        let repaired = repair_runtime_attribute_names(source);

        assert!(repaired.contains("dialog.replica/profile"));
        assert!(repaired.contains("dialog.replica/subject"));
        assert!(repaired.contains(r#""description":"dialog.origin/profile""#));
    }

    fn legacy_effect_source() -> String {
        serde_json::json!({
            "assert!": {
                "with": {
                    "tag": {
                        "the": "migration.test/pong-tag",
                        "as": "Text",
                        "cardinality": "one",
                        "description": "tag"
                    }
                }
            },
            "when": [{
                "assert": {
                    "with": {
                        "tag": {
                            "the": "migration.test/ping-tag",
                            "as": "Text",
                            "cardinality": "one",
                            "description": "tag"
                        }
                    }
                },
                "where": {
                    "this": { "?": { "name": "this" } },
                    "tag": { "?": { "name": "tag" } }
                }
            }]
        })
        .to_string()
        .replace('"', "\"\"")
    }

    async fn parse_legacy_runtime(csv: &str) -> Result<LegacyRuntime> {
        let workspace = tempfile::tempdir()?;
        let path = workspace.path().join("legacy.csv");
        std::fs::write(&path, csv)?;
        legacy_runtime(&path).await
    }

    #[tokio::test]
    async fn it_uses_the_effect_marker_instead_of_the_entity_prefix() {
        let custom = "did:key:zTESTfixture111111111111111111111111111111111";
        let csv = format!(
            "the,of,as,is,cause\n\
             dialog.meta/effect,{custom},entity,db:effect,\n\
             dialog.effect/source,{custom},text,\"{}\",\n\
             dialog.effect/polarity,{custom},text,assert,\n",
            legacy_effect_source()
        );

        let runtime = parse_legacy_runtime(&csv).await.unwrap();

        assert_eq!(runtime.rules.len(), 1);
        assert_eq!(runtime.ignored_effects, 0);
    }

    #[tokio::test]
    async fn it_rejects_a_marked_malformed_effect() {
        let custom = "did:key:zTESTfixture111111111111111111111111111111111";
        let csv = format!(
            "the,of,as,is,cause\n\
             dialog.meta/effect,{custom},entity,db:effect,\n\
             dialog.effect/source,{custom},text,not-json,\n\
             dialog.effect/polarity,{custom},text,assert,\n"
        );

        let error = match parse_legacy_runtime(&csv).await {
            Ok(_) => panic!("a marked malformed effect must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("could not compile"), "{error:#}");
    }

    #[tokio::test]
    async fn it_ignores_unmarked_effect_debris() {
        let custom = "did:key:zTESTfixture111111111111111111111111111111111";
        let csv = format!(
            "the,of,as,is,cause\n\
             dialog.effect/source,{custom},text,not-json,\n\
             dialog.effect/polarity,{custom},text,assert,\n"
        );

        let runtime = parse_legacy_runtime(&csv).await.unwrap();

        assert!(runtime.rules.is_empty());
        assert_eq!(runtime.ignored_effects, 1);
    }

    #[tokio::test]
    async fn it_rejects_legacy_deductive_rules() {
        let csv = "the,of,as,is,cause\n\
            db.rule/source,rule:legacy,bytes,00,\n";

        let error = match parse_legacy_runtime(csv).await {
            Ok(_) => panic!("a legacy deductive rule must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("deductive rules"), "{error:#}");
    }

    #[tokio::test]
    async fn it_rejects_an_invalid_legacy_transient_marker() {
        let csv = "the,of,as,is,cause\n\
            dialog.concept/transient,concept:legacy,boolean,true,\n";

        let error = match parse_legacy_runtime(csv).await {
            Ok(_) => panic!("an invalid transient marker must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("transient marker"), "{error:#}");
    }
}
