//! Behavioural tests for branch management: the checkout, creating and
//! deleting branches, and what merging does and does not change.

mod common;

use anyhow::Result;
use tonk_cli::branch::{self, BranchError};

use crate::common::{NOTE_ATTRIBUTE_DECL, NOTE_CONCEPT_DECL, TestSite};

/// Assert one note on whichever branch `site` is checked out on.
async fn write_note(site: &tonk_cli::site::TonkSite, body: &str) -> Result<()> {
    tonk_cli::eval::run_against_site(
        site,
        tonk_cli::eval::Source::Inline(format!(
            "{NOTE_ATTRIBUTE_DECL}\n{NOTE_CONCEPT_DECL}\nnote!:\n  body: \"{body}\"\n  ..: _\n"
        )),
        tonk_cli::eval::Options::default(),
    )
    .await?;
    Ok(())
}

/// How many notes the checked-out branch can see.
async fn note_count(site: &tonk_cli::site::TonkSite) -> Result<usize> {
    use dialog_query::{Output as _, Term};

    let session = site.branch().await?;
    let rows: Vec<dialog_query::Claim> = session
        .handle()
        .query()
        .select(dialog_query::AttributeQuery::new(
            Term::from(dialog_query::attribute::The::from(
                "xyz.tonk.note/body"
                    .parse::<dialog_artifacts::Attribute>()
                    .map_err(|error| anyhow::anyhow!("{error:?}"))?,
            )),
            Term::<dialog_artifacts::Entity>::var("of"),
            Term::<dialog_query::Any>::var("is"),
            Term::<dialog_query::attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("note query failed: {error:?}"))?;
    Ok(rows.len())
}

fn named<'a>(rows: &'a [branch::BranchRecord], name: &str) -> &'a branch::BranchRecord {
    rows.iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("'{name}' should be listed; got {rows:?}"))
}

mod when_listing_branches {
    use super::*;

    #[dialog_common::test]
    async fn it_reports_main_as_the_checkout_of_a_fresh_space() -> Result<()> {
        let test = TestSite::new().await?;
        let rows = branch::list(&test.site).await?;

        assert_eq!(test.site.head(), "main");
        assert!(named(&rows, "main").current);
        // A fresh space is seeded on `main`, so it has a head to show.
        assert!(named(&rows, "main").head.is_some());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lists_a_created_branch_beside_the_one_it_came_from() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let rows = branch::list(&test.site).await?;
        assert_eq!(named(&rows, "draft").head, named(&rows, "main").head);
        assert!(!named(&rows, "draft").current, "creating is not switching");
        Ok(())
    }
}

mod when_creating_a_branch {
    use super::*;

    #[dialog_common::test]
    async fn it_starts_at_the_checkouts_head() -> Result<()> {
        let test = TestSite::new().await?;
        let outcome = branch::create(&test.site, "draft", None, false).await?;

        assert_eq!(outcome.start, "main");
        let head = test.site.branch().await?.handle().revision();
        assert_eq!(outcome.head, head.map(|revision| revision.tree));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_can_check_the_space_out_onto_the_new_branch() -> Result<()> {
        let test = TestSite::new().await?;
        let outcome = branch::create(&test.site, "draft", None, true).await?;

        assert!(outcome.switched);
        // The checkout is resolved when a site opens, so the *next*
        // invocation is the one that lands on it — which is what a
        // second `tonk` process does.
        assert_eq!(test.reopen().await?.head(), "draft");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_name_already_in_use() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let error = branch::create(&test.site, "draft", None, false)
            .await
            .expect_err("a second create should refuse");
        assert!(matches!(error, BranchError::Exists(name) if name == "draft"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_start_point_that_is_not_a_branch() -> Result<()> {
        let test = TestSite::new().await?;
        // A tree hash is not a start point: a head is signed by the
        // session that minted it, so there is none to mint here.
        let error = branch::create(&test.site, "draft", Some("#nope"), false)
            .await
            .expect_err("an unknown start point should refuse");
        assert!(
            matches!(
                &error,
                BranchError::InvalidName { .. } | BranchError::Unknown(_)
            ),
            "{error}"
        );
        Ok(())
    }
}

mod when_working_on_a_branch {
    use super::*;

    #[dialog_common::test]
    async fn it_keeps_writes_off_the_branch_they_were_not_made_on() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, true).await?;

        let draft = test.reopen().await?;
        assert_eq!(draft.head(), "draft");
        write_note(&draft, "only on draft").await?;
        assert_eq!(note_count(&draft).await?, 1);

        // `main` is untouched: two branches share no facts until one is
        // merged into the other.
        assert_eq!(note_count(&test.site).await?, 0);
        Ok(())
    }

    #[dialog_common::test]
    async fn the_branch_flag_addresses_another_branch_without_switching() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let mut config = test.config.clone();
        config.branch = Some("draft".to_owned());
        let pinned = tonk_cli::site::TonkSite::open_with(&test.site.root, config).await?;
        write_note(&pinned, "written through the flag").await?;

        assert_eq!(pinned.head(), "draft");
        // The flag was for one invocation; the recorded checkout never
        // moved, so the next ordinary open is still on `main`.
        assert_eq!(test.reopen().await?.head(), "main");
        assert_eq!(note_count(&test.site).await?, 0);
        Ok(())
    }
}

mod when_merging {
    use super::*;

