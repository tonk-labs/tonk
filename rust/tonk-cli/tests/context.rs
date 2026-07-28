//! Behavioural tests for the live workflow card.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};
use tonk_cli::context;
use tonk_cli::spot::{Resolved, Source};

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
    assert_eq!(value["concepts"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        value["empty_spot_workflow"][0]["command"],
        "tonk concept add note --attr title:text:one --attr body:text:one"
    );
    Ok(())
}
