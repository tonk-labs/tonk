//! `tonk export` / `tonk import` — CSV round-trip of a branch's
//! artifacts.

mod common;

use anyhow::Result;
use tonk_cli::transfer::{self, Destination};

use crate::common::{ATTRIBUTE_DECL, TestSite};

/// Read a site's CSV export back as a String via a temp file.
async fn export_to_string(test: &TestSite) -> Result<String> {
    let path = test.parent.join("export.csv");
    transfer::export(&test.site, Destination::File(path.clone())).await?;
    Ok(tokio::fs::read_to_string(&path).await?)
}

#[tokio::test]
async fn it_exports_seeded_artifacts_as_csv() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;

    let csv = export_to_string(&test).await?;
    assert!(
        csv.starts_with("the,of,as,is,cause"),
        "export missing CSV header: {csv}",
    );
    assert!(
        csv.lines().count() > 1,
        "export should have at least one data row: {csv}",
    );
    Ok(())
}

#[tokio::test]
async fn it_round_trips_export_then_import() -> Result<()> {
    // Seed a site with extra attributes on top of the standard-library
    // baseline site init provides, and export the whole branch.
    let source = TestSite::new().await?;
    source.eval_inline(ATTRIBUTE_DECL).await?;
    let source_csv = export_to_string(&source).await?;
    let csv_path = source.parent.join("export.csv");

    // Import that full export into another fresh site and re-export.
    let dest = TestSite::new().await?;
    transfer::import(&dest.site, &csv_path).await?;
    let dest_csv = export_to_string(&dest).await?;

    // The `ATTRIBUTE_DECL` facts (single-line CSV rows, no embedded
    // newlines) must survive the round-trip identically. We assert on
    // those specific rows rather than diffing the whole export, which
    // also carries the large multi-line standard-library `display`
    // values that a naive line split would mangle.
    let attribute_rows = |csv: &str| -> Vec<String> {
        let mut rows: Vec<String> = csv
            .lines()
            .filter(|r| r.contains("xyz.tonk.task/"))
            .map(str::to_owned)
            .collect();
        rows.sort_unstable();
        rows
    };
    let source_rows = attribute_rows(&source_csv);
    assert!(
        !source_rows.is_empty(),
        "source export should contain the seeded task attributes"
    );
    assert_eq!(
        attribute_rows(&dest_csv),
        source_rows,
        "the seeded attributes must round-trip identically",
    );
    Ok(())
}

#[tokio::test]
async fn it_imports_an_empty_csv_without_error() -> Result<()> {
    let test = TestSite::new().await?;
    let path = test.parent.join("empty.csv");
    tokio::fs::write(&path, "the,of,as,is,cause\n").await?;

    // An empty import succeeds; it just commits nothing meaningful.
    let _ = transfer::import(&test.site, &path).await?;
    Ok(())
}

#[tokio::test]
async fn it_exports_to_stdout_destination() -> Result<()> {
    // Smoke-test the stdout path: export should report a non-zero
    // byte count for a seeded branch even when writing to stdout.
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    let bytes = transfer::export(&test.site, Destination::Stdout).await?;
    assert!(bytes > 0, "stdout export wrote nothing");
    Ok(())
}
