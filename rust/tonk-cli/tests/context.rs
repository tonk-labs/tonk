//! Behavioural tests for the live workflow card.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};
use tonk_cli::spot::{Resolved, Source};
use tonk_cli::{agents, context};

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

    let report = context::inspect(&resolved, &test.site).await?;
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

    let report = context::inspect(&resolved, &test.site).await?;
    let value: serde_json::Value = serde_json::from_str(&report.render_json()?)?;

    assert_eq!(value["schema_version"], "tonk.context.v1");
    assert_eq!(value["spot"]["name"], "empty");
    assert_eq!(value["spot"]["selected_via"], "flag");
    assert_eq!(value["spot"]["branch"], "main");
    assert_eq!(value["spot"]["cwd_selects_spot"], false);
    assert!(value["agents"].is_null());
    assert_eq!(value["concepts"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        value["empty_spot_workflow"][0]["command"],
        "tonk concept add note --attr title:text:one --attr body:text:one"
    );
    assert_eq!(
        value["interactivity_workflow"][0]["command"],
        "tonk guide events"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_routes_to_the_interactivity_loop_on_every_spot() -> Result<()> {
    // An agent that needs a button has no other signal that `project`
    // and `commands` exist, so the lane renders whether or not the spot
    // has any application concepts yet.
    let test = TestSite::new().await?;
    let resolved = Resolved {
        name: "empty".to_string(),
        site: test.site.root.clone(),
        source: Source::Flag,
    };

    let empty = context::inspect(&resolved, &test.site)
        .await?
        .render_markdown();
    assert!(
        empty.contains("## Make a concept respond to DOM events") && empty.contains("tonk project"),
        "{empty}"
    );

    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    let populated = context::inspect(&resolved, &test.site)
        .await?
        .render_markdown();
    assert!(
        populated.contains("## Make a concept respond to DOM events")
            && populated.contains("tonk project"),
        "{populated}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_surfaces_a_view_without_a_separate_home_step() -> Result<()> {
    // `tonk view add` auto-surfaces onto an unset home. A trailing
    // `tonk home note` step would be a no-op that reads as required.
    let test = TestSite::new().await?;
    let resolved = Resolved {
        name: "empty".to_string(),
        site: test.site.root.clone(),
        source: Source::Flag,
    };

    let report = context::inspect(&resolved, &test.site).await?;
    let commands: Vec<&str> = report
        .empty_spot_workflow
        .iter()
        .map(|step| step.command.as_str())
        .collect();

    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("tonk home")),
        "{commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with("tonk view add note")),
        "{commands:?}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_stays_deserializable_when_the_contract_grows_a_field() -> Result<()> {
    // The version-change rule on `SCHEMA_VERSION` promises additive
    // growth keeps `tonk.context.v1`, which only holds if consumers
    // tolerate fields they do not know. Pin the tolerant half here so
    // the promise is enforced rather than assumed.
    #[derive(serde::Deserialize)]
    struct Consumer {
        schema_version: String,
        spot: ConsumerSpot,
    }

    #[derive(serde::Deserialize)]
    struct ConsumerSpot {
        name: String,
    }

    let test = TestSite::new().await?;
    let resolved = Resolved {
        name: "empty".to_string(),
        site: test.site.root.clone(),
        source: Source::Flag,
    };

    let report = context::inspect(&resolved, &test.site).await?;
    let mut value: serde_json::Value = serde_json::from_str(&report.render_json()?)?;
    value["a_field_from_a_later_version"] = serde_json::json!({ "nested": [1, 2, 3] });
    value["spot"]["another_unknown_field"] = serde_json::json!("surprise");

    let consumer: Consumer = serde_json::from_value(value)?;
    assert_eq!(consumer.schema_version, context::SCHEMA_VERSION);
    assert_eq!(consumer.spot.name, "empty");
    Ok(())
}

#[dialog_common::test]
async fn it_exposes_claim_backed_agent_context_with_source_revision() -> Result<()> {
    let test = TestSite::new().await?;
    let expected = "# Spot context\n\n1. Run `tonk query task --json`.\n";
    let claim = agents::set(&test.site, expected, false).await?;
    let resolved = Resolved {
        name: "bench".to_string(),
        site: test.site.root.clone(),
        source: Source::Env,
    };

    let report = context::inspect(&resolved, &test.site).await?;
    let markdown = report.render_markdown();
    let value: serde_json::Value = serde_json::from_str(&report.render_json()?)?;

    assert!(markdown.contains("## Spot AGENTS.md"), "{markdown}");
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
