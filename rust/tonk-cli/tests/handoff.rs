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
    assert!(html.contains("Your agent connected"));
    assert!(html.contains("Dismiss agent connection notification"));
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
    assert!(
        html.contains(
            "npx --yes @tonk/cli connect 'https://example.test/join?access=proof#secret'"
        )
    );
    assert!(
        !html.contains("<pre"),
        "machine instructions should not be visible"
    );
    Ok(())
}

#[dialog_common::test]
async fn account_approval_ignores_all_invite_routing_metadata() -> anyhow::Result<()> {
    use tonk_schema::prelude::DidExt as _;

    let test = common::TestSite::new().await?;
    let invite = tonk_cli::invite::mint(
        &test.site,
        Some("https://untrusted.example/join"),
        Some("https://provider.example/ucan/"),
    )
    .await?;
    let checked = tonk_cli::invite::preflight(&invite.url).await?;

    assert_eq!(
        checked.invitation.subject.0,
        test.site.repository.did().this()
    );
    assert_eq!(
        tonk_cli::handoff::approval_page(None)?,
        tonk_cli::account::DEFAULT_LINK_PAGE
    );
    Ok(())
}

#[dialog_common::test]
async fn an_explicit_approval_page_is_a_validated_user_override() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    let invite =
        tonk_cli::invite::mint(&test.site, Some("https://untrusted.example/join"), None).await?;
    tonk_cli::invite::preflight(&invite.url).await?;

    assert_eq!(
        tonk_cli::handoff::approval_page(Some("http://127.0.0.1:8080/settings/link"))?,
        "http://127.0.0.1:8080/settings/link"
    );
    Ok(())
}

#[dialog_common::test]
async fn retry_matches_only_the_exact_invitation_and_skips_broken_entries() -> anyhow::Result<()> {
    let inviter = common::TestSite::new().await?;
    let first = tonk_cli::invite::mint(&inviter.site, None, None).await?;
    let fresh = tonk_cli::invite::mint(&inviter.site, None, None).await?;
    let claimed = tempfile::tempdir()?;
    let parent = claimed.path().canonicalize()?;
    let root = parent.join("joined-site");
    let config = common::isolated_config(&parent)?;
    tonk_cli::invite::claim(&root, &first.url, config.clone()).await?;

    let mut registry = tonk_cli::space::Registry::default();
    registry.spaces.insert(
        "a-broken".into(),
        tonk_cli::space::SpaceEntry::at(parent.join("missing-site")),
    );
    registry.spaces.insert(
        "z-existing".into(),
        tonk_cli::space::SpaceEntry::at(root.clone()),
    );

    let first = tonk_cli::invite::preflight(&first.url).await?;
    let spoof_root = parent.join("spoof-site");
    let spoof = tonk_cli::site::TonkSite::init_at_with(&spoof_root, config.clone()).await?;
    spoof
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(first.invitation.clone())
        .commit()
        .perform(&spoof.operator)
        .await?;
    registry.spaces.insert(
        "b-spoof".into(),
        tonk_cli::space::SpaceEntry::at(spoof_root),
    );
    // The retained local claim must suffice even without its replicated row.
    let older = tonk_cli::site::TonkSite::open_with(&root, config.clone()).await?;
    older
        .branch()
        .await?
        .handle()
        .transaction()
        .retract(first.invitation.clone())
        .commit()
        .perform(&older.operator)
        .await?;
    let matched =
        tonk_cli::handoff::matching_invitation(&registry, &config, &first.invitation).await;
    assert_eq!(matched.name, Some("z-existing".into()));
    assert_eq!(matched.diagnostics.len(), 1, "{:#?}", matched.diagnostics);
    assert!(matched.diagnostics[0].contains("a-broken"));

    let fresh = tonk_cli::invite::preflight(&fresh.url).await?;
    // Model a pull of another member's claim into the older replica.
    older
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(fresh.invitation.clone())
        .commit()
        .perform(&older.operator)
        .await?;
    let unmatched =
        tonk_cli::handoff::matching_invitation(&registry, &config, &fresh.invitation).await;
    assert_eq!(unmatched.name, None);
    assert_eq!(
        unmatched.diagnostics.len(),
        1,
        "{:#?}",
        unmatched.diagnostics
    );
    // Pre-marker replicas cannot establish a local claim from roster data.
    std::fs::remove_file(root.join("claimed-invitation"))?;
    older
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(first.invitation.clone())
        .commit()
        .perform(&older.operator)
        .await?;
    let legacy =
        tonk_cli::handoff::matching_invitation(&registry, &config, &first.invitation).await;
    assert_eq!(legacy.name, None);
    assert_eq!(legacy.diagnostics.len(), 1);
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
