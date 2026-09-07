//! The connection receipt must resolve through the same seeded model as the UI.
mod common;

#[dialog_common::test]
async fn connection_receipt_is_visible_only_in_the_connected_space() -> anyhow::Result<()> {
    let connected = common::TestSite::new().await?;
    let other = common::TestSite::new().await?;
    let query = "agent-connection:\n  this: id:tonk:agent-connection\n  status: ?status\n";
    let before = connected.eval_inline(query).await?;
    assert!(before.response.matches_after[0].results.is_empty());
    tonk_cli::handoff::record_connection(&connected.site).await?;
    let after = connected.eval_inline(query).await?;
    assert_eq!(after.response.matches_after[0].results.len(), 1);
    assert!(after.stdout.contains("Agent connection confirmed"));
    let route =
        tonk_cli::render::RenderRoute::parse("id:tonk:agent-connection@tonk:agent-connection")?;
    let html = tonk_cli::render::render(&connected.site, &route).await?;
    assert!(html.contains("Agent connection confirmed"));
    assert!(html.contains("sent a reply"));
    let untouched = other.eval_inline(query).await?;
    assert!(untouched.response.matches_after[0].results.is_empty());
    Ok(())
}

#[dialog_common::test]
async fn agent_prompt_is_copyable_without_showing_machine_instructions() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline(
        r#"tonk/agent-invite!:
  this: id:test:prompt
  name: "Test space"
  link: "https://example.test/join?access=proof#secret"
  access: "proof"
  remote: ""
  code: "secret"
"#,
    )
    .await?;
    let route = tonk_cli::render::RenderRoute::parse("id:test:prompt@tonk:agent-invite")?;
    let html = tonk_cli::render::render(&test.site, &route).await?;
    assert!(html.contains("Copy the prompt and give it to an agent of your choice."));
    assert!(html.contains("copy-label=\"Copy prompt\""));
    assert!(html.contains("tonk connect 'https://example.test/join?access=proof#secret'"));
    assert!(
        !html.contains("<pre"),
        "machine instructions should not be visible"
    );
    Ok(())
}

#[dialog_common::test]
async fn confirmation_publishes_the_receipt_to_the_upstream() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    let upstream = test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?;
    test.site
        .branch()
        .await?
        .handle()
        .set_upstream(&upstream)
        .perform(&test.site.operator)
        .await?;
    tonk_cli::sync::push(&test.site).await?;
    tonk_cli::handoff::confirm_connection(&test.site).await?;
    let published = test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?;
    let local = test.site.branch().await?;
    assert_eq!(
        published.revision().unwrap().tree,
        local.handle().revision().unwrap().tree
    );
    let receipt = test
        .eval_inline("agent-connection:\n  this: id:tonk:agent-connection\n  status: ?status\n")
        .await?;
    assert_eq!(receipt.response.matches_after[0].results.len(), 1);
    Ok(())
}

#[dialog_common::test]
async fn failed_pull_does_not_record_a_connection() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    assert!(
        tonk_cli::handoff::confirm_connection(&test.site)
            .await
            .is_err()
    );
    let receipt = test
        .eval_inline("agent-connection:\n  this: id:tonk:agent-connection\n  status: ?status\n")
        .await?;
    assert!(receipt.response.matches_after[0].results.is_empty());
    Ok(())
}

#[dialog_common::test]
async fn connection_uses_the_space_name_and_avoids_local_collisions() -> anyhow::Result<()> {
    use tonk_schema::prelude::DidExt as _;
    let test = common::TestSite::new().await?;
    test.site
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(tonk_schema::RepositoryName {
            this: test.site.repository.did().this(),
            name: tonk_schema::domain::repo::Name("Test Garden".into()),
        })
        .commit()
        .perform(&test.site.operator)
        .await?;
    let mut registry = tonk_cli::space::Registry::default();
    assert_eq!(
        tonk_cli::handoff::synced_name(&test.site, &registry).await?,
        "test-garden"
    );
    registry.spaces.insert(
        "test-garden".into(),
        tonk_cli::space::SpaceEntry::at(test.parent.clone()),
    );
    assert_eq!(
        tonk_cli::handoff::synced_name(&test.site, &registry).await?,
        "test-garden-2"
    );
    let displayed = test
        .eval_inline("tonk/repository:\n  name: ?name\n")
        .await?;
    assert!(
        displayed.stdout.contains("Test Garden"),
        "choosing a local alias must not rename the synced space"
    );
    Ok(())
}
