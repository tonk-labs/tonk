//! `tonk blob` — ingest, read back, list.

mod common;

use anyhow::Result;
use tonk_cli::blob;

use crate::common::TestSite;

#[dialog_common::test]
async fn it_adds_a_blob_and_prints_its_reference() -> Result<()> {
    let test = TestSite::new().await?;
    let png = test.parent.join("pixel.png");
    // Not a real PNG; content is irrelevant to addressing.
    tokio::fs::write(&png, b"\x89PNG fake pixel data").await?;

    let outcome = blob::add(&test.site, &png, None).await?;
    assert!(outcome.entity.as_str().starts_with("blob:"));
    assert_eq!(outcome.content_type, "image/png");
    assert_eq!(outcome.size, 20);

    // Same content → same reference (content-addressed, idempotent).
    let again = blob::add(&test.site, &png, None).await?;
    assert_eq!(again.entity, outcome.entity);
    Ok(())
}

#[dialog_common::test]
async fn it_honors_an_explicit_content_type_override() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("data.bin");
    tokio::fs::write(&file, b"arbitrary bytes").await?;

    let outcome = blob::add(&test.site, &file, Some("application/octet-stream".to_string())).await?;
    assert_eq!(outcome.content_type, "application/octet-stream");
    Ok(())
}
