use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};
use tonk_cli::log::{self, LogError};

const TITLE: &str = "xyz.tonk.task/title";
const DONE: &str = "xyz.tonk.task/done";

/// A site with the `task` concept declared and one task, `t`, whose
/// title was then changed from "old" to "new".
async fn site_with_an_edited_task() -> Result<TestSite> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    test.eval_inline("task!: &t\n  title: \"old\"\n  done: false\n")
        .await?;
    tonk_cli::data_ops::assert_op(
        &test.site,
        "task",
        Some("t"),
        &["--title".into(), "new".into()],
    )
    .await?;
    Ok(test)
}

mod when_listing_revisions {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_the_newest_revision_first_with_its_parent() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let rows = log::revisions(&test.site, 100).await?;
        assert!(
            rows.len() >= 4,
            "two declarations, a create and an edit are four commits:\n{rows:#?}"
        );
        for pair in rows.windows(2) {
            assert!(
                pair[0].edition > pair[1].edition,
                "newest first, every revision before its ancestors:\n{rows:#?}"
            );
            assert_eq!(
                pair[0].parents,
                vec![pair[1].version.clone()],
                "a linear history: each revision's one parent is the next row"
            );
        }
        let first = rows.last().expect("at least one revision");
        assert!(first.parents.is_empty(), "the first revision has no parent");
        assert!(rows[0].issuer.starts_with("did:key:"));
        assert!(rows[0].authority.starts_with("did:key:"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lists_at_most_the_limit() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let rows = log::revisions(&test.site, 2).await?;
        assert_eq!(rows.len(), 2);
        Ok(())
    }
}

mod when_listing_the_changes_of_a_concept {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_every_claim_on_the_concepts_fields_newest_first() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let filter = log::filter_for(&test.site, Some("task"), None, None).await?;
        let rows = log::changes(&test.site, &filter, 100).await?;

        assert!(
            rows.iter()
                .all(|row| row.attribute == TITLE || row.attribute == DONE),
            "only the concept's fields:\n{rows:#?}"
        );
        for pair in rows.windows(2) {
            assert!(
                pair[0].edition >= pair[1].edition,
                "newest first:\n{rows:#?}"
            );
        }
        let titles: Vec<&str> = rows
            .iter()
            .filter(|row| row.attribute == TITLE)
            .map(|row| row.value_text.as_str())
            .collect();
        assert_eq!(titles, vec!["\"new\"", "\"old\""]);
        let edit = &rows[0];
        assert_eq!(edit.change, "assert");
        assert_eq!(edit.name.as_deref(), Some("t"));
        assert_eq!(edit.value, serde_json::json!("new"));
        assert!(
            !edit.supersedes.is_empty(),
            "the edit cites the claim it replaced:\n{edit:#?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_narrows_to_one_entity() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        test.eval_inline("task!: &other\n  title: \"other\"\n  done: true\n")
            .await?;
        let filter = log::filter_for(&test.site, Some("task"), Some("other"), None).await?;
        let rows = log::changes(&test.site, &filter, 100).await?;
        assert_eq!(rows.len(), 2, "title and done, once each:\n{rows:#?}");
        assert!(rows.iter().all(|row| row.name.as_deref() == Some("other")));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_shows_what_one_revision_recorded() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let all = log::filter_for(&test.site, Some("task"), None, None).await?;
        let newest = log::changes(&test.site, &all, 1).await?;
        let version = newest[0].version.clone();

        let one = log::filter_for(&test.site, None, None, Some(&version)).await?;
        let rows = log::changes(&test.site, &one, 100).await?;
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.version == version));
        assert!(
            rows.iter()
                .any(|row| row.attribute == TITLE && row.value_text == "\"new\""),
            "the edit's revision recorded the new title:\n{rows:#?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_counts_the_limit_in_revisions() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let filter = log::filter_for(&test.site, Some("task"), None, None).await?;
        let rows = log::changes(&test.site, &filter, 1).await?;
        let versions: std::collections::BTreeSet<&str> =
            rows.iter().map(|row| row.version.as_str()).collect();
        assert_eq!(versions.len(), 1, "one revision's rows only:\n{rows:#?}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_concept_that_is_not_on_the_branch() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let error = log::filter_for(&test.site, Some("nope"), None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, LogError::Concept(_)), "{error}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_an_entity_it_cannot_resolve() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let error = log::filter_for(&test.site, Some("task"), Some("nobody"), None)
            .await
            .unwrap_err();
        assert!(matches!(error, LogError::UnknownEntity(_)), "{error}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_revision_in_another_form() -> Result<()> {
        let test = site_with_an_edited_task().await?;
        let error = log::filter_for(&test.site, None, None, Some("#abc"))
            .await
            .unwrap_err();
        assert!(matches!(error, LogError::InvalidVersion(_)), "{error}");
        Ok(())
    }
}
