//! Behavioural tests for the preview projection: the conclusions
//! handed to the harness must be the same data `<tonk-display>`
//! would receive from its entity subscription.

mod common;

mod when_projecting_an_entity_for_preview {
    use anyhow::Result;
    use ipld_core::ipld::Ipld;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    #[dialog_common::test]
    async fn it_projects_real_branch_fields_for_a_named_subject() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t1\n  title: \"Buy milk\"\n  done: false\n")
            .await?;

        let projection = slide::preview::project::project_entity(
            &test.site.branch,
            &test.site.operator,
            "task",
            "t1",
        )
        .await?;

        assert_eq!(
            projection.conclusions.len(),
            1,
            "one row for a pinned entity"
        );
        let fields = &projection.conclusions[0].fields;
        assert_eq!(
            fields.get("title"),
            Some(&Ipld::String("Buy milk".into())),
            "projected fields carry the asserted value, got {fields:?}",
        );
        assert!(
            projection.descriptor_fields.contains(&"title".to_string()),
            "descriptor fields enumerate the model's schema, got {:?}",
            projection.descriptor_fields,
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_on_an_unknown_model_name() -> Result<()> {
        let test = common::TestSite::new().await?;
        let result = slide::preview::project::project_entity(
            &test.site.branch,
            &test.site.operator,
            "no-such-concept",
            "t1",
        )
        .await;
        assert!(
            result.is_err(),
            "unknown model must error, not render blank"
        );
        Ok(())
    }
}
