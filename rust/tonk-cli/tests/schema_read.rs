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
