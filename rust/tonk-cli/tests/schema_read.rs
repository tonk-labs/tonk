//! Behavioural tests for the concept schema-read API
//! (`tonk_cli::schema::find_concept`): looking up a single named
//! concept's fields, types, and cardinalities off the branch.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};

mod when_reading_a_concepts_schema {
    use super::*;

    #[dialog_common::test]
    async fn it_returns_fields_types_and_cardinality_for_a_named_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?; // seeds task-title / task-done
        test.eval_inline(CONCEPT_DECL).await?; // seeds the `task` concept
        let info = tonk_cli::schema::find_concept(&test.site, "task")
            .await?
            .expect("task concept should be found");
        assert_eq!(info.name, "task");
        let fields: Vec<&str> = info.descriptor.with().iter().map(|(f, _)| f).collect();
        assert!(
            fields.contains(&"title"),
            "task should have a title field, got {fields:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_none_for_an_unknown_concept() -> Result<()> {
        let test = TestSite::new().await?;
        assert!(
            tonk_cli::schema::find_concept(&test.site, "nope")
                .await?
                .is_none()
        );
        Ok(())
    }
}

#[dialog_common::test]
async fn nominal_command_and_projection_schema_round_trip() -> Result<()> {
    let source = TestSite::new().await?;
    source
        .eval_inline(
            r#"
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
  actions: [prevent-default]
"#,
        )
        .await?;
    let exported = tonk_cli::schema::render(&source.site).await?;
    assert!(exported.contains("command!: &todo/add"));
    assert!(exported.contains("projection!: &todo/add-form"));

    let destination = TestSite::new().await?;
    destination.eval_inline(&exported).await?;
    let before = tonk_cli::commands::inventory(&source.site).await?;
    let after = tonk_cli::commands::inventory(&destination.site).await?;
    let before = before
        .nominal
        .iter()
        .find(|command| command.kind == "id:todo/add")
        .expect("source command");
    let after = after
        .nominal
        .iter()
        .find(|command| command.kind == "id:todo/add")
        .expect("round-tripped command");
    assert_eq!(before.source, after.source);
    assert_eq!(before.projections[0].source, after.projections[0].source);
    Ok(())
}
