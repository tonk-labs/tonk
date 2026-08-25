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
async fn it_plans_an_add_without_writing_anything() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("notes.md");
    tokio::fs::write(&file, b"# heading").await?;

    let plan = blob::plan(&file, None).await?;
    assert_eq!(plan.content_type, "text/markdown");
    assert_eq!(plan.size, 9);
    assert_eq!(plan.name, "notes.md");

    // Nothing reached the branch: the metadata facts a real add asserts
    // are what `blob ls` reads, so an empty listing is the proof.
    assert!(blob::ls(&test.site).await?.is_empty());
    Ok(())
}

#[dialog_common::test]
async fn it_plans_with_the_content_type_override_it_would_assert() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("data.bin");
    tokio::fs::write(&file, b"arbitrary bytes").await?;

    let plan = blob::plan(&file, Some("application/x-custom".to_string())).await?;
    assert_eq!(plan.content_type, "application/x-custom");
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_to_plan_an_unreadable_file() -> Result<()> {
    let test = TestSite::new().await?;
    // A preview that reported success for a file it could never read
    // would be a preview of something that cannot happen.
    let missing = test.parent.join("absent.png");
    assert!(blob::plan(&missing, None).await.is_err());
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

#[dialog_common::test]
async fn it_attaches_blob_bytes_without_asserting_metadata() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("legacy.bin");
    tokio::fs::write(&file, b"legacy blob bytes").await?;

    let attached = blob::attach(&test.site, "main", &file).await;
    assert!(attached.is_ok(), "raw attachment failed: {attached:?}");
    let attached = attached.unwrap();

    let mut out = Vec::new();
    blob::cat(&test.site, attached.entity.as_str(), &mut out).await?;
    assert_eq!(out, b"legacy blob bytes");

    let export = test.parent.join("after-attach.csv");
    tonk_cli::transfer::export(
        &test.site,
        tonk_cli::transfer::Destination::File(export.clone()),
    )
    .await?;
    let csv = tokio::fs::read_to_string(export).await?;
    assert!(
        !csv.contains(attached.entity.as_str()),
        "raw attachment must not invent metadata facts: {csv}"
    );
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

/// `ls` reads the metadata `add` asserted, so a freshly added blob is
/// listed with the content type and file name it was ingested under.
#[tokio::test]
async fn it_lists_an_added_blob_with_its_metadata() -> Result<()> {
    let test = TestSite::new().await?;
    let file = test.parent.join("pic.png");
    tokio::fs::write(&file, b"\x89PNG bytes").await?;
    let added = blob::add(&test.site, &file, None).await?;

    let rows = blob::ls(&test.site).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entity, added.entity);
    assert_eq!(rows[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(rows[0].name.as_deref(), Some("pic.png"));
    Ok(())
}

/// Re-adding the same bytes is idempotent all the way through the
/// listing: one entity, one row.
#[tokio::test]
async fn it_lists_one_row_per_distinct_blob() -> Result<()> {
    let test = TestSite::new().await?;
    let first = test.parent.join("a.png");
    let second = test.parent.join("b.txt");
    tokio::fs::write(&first, b"\x89PNG bytes").await?;
    tokio::fs::write(&second, b"plain").await?;
    blob::add(&test.site, &first, None).await?;
    blob::add(&test.site, &first, None).await?;
    blob::add(&test.site, &second, None).await?;

    let rows = blob::ls(&test.site).await?;
    assert_eq!(rows.len(), 2);
    let mut types: Vec<_> = rows.iter().filter_map(|r| r.content_type.clone()).collect();
    types.sort();
    assert_eq!(types, vec!["image/png", "text/plain"]);
    Ok(())
}

/// A branch that has ingested nothing lists nothing — an empty
/// listing, not an error.
#[tokio::test]
async fn it_lists_nothing_on_a_branch_with_no_blobs() -> Result<()> {
    let test = TestSite::new().await?;
    assert!(blob::ls(&test.site).await?.is_empty());
    Ok(())
}
