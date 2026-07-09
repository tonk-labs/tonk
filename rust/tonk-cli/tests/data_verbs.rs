mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};

// The verbs are exercised through the library handlers, not the binary,
// to avoid spawning a subprocess. Each handler returns its rendered
// stdout + an ExitCode. (Task 3 introduces `tonk_cli::data_ops`.)
mod when_describing_a_concept {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_fields_with_types() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?; // seeds task-title / task-done
        test.eval_inline(CONCEPT_DECL).await?; // seeds the `task` concept
        let out = tonk_cli::data_ops::describe(&test.site, "task").await?;
        assert!(
            out.contains("title"),
            "describe should list the title field:\n{out}"
        );
        assert!(
            out.contains("Text"),
            "describe should show the field type:\n{out}"
        );
        Ok(())
    }
}
