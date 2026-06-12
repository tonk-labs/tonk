//! `slide export` / `slide import` — CSV round-trip of a branch's
//! artifacts.

mod common;

use anyhow::Result;
use slide::transfer::{self, Destination};

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
    // Seed one site, export it to a file.
    let source = TestSite::new().await?;
    source.eval_inline(ATTRIBUTE_DECL).await?;
    let source_csv = export_to_string(&source).await?;
    let csv_path = source.parent.join("export.csv");

    // Import that file into a fresh, empty site.
    let dest = TestSite::new().await?;
    let _revision = transfer::import(&dest.site, &csv_path).await?;

    // The destination's export now matches the source's, row-for-row.
    let dest_csv = export_to_string(&dest).await?;
    let mut source_rows: Vec<&str> = source_csv.lines().skip(1).collect();
    let mut dest_rows: Vec<&str> = dest_csv.lines().skip(1).collect();
    source_rows.sort_unstable();
    dest_rows.sort_unstable();
    assert_eq!(
        dest_rows, source_rows,
        "round-tripped rows differ\nsource:\n{source_csv}\ndest:\n{dest_csv}",
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
