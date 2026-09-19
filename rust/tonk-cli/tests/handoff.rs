//! The connection receipt must resolve through the same seeded model as the UI.
mod common;

#[dialog_common::test]
async fn pending_handoff_does_not_resolve_as_a_copyable_prompt() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline(
        r#"tonk/repository!:
  this: id:test:pending-handoff
  name: "Untitled"
tonk/agent-handoff-state!:
  this: id:test:pending-handoff
  status: "Create an account to connect an agent."
  link: ""
  account: did:key:device-placeholder
"#,
    )
    .await?;
    let query = "tonk/agent-invite:\n  this: id:test:pending-handoff\n  link: ?link\n";
    let pending = test.eval_inline(query).await?;
    assert!(
        pending.response.matches_after[0].results.is_empty(),
        "a pending response must not bypass the ready rule"
    );
    // Inline notation adds claims; replace the previous response explicitly
    // to model the worker overlay's cardinality-one supersession.
    test.eval_inline("tonk/agent-handoff-state!:\n  this: id:test:pending-handoff\n  ..: _\n")
        .await?;
    test.eval_inline(
        r#"tonk/agent-handoff-state!:
  this: id:test:pending-handoff
  status: "ready"
  link: "https://example.test/join#test"
  account: did:key:expected-account
"#,
    )
    .await?;
    let ready = test.eval_inline(query).await?;
    assert_eq!(ready.response.matches_after[0].results.len(), 1);
    assert!(ready.stdout.contains("https://example.test/join#test"));
    test.eval_inline("tonk/agent-handoff-state!:\n  this: id:test:pending-handoff\n  ..: _\n")
        .await?;
    test.eval_inline(
        r#"tonk/agent-handoff-state!:
  this: id:test:pending-handoff
  status: "Account changed; generate a new handoff."
  link: ""
  account: did:key:device-placeholder
"#,
    )
    .await?;
    let invalidated = test.eval_inline(query).await?;
    assert!(invalidated.response.matches_after[0].results.is_empty());
    Ok(())
}

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
  account: did:key:expected-account
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
        .publish()
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
        .publish()
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
        .publish()
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
        .publish()
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
        .publish()
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

#[dialog_common::test]
async fn connect_rejects_open_invite_before_mutation() -> anyhow::Result<()> {
    let issuer = common::TestSite::new().await?;
    let invite =
        tonk_cli::invite::mint(&issuer.site, Some("https://example.test/join"), None).await?;
    let home = tempfile::tempdir()?;
    let binary = std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into());
    let output = std::process::Command::new(binary)
        .args(["connect", &invite.url, "--no-open"])
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("TONK_SPACES_STATE", home.path().join("spaces"))
        .env("TONK_TELEMETRY_STATE", home.path().join("telemetry"))
        .env("TONK_UPDATE_STATE", home.path().join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_SPACE")
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("account-scoped"));
    assert!(
        !home.path().join("spaces").exists(),
        "legacy open invite must fail before local account/space writes"
    );
    Ok(())
}

#[cfg(feature = "integration-tests")]
#[dialog_common::test]
async fn resume_metadata_requires_the_exact_local_claim_and_account_authority() -> anyhow::Result<()>
{
    use tonk_cli::handoff::{HandoffMetadata, preflight_connect};

    let issuer = common::TestSite::new().await?;
    let recipient = common::AccountFixture::new().await?;
    let invite =
        tonk_cli::invite::mint_targeted(&issuer.site, None, None, recipient.link.issuer().as_str())
            .await?;
    let checked = preflight_connect(&invite.url).await?;
    let root = recipient.tmp.path().join("handoff-replica");
    tonk_cli::invite::claim(&root, &invite.url, recipient.config.clone()).await?;
    let replica = tonk_cli::site::TonkSite::open_with(&root, recipient.config.clone()).await?;
    checked.metadata.validate_replica(&replica).await?;
    checked.metadata.save(&root)?;
    assert_eq!(HandoffMetadata::read(&root)?, checked.metadata);
    let saved = std::fs::read_to_string(root.join("agent-handoff.json"))?;
    assert!(!saved.contains(&invite.url));
    let fields: serde_json::Value = serde_json::from_str(&saved)?;
    assert_eq!(fields.as_object().unwrap().len(), 4);

    let mut wrong_subject = checked.metadata.clone();
    wrong_subject.subject = recipient.profile.did();
    assert!(
        wrong_subject
            .validate_replica(&replica)
            .await
            .unwrap_err()
            .to_string()
            .contains("another repository")
    );
    let mut wrong_account = checked.metadata.clone();
    wrong_account.expected_root = issuer.site.profile.did();
    assert!(
        wrong_account
            .validate_replica(&replica)
            .await
            .unwrap_err()
            .to_string()
            .contains("fresh --name")
    );
    std::fs::write(root.join("claimed-invitation"), "another invitation")?;
    assert!(
        checked
            .metadata
            .validate_replica(&replica)
            .await
            .unwrap_err()
            .to_string()
            .contains("invitation claim")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("agent-handoff.json"))?,
        saved
    );
    Ok(())
}

#[dialog_common::test]
async fn explicit_library_evaluation_refreshes_a_frozen_blank_canvas() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline("view!:\n  this: tonk:blank\n  show:\n    ui: '<p>Frozen old canvas</p>'\n")
        .await?;
    let replica = tonk_schema::Replica::new(test.site.profile.did(), test.site.repository.did());
    test.site
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(replica.clone())
        .commit()
        .publish()
        .perform(&test.site.operator)
        .await?;
    let route = tonk_cli::render::RenderRoute::parse(&format!("{}@tonk:blank", replica.this()))?;
    assert!(
        tonk_cli::render::render(&test.site, &route)
            .await?
            .contains("Frozen old canvas")
    );
    test.eval_inline(include_str!("../../tonk-core/assets/library/core.yaml"))
        .await?;
    let refreshed = tonk_cli::render::render(&test.site, &route).await?;
    assert!(!refreshed.contains("Frozen old canvas"));
    assert!(refreshed.contains("tonk:agent-handoff"), "{refreshed}");
    Ok(())
}

#[cfg(feature = "integration-tests")]
#[dialog_common::test]
async fn self_account_handoff_retains_a_reusable_prefix() -> anyhow::Result<()> {
    let account = common::AccountFixture::new().await?;
    let owned = tonk_cli::site::TonkSite::init_at_with(
        &account.tmp.path().join("owned"),
        account.config.clone(),
    )
    .await?;
    let minted =
        tonk_cli::invite::mint_targeted(&owned, None, None, account.link.issuer().as_str()).await?;
    let checked = tonk_cli::handoff::preflight_connect(&minted.url).await?;
    account
        .profile
        .credential()
        .site(tonk_account::prefix::space_root_site(
            &owned.repository.did(),
            account.link.issuer(),
        ))
        .save(Vec::<u8>::new())
        .perform(&owned.operator)
        .await?;
    let root = account.tmp.path().join("self-handoff");
    tonk_cli::invite::claim(&root, &minted.url, account.config.clone()).await?;
    let replica = tonk_cli::site::TonkSite::open_with(&root, account.config.clone()).await?;
    checked.metadata.validate_replica(&replica).await?;
    Ok(())
}
