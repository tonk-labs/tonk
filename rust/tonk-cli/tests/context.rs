//! Behavioural tests for the live workflow card.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};
use tonk_cli::space::{Resolved, Source};
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

    assert_eq!(value["schemaVersion"], "tonk.context.v2");
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

    // v2 is camelCase throughout: a v1 reader must miss, not silently
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
    let claim = agents::set(&test.site, expected, false).await?;
    let resolved = Resolved {
        name: "bench".to_string(),
        site: test.site.root.clone(),
        source: Source::Env,
    };

    let report = context::inspect(&resolved, &test.site).await?;
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
