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

mod when_the_concept_is_unknown {
    use super::*;

    #[dialog_common::test]
    async fn it_errors_with_a_known_concepts_list() -> anyhow::Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let err = tonk_cli::data_ops::describe(&test.site, "widget")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("task"),
            "unknown-concept error should list known concepts: {msg}"
        );
        Ok(())
    }
}

mod when_reading_instances {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_all_instances_of_a_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            "task!:\n  title: \"alpha\"\n  done: false\ntask!:\n  title: \"beta\"\n  done: false\n",
        )
        .await?;
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(
            out.contains("alpha") && out.contains("beta"),
            "list should show both:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_emits_json_when_requested() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!:\n  title: \"gamma\"\n  done: false\n")
            .await?;
        let out = tonk_cli::data_ops::list(&test.site, "task", true).await?;
        assert!(
            out.trim_start().starts_with('{') || out.trim_start().starts_with('['),
            "json output:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_gets_a_single_instance_by_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let outcome = test
            .eval_inline("task!: &t\n  title: \"delta\"\n  done: false\n")
            .await?;
        let entity = outcome
            .response
            .commits
            .entities
            .get("t")
            .expect("entity 't' should be bound in the commit summary")
            .clone();

        let out = tonk_cli::data_ops::get(&test.site, "task", &entity, false).await?;
        assert!(
            out.contains("delta"),
            "get should show the fetched instance:\n{out}"
        );
        Ok(())
    }
}