    #[dialog_common::test]
    async fn it_brings_the_other_branchs_facts_into_the_checkout() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, true).await?;
        write_note(&test.reopen().await?, "from draft").await?;

        // Back on `main`, which has never seen the note.
        let main = test.reopen_on("main").await?;
        assert_eq!(note_count(&main).await?, 0);

        let outcome = branch::merge(&main, "draft").await?;
        assert!(outcome.advanced);
        assert_eq!(note_count(&main).await?, 1);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_a_branch_with_nothing_new_as_already_merged() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let outcome = branch::merge(&test.site, "draft").await?;
        assert!(!outcome.advanced, "a copy of main has nothing to give");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_leaves_an_untracked_branch_still_tracking_nothing() -> Result<()> {
        // Dialog's pull records its source as a tracked upstream so a
        // repeat merge is incremental. On a branch tracking nothing, that
        // entry would become the default — and `tonk push` would then aim
        // at a local branch nobody asked it to push to.
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;
        branch::merge(&test.site, "draft").await?;

        let after = test.reopen().await?;
        assert_eq!(
            after.branch().await?.handle().upstream(),
            None,
            "merging must not wire an upstream"
        );
        assert!(
            named(&branch::list(&after).await?, "main")
                .upstream
                .is_none()
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_merge_a_branch_into_itself() -> Result<()> {
        let test = TestSite::new().await?;
        let error = branch::merge(&test.site, "main")
            .await
            .expect_err("merging the checkout into itself should refuse");
        assert!(matches!(error, BranchError::Itself(name) if name == "main"));
        Ok(())
    }
}

mod when_deleting_a_branch {
    use super::*;

    #[dialog_common::test]
    async fn it_drops_the_branch_and_stops_listing_it() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let outcome = branch::delete(&test.site, "draft").await?;
        assert!(outcome.head.is_some());
        assert!(!branch::exists(&test.site, "draft").await?);

        let rows = branch::list(&test.reopen().await?).await?;
        assert!(
            !rows.iter().any(|row| row.name == "draft"),
            "a deleted branch should not be listed; got {rows:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_the_content_branch_and_the_checkout() -> Result<()> {
        let test = TestSite::new().await?;
        let error = branch::delete(&test.site, "main")
            .await
            .expect_err("the content branch should be refused");
        assert!(matches!(error, BranchError::ContentBranch(_)), "{error}");

        branch::create(&test.site, "draft", None, true).await?;
        let draft = test.reopen().await?;
        let error = branch::delete(&draft, "draft")
            .await
            .expect_err("the checkout should be refused");
        assert!(matches!(error, BranchError::CheckedOut(_)), "{error}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_tonks_own_meta_branch() -> Result<()> {
        let test = TestSite::new().await?;
        let error = branch::delete(&test.site, "meta")
            .await
            .expect_err("meta should be refused");
        assert!(matches!(error, BranchError::Reserved(_)), "{error}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_leaves_no_branch_tracking_the_one_that_is_gone() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;
        branch::set_upstream(&test.site, Some("main"), "draft").await?;
        assert_eq!(
            named(&branch::list(&test.reopen().await?).await?, "main").upstream,
            Some("draft".to_owned()),
        );

        let after_wiring = test.reopen().await?;
        branch::delete(&after_wiring, "draft").await?;

        // A branch tracking one that no longer exists fails deeper, in
        // dialog's vocabulary rather than tonk's — so the delete sweeps
        // the entry rather than leaving it dangling.
        let rows = branch::list(&test.reopen().await?).await?;
        assert!(named(&rows, "main").upstream.is_none(), "{rows:?}");
        Ok(())
    }
}

mod when_switching {
    use super::*;

    #[dialog_common::test]
    async fn it_records_a_checkout_the_next_open_picks_up() -> Result<()> {
        let test = TestSite::new().await?;
        branch::create(&test.site, "draft", None, false).await?;

        let outcome = branch::switch(&test.site, "draft", false).await?;
        assert_eq!(outcome.previous, "main");
        assert!(!outcome.created);
        assert_eq!(test.reopen().await?.head(), "draft");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_creates_the_branch_first_when_asked() -> Result<()> {
        let test = TestSite::new().await?;
        let outcome = branch::switch(&test.site, "draft", true).await?;

        assert!(outcome.created);
        assert_eq!(test.reopen().await?.head(), "draft");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_branch_that_does_not_exist() -> Result<()> {
        let test = TestSite::new().await?;
        let error = branch::switch(&test.site, "draft", false)
            .await
            .expect_err("switching to a missing branch should refuse");
        assert!(matches!(error, BranchError::Unknown(name) if name == "draft"));
        Ok(())
    }
}
