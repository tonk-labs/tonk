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

    let outcome = blob::add(
        &test.site,
        &file,
        Some("application/octet-stream".to_string()),
    )
    .await?;
    assert_eq!(outcome.content_type, "application/octet-stream");
    Ok(())
}

#[tokio::test]
async fn it_cats_a_blob_back() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("note.txt");
    tokio::fs::write(&file, b"hello blob").await?;
    let added = blob::add(&test.site, &file, None).await?;

    let mut out = Vec::new();
    let written = blob::cat(&test.site, added.entity.as_str(), &mut out).await?;
    assert_eq!(written, 10);
    assert_eq!(out, b"hello blob");

    // A well-formed but unknown reference is a clean error.
    let missing = "blob:11111111111111111111111111111111"; // 32 one-bytes in base58
    assert!(matches!(
        blob::cat(&test.site, missing, &mut Vec::new()).await,
        Err(blob::BlobError::NotFound(_))
    ));
    // A non-blob URI is rejected up front.
    assert!(
        blob::cat(&test.site, "id:alice", &mut Vec::new())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn it_lists_blobs_with_metadata() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("pic.png");
    tokio::fs::write(&file, b"\x89PNG bytes").await?;
    let added = blob::add(&test.site, &file, None).await?;

    let rows = blob::ls(&test.site).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entity, added.entity);
    assert_eq!(rows[0].size, 10);
    assert_eq!(rows[0].content_type.as_deref(), Some("image/png"));
    Ok(())
}
