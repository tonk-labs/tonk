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

// `task` (see `CONCEPT_DECL`) has two fields declared under `with:`
// — `title` (Text) and `done` (Boolean) — and neither is declared
// via a `maybe:` block, so `ConceptFieldDescriptor::is_optional()`
// is `false` for both: `add`'s dynamic clap `Command` (built with
// `all_required=true`) makes *both* required args. A `--title`-only
// add therefore fails clap's required-argument check before
// anything is built or committed, so the happy-path test below
// supplies both flags.
mod when_adding_an_instance {
    use super::*;

    #[dialog_common::test]
    async fn it_commits_an_instance_from_typed_flags() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec![
            "--title".to_string(),
            "Write the plan".to_string(),
            "--done".to_string(),
            "false".to_string(),
        ];
        tonk_cli::data_ops::add(&test.site, "task", &argv).await?;
        // Verify it landed: list should now show the title.
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(
            out.contains("Write the plan"),
            "added instance should appear in list:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_enumerating_valid_flags_on_unknown_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec!["--nope".to_string(), "x".to_string()];
        let err = tonk_cli::data_ops::add(&test.site, "task", &argv)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("title"),
            "error should enumerate valid flags:\n{msg}"
        );
        Ok(())
    }
}

// `task` has two required fields (`title`, `done`; see the note
// above `when_adding_an_instance`), so a bare `&anchor` seed with a
// name but no `this:` field trips the analyzer's "no incomplete
// fresh-entity assertion" rule unless every required field is
// supplied up front — every seed below sets both.
mod when_setting_and_removing {
    use super::*;

    #[dialog_common::test]
    async fn it_overwrites_a_field_on_a_named_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // Seed a named task `t1` with an anchor so it is addressable
        // by name across separate eval calls.
        test.eval_inline("task!: &t1\n  title: \"old\"\n  done: false\n")
            .await?;
        tonk_cli::data_ops::set(&test.site, "task", "t1", &["--title".into(), "new".into()])
            .await?;
        let out = tonk_cli::data_ops::get(&test.site, "task", "t1", false).await?;
        assert!(
            out.contains("new") && !out.contains("old"),
            "set should overwrite:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_set_with_no_fields_supplied() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t1b\n  title: \"old\"\n  done: false\n")
            .await?;
        let err = tonk_cli::data_ops::set(&test.site, "task", "t1b", &[])
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("at least one"),
            "empty set should be rejected:\n{msg}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_a_single_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t2\n  title: \"retract-me\"\n  done: false\n")
            .await?;
        tonk_cli::data_ops::rm(&test.site, "task", "t2", Some("title")).await?;
        // After retracting its only declared field, the concept
        // query no longer matches it (a concept query requires
        // every field present).
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(
            !out.contains("retract-me"),
            "retracted field should drop the row from the concept query:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_a_whole_instance() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t3\n  title: \"gone-entirely\"\n  done: false\n")
            .await?;
        tonk_cli::data_ops::rm(&test.site, "task", "t3", None).await?;
        let out = tonk_cli::data_ops::list(&test.site, "task", false).await?;
        assert!(
            !out.contains("gone-entirely"),
            "whole-instance rm should drop the row:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_enumerating_valid_fields_on_an_unknown_rm_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t4\n  title: \"x\"\n  done: false\n")
            .await?;
        let err = tonk_cli::data_ops::rm(&test.site, "task", "t4", Some("nope"))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("title"),
            "error should enumerate valid fields:\n{msg}"
        );
        Ok(())
    }
}
