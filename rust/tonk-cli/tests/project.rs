//! Headless nominal projection behavior.

mod common;

use anyhow::Result;
use dialog_artifacts::Value;
use tonk_cli::project::{FixtureInput, run};
use tonk_schema::claim::SourceClaim;

use crate::common::TestSite;

const DECLARATIONS: &str = r#"
command!: &todo/add
  with:
    title: { description: "Title", the: xyz.tonk.todo/title, as: Text }
  maybe:
    done: { description: "Done", the: xyz.tonk.todo/done, as: Boolean }
projection!: &todo/add-form
  command: todo/add
  default: true
  arguments:
    title: { control: "note-body" }
    done: { control: { name: "is-done", property: checked } }
  actions:
    - prevent-default
"#;

#[dialog_common::test]
async fn projection_preserves_blank_and_omits_missing_optional_without_mutating() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(DECLARATIONS).await?;
    let before = test.site.branch().await?.handle().revision();
    let fixture: FixtureInput = serde_yaml::from_str(
        r#"
controls:
  note-body: { value: "" }
"#,
    )?;
    let report = run(&test.site, "todo/add", &fixture, false).await?;
    let after = test.site.branch().await?.handle().revision();
    assert_eq!(before, after, "default projection must remain read-only");
    assert_eq!(report.omitted, vec!["done"]);
    let SourceClaim::Invoke(invocation) = &report.request.claims[0] else {
        panic!("projection must produce invoke");
    };
    assert_eq!(
        invocation.arguments.get("title"),
        Some(&Value::String(String::new()))
    );
    assert!(!invocation.arguments.contains_key("done"));
    Ok(())
}

#[dialog_common::test]
async fn redaction_keeps_field_and_source_names() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(DECLARATIONS).await?;
    let fixture: FixtureInput = serde_yaml::from_str(
        r#"
controls:
  note-body: { value: "Secret" }
  is-done: { checked: false }
"#,
    )?;
    let report = run(&test.site, "todo/add-form", &fixture, false)
        .await?
        .redact();
    let json = serde_json::to_value(report)?;
    assert_eq!(json["trace"][0]["field"], "title");
    assert_eq!(json["trace"][0]["source"]["control"]["name"], "note-body");
    assert_eq!(json["trace"][0]["value"], "<redacted>");
    assert_eq!(
        json["request"]["claims"][0]["arguments"]["title"],
        "<redacted>"
    );
    Ok(())
}

#[dialog_common::test]
async fn missing_required_source_is_classified() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(DECLARATIONS).await?;
    let error = run(&test.site, "todo/add-form", &FixtureInput::default(), false)
        .await
        .expect_err("required source should fail");
    assert!(
        error
            .to_string()
            .contains("missing required field \"title\"")
    );
    Ok(())
}

#[dialog_common::test]
async fn explicit_transact_runs_the_registered_declarative_rule() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(
        r#"
command!: &todo/add
  with:
    title: { description: "Title", the: xyz.tonk.todo/title, as: Text }
projection!: &todo/add-form
  command: todo/add
  default: true
  arguments:
    title: { control: "note-body" }
concept!: &todo/item
  with:
    title: { description: "Title", the: xyz.tonk.todo/title, as: Text }
rule!:
  this: id:todo/add-rule
  assert!: todo/item
  when:
    - assert: todo/add
      where: { this: ?this, title: ?title }
"#,
    )
    .await?;
    let fixture: FixtureInput = serde_yaml::from_str(
        r#"
controls:
  note-body: { value: "Persist me" }
"#,
    )?;

    let report = run(&test.site, "todo/add-form", &fixture, true).await?;
    assert!(report.revision_after.is_some());
    let queried = test
        .eval_inline("todo/item:\n  this: ?todo\n  title: ?title\n")
        .await?;
    let rows = &queried.response.matches_after[0].results;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["title"], serde_json::json!("Persist me"));
    Ok(())
}
