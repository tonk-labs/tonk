//! Behavioural tests for the live workflow card.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};
use tonk_cli::space::{Resolved, Source};
use tonk_cli::{agents, context};

/// The sync and account sections the CLI passes in.
///
/// `inspect` takes them rather than gathering them, so these tests can
/// pin the schema-derived half of the report without an account or a
/// network round trip.
fn offline_sections() -> (context::SyncContext, context::AccountContext) {
    (
        context::SyncContext::offline(false, None),
        context::AccountContext {
            signed_in: false,
            account: None,
            account_service: None,
            device: Some("did:device".to_string()),
            state: None,
        },
    )
}

#[dialog_common::test]
async fn it_maps_live_schema_to_a_read_update_verify_workflow() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    let resolved = Resolved {
        name: "bench".to_string(),
        site: test.site.root.clone(),
        source: Source::Env,
    };

    let report = {
        let (sync, account) = offline_sections();
        context::inspect(&resolved, &test.site, sync, account).await?
    };
    let markdown = report.render_markdown();

    assert!(
        markdown.contains("Update an existing `task` safely"),
        "{markdown}"
    );
    assert!(markdown.contains("`tonk query task --json`"), "{markdown}");
    assert!(
        markdown.contains("`tonk assert task <ENTITY> --done true`"),
        "{markdown}"
    );
    assert!(
        markdown.contains("`tonk query task <ENTITY> --json`"),
        "{markdown}"
    );
    assert!(markdown.contains("`title` text/one/required"), "{markdown}");
    assert!(
        markdown.contains("`done` boolean/one/required"),
        "{markdown}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_emits_a_versioned_complete_contract() -> Result<()> {
    let test = TestSite::new().await?;
    let resolved = Resolved {
        name: "empty".to_string(),
        site: test.site.root.clone(),
        source: Source::Flag,
    };

    let report = {
        let (sync, account) = offline_sections();
        context::inspect(&resolved, &test.site, sync, account).await?
    };
    let value: serde_json::Value = serde_json::from_str(&report.render_json()?)?;

    assert_eq!(value["schemaVersion"], "tonk.context.v3");
    assert_eq!(value["space"]["name"], "empty");
    assert_eq!(value["space"]["selectedVia"], "flag");
    assert_eq!(value["space"]["branch"], "main");
    assert_eq!(value["space"]["cwdSelectsSpace"], false);
    assert!(value["agents"].is_null());
    assert_eq!(value["concepts"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        value["emptySpaceWorkflow"][0]["command"],
        "tonk concept add note --attr title:text:one --attr body:text:one"
    );

    // v3 is camelCase throughout: a v1 reader must miss, not silently
    // half-match on the keys that happen to be one word.
    for retired in ["schema_version", "empty_space_workflow"] {
        assert!(value.get(retired).is_none(), "{retired} survived the bump");
    }
    Ok(())
}

#[dialog_common::test]
async fn it_exposes_claim_backed_agent_context_with_source_revision() -> Result<()> {
    let test = TestSite::new().await?;
    let expected = "# Space context\n\n1. Run `tonk query task --json`.\n";
    let claim = agents::set(&test.site, expected, Default::default())
        .await?
        .expect("committed");
    let resolved = Resolved {
        name: "bench".to_string(),
        site: test.site.root.clone(),
        source: Source::Env,
    };

    let report = {
        let (sync, account) = offline_sections();
        context::inspect(&resolved, &test.site, sync, account).await?
    };
    let markdown = report.render_markdown();
    let value: serde_json::Value = serde_json::from_str(&report.render_json()?)?;

    assert!(markdown.contains("## Space AGENTS.md"), "{markdown}");
    assert!(markdown.contains(&claim.entity), "{markdown}");
    assert!(markdown.contains(&claim.revision), "{markdown}");
    assert!(markdown.contains(expected), "{markdown}");
    assert_eq!(value["agents"]["source"], agents::SOURCE);
    assert_eq!(value["agents"]["attribute"], agents::ATTRIBUTE);
    assert_eq!(value["agents"]["entity"], claim.entity);
    assert_eq!(value["agents"]["revision"], claim.revision);
    assert_eq!(value["agents"]["markdown"], expected);
    Ok(())
}

mod when_reporting_where_i_am {
    use super::*;

    /// Four commands answered "where am I" in four layouts, naming the
    /// same field three ways: `space:` in `tonk status`, `current space:`
    /// in `tonk space use`, and ``space: `demo` `` in `tonk context`.
    /// They render the same sections now, so one vocabulary covers all.
    #[dialog_common::test]
    fn it_renders_one_field_vocabulary_across_the_sections() {
        let space = context::SpaceContext {
            name: "demo".to_string(),
            site: "/spaces/demo".to_string(),
            selected_via: "env".to_string(),
            branch: "main",
            cwd_selects_space: false,
        };
        assert_eq!(
            space.render(),
            "space: demo\nsite: /spaces/demo\nselected via: env\nbranch: main\n"
        );
    }

    #[dialog_common::test]
    fn it_says_when_the_sync_state_was_not_fetched() {
        // `tonk context` is what bare `tonk` runs and does not touch the
        // network, so it can see that an upstream exists but not where the
        // branch stands against it. Reporting `synced` there would be a
        // claim it has not checked.
        let sync = context::SyncContext {
            state: context::ContextSyncState::NotFetched,
            hash: None,
            fetched: false,
        };
        assert_eq!(
            sync.render(),
            "sync: upstream configured, not checked (run `tonk status`)\n"
        );

        let fetched = context::SyncContext {
            state: context::ContextSyncState::Synced,
            hash: Some("#abc".to_string()),
            fetched: true,
        };
        assert_eq!(fetched.render(), "sync: synced\nhash: #abc\n");
    }

    #[dialog_common::test]
    async fn it_carries_the_sync_and_account_sections_into_the_card() -> Result<()> {
        let test = TestSite::new().await?;
        let resolved = Resolved {
            name: "bench".to_string(),
            site: test.site.root.clone(),
            source: Source::Env,
        };
        let (sync, account) = offline_sections();
        let report = context::inspect(&resolved, &test.site, sync, account).await?;
        let markdown = report.render_markdown();

        // The card is the union of the three sections the other commands
        // each print one of.
        assert!(markdown.contains("space: bench\n"), "{markdown}");
        assert!(markdown.contains("selected via: env\n"), "{markdown}");
        assert!(markdown.contains("sync: no-upstream"), "{markdown}");
        assert!(markdown.contains("signed in: no\n"), "{markdown}");
        assert!(markdown.contains("device: did:device\n"), "{markdown}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_versions_the_json_contract_as_v3() -> Result<()> {
        let test = TestSite::new().await?;
        let resolved = Resolved {
            name: "bench".to_string(),
            site: test.site.root.clone(),
            source: Source::Env,
        };
        let (sync, account) = offline_sections();
        let report = context::inspect(&resolved, &test.site, sync, account).await?;
        let json: serde_json::Value = serde_json::from_str(&report.render_json()?)?;

        // Absorbing the two sections is breaking for a reader that pinned
        // v2, so the v3 version moves with them.
        assert_eq!(json["schemaVersion"], "tonk.context.v3");
        assert_eq!(json["sync"]["fetched"], false);
        assert_eq!(json["account"]["signedIn"], false);
        Ok(())
    }
}
