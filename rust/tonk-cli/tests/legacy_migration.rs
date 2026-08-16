//! Migrating a spot written by the pre-dialog-upgrade build.
//!
//! The fixture is a real spot *directory* created by the `v0.6.7` binary
//! (dialog `rev = e8bbe462`), not something this build produced — reasoning
//! about format compatibility from current code cannot show what an older
//! build actually wrote. See `tests/fixtures/README.md`.

mod common;

use std::io::BufReader;

use anyhow::Result;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_browser);

/// A legacy export migrates into a spot the current build can query, and the
/// data comes back pointing at the entity it always did.
///
/// The identity half is the point. A migrated spot that answers with the same
/// values under *new* entities is a copy, not an upgrade: its peers would
/// treat it as a different spot. So this asserts the exact DID the old build
/// minted, not merely that a row with the right title exists.
#[dialog_common::test]
async fn it_migrates_a_legacy_export_preserving_identity() -> Result<()> {
    // Exported from the fixture spot by the old binary. Committed as CSV
    // beside the database because this build cannot read that database —
    // opening its branch fails with `missing field \`branch\``, which is the
    // whole reason the migration exists.
    let legacy = include_str!("fixtures/legacy-v0.6.7.csv");

    let mut migrated = Vec::new();
    let report =
        tonk_cli::legacy::migrate_export(BufReader::new(legacy.as_bytes()), &mut migrated)?;

    // The schema moved namespace wholesale, and almost nothing is dropped:
    // what the new build owns is a rounding error beside what it inherits.
    assert!(
        report.remapped > report.dropped * 10,
        "the rename carries the schema and the drop list is dialog's own \
         bookkeeping; remapped {} dropped {}",
        report.remapped,
        report.dropped
    );
    // Nothing reserved may survive, or the import refuses the transaction.
    let text = String::from_utf8_lossy(&migrated);
    for reserved in ["dialog.effect/", "dialog.rule/", "dialog.concept/transient"] {
        assert!(
            !text.contains(reserved),
            "{reserved} is reserved or retired and must not survive migration"
        );
    }

    let site = common::TestSite::new().await?;
    let path = site.tmp.path().join("migrated.csv");
    std::fs::write(&path, &migrated)?;
    tonk_cli::transfer::import(&site.site, &path).await?;

    // The assertion that matters: the note the OLD build wrote, queried by
    // name, answering with the same value on the same entity.
    let rows = site
        .eval_inline("note:\n  this: ?this\n  title: ?title\n")
        .await?
        .stdout;
    assert!(
        rows.contains("written by the old build"),
        "the migrated spot must still answer the legacy note query; saw:\n{rows}"
    );
    assert!(
        rows.contains("did:key:z6Mk3VY17HUDh9rW6UpiDdtF9BGmdqfsYC2ZGzk4rAJadk2H"),
        "the note must keep the entity the old build minted, or the migrated \
         spot is a copy rather than an upgrade; saw:\n{rows}"
    );
    Ok(())
}
