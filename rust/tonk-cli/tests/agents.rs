//! Behavioural tests for claim-backed space agent context.

mod common;

use anyhow::Result;
use tonk_cli::agents;

use crate::common::TestSite;

#[dialog_common::test]
async fn it_maps_markdown_to_the_repository_subject() -> Result<()> {
    let test = TestSite::new().await?;
    let expected = "# Space context\n\n- Use `task` for launch work.\n";

    let stored = agents::set(&test.site, expected, false).await?;

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
    let first = agents::set(&test.site, "# First\n", false).await?;
    let second = agents::set(&test.site, "# Second\n", false).await?;

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
    assert!(agents::set(&test.site, " \n", false).await.is_err());
    let oversized = "x".repeat(agents::MAX_MARKDOWN_BYTES + 1);
    assert!(agents::set(&test.site, &oversized, false).await.is_err());
    Ok(())
}
