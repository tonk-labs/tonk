//! Behavioural tests for claim-backed space agent context.

mod common;

use anyhow::Result;
use tonk_cli::agents;

use crate::common::TestSite;

#[dialog_common::test]
async fn it_maps_markdown_to_the_repository_subject() -> Result<()> {
    let test = TestSite::new().await?;
    let expected = "# Space context\n\n- Use `task` for launch work.\n";

    let stored = agents::set(&test.site, expected, Default::default())
        .await?
        .expect("committed");

    assert_eq!(stored.source, agents::SOURCE);
    assert_eq!(stored.attribute, agents::ATTRIBUTE);
    assert_eq!(stored.entity, test.site.repository.did().to_string());
    assert_eq!(stored.markdown, expected);
    assert_eq!(agents::get(&test.site).await?, Some(stored));
    Ok(())
}

#[dialog_common::test]
async fn it_supersedes_the_document_on_the_same_space_entity() -> Result<()> {
    let test = TestSite::new().await?;
    let first = agents::set(&test.site, "# First\n", Default::default())
        .await?
        .expect("committed");
    let second = agents::set(&test.site, "# Second\n", Default::default())
        .await?
        .expect("committed");

    assert_eq!(first.entity, second.entity);
    assert_ne!(first.revision, second.revision);
    assert_eq!(
        agents::get(&test.site).await?.map(|claim| claim.markdown),
        Some("# Second\n".to_string())
    );
    Ok(())
}

#[dialog_common::test]
async fn it_rejects_empty_and_unbounded_documents() -> Result<()> {
    let test = TestSite::new().await?;
    assert!(
        agents::set(&test.site, " \n", Default::default())
            .await
            .is_err()
    );
    let oversized = "x".repeat(agents::MAX_MARKDOWN_BYTES + 1);
    assert!(
        agents::set(&test.site, &oversized, Default::default())
            .await
            .is_err()
    );
    Ok(())
}

/// A dry run analyzes the document and drops it, so nothing is written and
/// there is no claim to read back.
#[dialog_common::test]
async fn it_writes_nothing_on_a_dry_run() -> Result<()> {
    let test = TestSite::new().await?;
    let preview = agents::set(
        &test.site,
        "# Not committed\n",
        tonk_cli::data_ops::WriteOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .await?;

    assert!(preview.is_none());
    assert_eq!(agents::get(&test.site).await?, None);
    Ok(())
}
