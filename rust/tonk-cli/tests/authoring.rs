mod common;

use anyhow::Result;

use crate::common::TestSite;

mod when_adding_a_concept {
    use super::*;

    #[dialog_common::test]
    async fn it_authors_an_anchored_concept_usable_by_the_data_verbs() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into(), "target:text:one".into()],
            Some("a tracked habit"),
        )
        .await?;
        // The anchored concept is immediately usable end to end:
        // schema-aware assert, then query sees the instance.
        tonk_cli::data_ops::assert_op(
            &test.site,
            "habit",
            None,
            &[
                "--name".into(),
                "Run".into(),
                "--target".into(),
                "5k".into(),
            ],
        )
        .await?;
        let out = tonk_cli::data_ops::query(&test.site, "habit", false).await?;
        assert!(out.contains("Run"), "authored concept round-trips:\n{out}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_an_existing_concept_name() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(&test.site, "habit", &["name:text:one".into()], None)
            .await?;
        let err =
            tonk_cli::data_ops::concept_add(&test.site, "habit", &["name:text:one".into()], None)
                .await
                .unwrap_err();
        assert!(format!("{err}").contains("already exists"), "{err}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_enumerates_valid_types_on_a_bad_attr() -> Result<()> {
        let test = TestSite::new().await?;
        let err =
            tonk_cli::data_ops::concept_add(&test.site, "habit", &["name:string:one".into()], None)
                .await
                .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("UnsignedInteger") && msg.contains("Text"),
            "{msg}"
        );
        Ok(())
    }
}

async fn seed_habit(test: &TestSite) -> Result<()> {
    tonk_cli::data_ops::concept_add(
        &test.site,
        "habit",
        &["name:text:one".into()],
        Some("a habit"),
    )
    .await?;
    tonk_cli::data_ops::assert_op(&test.site, "habit", None, &["--name".into(), "Run".into()])
        .await?;
    Ok(())
}

mod when_setting_the_home {
    use super::*;

    #[dialog_common::test]
    async fn it_repoints_the_space_alias_and_renders_the_data() -> Result<()> {
        let test = TestSite::new().await?;
        seed_habit(&test).await?;
        // The verified recipe (repoint-findings recipe 3) always pairs
        // the data concept with a view — the headless renderer has no
        // default item view (`no view found for model` without one).
        tonk_cli::data_ops::view_add(&test.site, "habit", None, "<b>{name}</b>").await?;
        let out = tonk_cli::data_ops::home(&test.site, &["habit".into()]).await?;
        assert!(
            out.contains("/space/"),
            "home should print the live path:\n{out}"
        );
        // End to end through the same resolution pipeline the browser
        // runs: the replica entity rendered at model tonk/space must
        // now show the habit data (repoint-findings recipe 3).
        let replica = tonk_cli::data_ops::query(&test.site, "tonk/replica", false).await?;
        let entity = replica
            .lines()
            .find_map(|l| l.trim().strip_prefix("this: ").map(str::to_owned))
            .expect("a fresh site has a replica entity");
        let route = tonk_cli::render::RenderRoute::parse(&format!("{entity}@tonk/space"))?;
        let html = tonk_cli::render::render(&test.site, &route).await?;
        assert!(
            html.contains("Run"),
            "the space home must render the habit directory:\n{html}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_on_an_unknown_model() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::home(&test.site, &["nope".into()])
            .await
            .unwrap_err();
        assert!(
            format!("{err}").contains("no concept named 'nope'"),
            "{err}"
        );
        Ok(())
    }
}

mod when_adding_a_view {
    use super::*;

    #[dialog_common::test]
    async fn it_asserts_the_view_and_auto_surfaces_an_unset_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        let out = tonk_cli::data_ops::view_add(&test.site, "habit", None, "<b>{name}</b>").await?;
        assert!(
            out.contains("/space/"),
            "auto-surface should print the live path:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_repoint_an_already_set_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        tonk_cli::data_ops::home(&test.site, &["habit".into()]).await?;
        let out =
            tonk_cli::data_ops::view_add(&test.site, "habit", Some("habit-alt"), "<i>{name}</i>")
                .await?;
        assert!(
            !out.contains("home set:"),
            "an explicitly set home must not be re-pointed by view add:\n{out}"
        );
        Ok(())
    }
}
