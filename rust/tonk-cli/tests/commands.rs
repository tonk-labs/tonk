//! Revision-pinned command inventory behavior.

mod common;

use anyhow::Result;
use tonk_cli::commands::inventory;

use crate::common::TestSite;

#[dialog_common::test]
async fn inventory_lists_nominal_projection_and_effect_source() -> Result<()> {
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
    title: { control: title }
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

    let inventory = inventory(&test.site).await?;
    assert_ne!(inventory.revision, "unborn");
    let command = inventory
        .nominal
        .iter()
        .find(|command| command.kind == "id:todo/add")
        .expect("todo/add command");
    assert_eq!(command.projections.len(), 1);
    assert_eq!(command.projections[0].entity, "id:todo/add-form");
    let effect = inventory
        .effects
        .iter()
        .find(|effect| effect.command == "id:todo/add")
        .expect("todo/add rule index");
    assert_eq!(effect.effect, "id:todo/add-rule");
    assert!(!effect.source.is_empty());
    Ok(())
}
