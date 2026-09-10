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
            Default::default(),
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
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await?;
        let err = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("already exists"), "{err}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_enumerates_valid_types_on_a_bad_attr() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:string:one".into()],
            None,
            Default::default(),
        )
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
        Default::default(),
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
        tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{name}</b>",
            false,
            Default::default(),
        )
        .await?;
        let out =
            tonk_cli::data_ops::home(&test.site, &["habit".into()], Default::default()).await?;
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
        let err = tonk_cli::data_ops::home(&test.site, &["nope".into()], Default::default())
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
    use tonk_cli::authoring::ViewKind;

    async fn render_home(test: &TestSite) -> Result<String> {
        let replica = tonk_cli::data_ops::query(&test.site, "tonk/replica", false).await?;
        let entity = replica
            .lines()
            .find_map(|line| line.trim().strip_prefix("this: ").map(str::to_owned))
            .expect("a fresh site has a replica entity");
        let route = tonk_cli::render::RenderRoute::parse(&format!("{entity}@tonk/space"))?;
        Ok(tonk_cli::render::render(&test.site, &route).await?)
    }

    #[dialog_common::test]
    async fn it_asserts_the_view_and_auto_surfaces_an_unset_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        let before = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("seed revision")
            .edition
            .value();
        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{name}</b>",
            false,
            Default::default(),
        )
        .await?;
        assert!(
            out.contains("/space/"),
            "auto-surface should print the live path:\n{out}"
        );
        let after = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("view revision")
            .edition
            .value();
        assert_eq!(
            after,
            before + 1,
            "the view and automatic home update should commit together"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn dry_run_reports_identity_without_committing() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        let before = test.site.branch().await?.handle().revision();

        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            ViewKind::Detail,
            "<b>{name}</b>",
            false,
            tonk_cli::data_ops::WriteOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .await?;

        assert!(out.contains("dry run — nothing committed"), "{out}");
        assert!(out.contains("would have asserted the ui view"), "{out}");
        assert_eq!(test.site.branch().await?.handle().revision(), before);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_repoint_an_already_set_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        tonk_cli::data_ops::home(&test.site, &["habit".into()], Default::default()).await?;
        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            tonk_cli::authoring::ViewKind::Detail,
            "<i>{name}</i>",
            false,
            Default::default(),
        )
        .await?;
        assert!(
            !out.contains("home set:"),
            "an explicitly set home must not be re-pointed by view add:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_authors_and_renders_a_directory_view_for_every_row() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        tonk_cli::data_ops::assert_op(&test.site, "habit", None, &["--name".into(), "Walk".into()])
            .await?;

        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            ViewKind::Directory,
            "<li>{name}</li>",
            false,
            Default::default(),
        )
        .await?;
        assert!(out.contains("set the home to habit"), "{out}");

        let route = tonk_cli::render::RenderRoute::parse("habit")?;
        let html = tonk_cli::render::render(&test.site, &route).await?;
        assert!(html.contains("Run"), "{html}");
        assert!(html.contains("Walk"), "{html}");
        assert!(!html.contains("<wa-carousel"), "{html}");
        Ok(())
    }

    #[dialog_common::test]
    async fn label_and_title_views_do_not_auto_surface_a_blank_home() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;

        for kind in [ViewKind::Label, ViewKind::Title] {
            let out = tonk_cli::data_ops::view_add(
                &test.site,
                "habit",
                kind,
                "<b>{name}</b>",
                false,
                Default::default(),
            )
            .await?;
            assert!(out.contains("home unchanged"), "{out}");
            assert!(!out.contains("live at /space/"), "{out}");
        }

        let html = render_home(&test).await?;
        assert!(!html.contains("Run"), "blank home was replaced:\n{html}");
        Ok(())
    }

    #[dialog_common::test]
    async fn explicit_home_replaces_the_existing_home_in_one_revision() -> Result<()> {
        let test = TestSite::new().await?;
        super::seed_habit(&test).await?;
        tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            ViewKind::Detail,
            "<b>{name}</b>",
            false,
            Default::default(),
        )
        .await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "note",
            &["title:text:one".into()],
            Some("a note"),
            Default::default(),
        )
        .await?;
        tonk_cli::data_ops::assert_op(
            &test.site,
            "note",
            None,
            &["--title".into(), "Write".into()],
        )
        .await?;
        let before = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("revision before explicit home")
            .edition
            .value();

        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "note",
            ViewKind::Directory,
            "<li>{title}</li>",
            true,
            Default::default(),
        )
        .await?;
        assert!(out.contains("set the home to note"), "{out}");
        let after = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("revision after explicit home")
            .edition
            .value();
        assert_eq!(after, before + 1);

        let html = render_home(&test).await?;
        assert!(html.contains("Write"), "{html}");
        assert!(!html.contains("Run"), "old home remained active:\n{html}");
        Ok(())
    }
}

/// The reason `element` exists beside the deprecated `component`:
/// identity that never moves. A `component!:` with no `this:` is keyed
/// by its body digest, so editing it writes a SECOND row and the realm
/// loads both. `element` pins `element:<tag>`, so facts accumulate on
/// one entity and a later assertion supersedes only the methods it
/// names.
mod when_defining_an_element {
    use super::*;

    fn methods(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[dialog_common::test]
    async fn it_lists_the_element_under_its_tag_with_its_methods() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            &methods(&[
                ("connected", "(self) => { self.textContent = 'v1'; }"),
                ("disconnected", "(self) => {}"),
            ]),
            Default::default(),
        )
        .await?;
        let listed = tonk_cli::elements::list(&test.site).await?;
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].tag.as_deref(), Some("tally-widget"));
        assert_eq!(listed[0].entity.to_string(), "element:tally-widget");
        assert_eq!(listed[0].methods, vec!["connected", "disconnected"]);
        assert!(!listed[0].deprecated);
        Ok(())
    }

    /// The property the whole `method:` dictionary exists for, and the
    /// one derived identity cannot provide: authoring ONE method
    /// leaves the others standing.
    #[dialog_common::test]
    async fn it_supersedes_only_the_methods_a_later_assertion_names() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            &methods(&[
                ("connected", "(self) => { self.textContent = 'v1'; }"),
                ("disconnected", "(self) => { self.dataset.gone = '1'; }"),
                ("bump", "(self) => 1"),
            ]),
            Default::default(),
        )
        .await?;
        // Re-author `connected` ALONE.
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            &methods(&[("connected", "(self) => { self.textContent = 'v2'; }")]),
            Default::default(),
        )
        .await?;

        let listed = tonk_cli::elements::list(&test.site).await?;
        assert_eq!(listed.len(), 1, "redefinition accrued a row: {listed:?}");
        assert_eq!(
            listed[0].methods,
            vec!["bump", "connected", "disconnected"],
            "authoring one method dropped the others",
        );
        // And the surviving `connected` is the NEW one.
        let described = tonk_cli::data_ops::query(&test.site, "element", false).await?;
        assert!(described.contains("'v2'"), "{described}");
        assert!(
            !described.contains("'v1'"),
            "old method still present:\n{described}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_records_the_accrual_the_deprecated_component_concept_still_has() -> Result<()> {
        let test = TestSite::new().await?;
        // Two `component!:` assertions differing only in module text.
        // Each is keyed by its own body digest, so the branch ends up
        // holding both definitions of the same custom element and the
        // directory view mounts both.
        for body in ["console.log('v1');", "console.log('v2');"] {
            let doc = format!("component!:\n  module: |\n    {body}\n");
            test.eval_inline(&doc).await?;
        }
        let listed = tonk_cli::elements::list(&test.site).await?;
        let components: Vec<_> = listed.iter().filter(|row| row.deprecated).collect();
        assert_eq!(
            components.len(),
            2,
            "expected the digest-keyed concept to accrue both rows: {listed:?}",
        );
        // And the tag is unrecoverable from either — the listing has
        // nothing to show but a digest.
        assert!(components.iter().all(|row| row.tag.is_none()), "{listed:?}");
        Ok(())
    }

    /// The two shapes share nothing — different attributes, different
    /// loaders — so a branch can carry both without either shadowing
    /// the other. Nothing has to be migrated to adopt `element`.
    #[dialog_common::test]
    async fn it_carries_component_and_element_rows_side_by_side() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(
            "component!:\n  module: |\n    customElements.define('old-widget', class extends HTMLElement {});\n",
        )
        .await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "new-widget",
            &methods(&[("connected", "(self) => { self.textContent = 'new'; }")]),
            Default::default(),
        )
        .await?;

        let listed = tonk_cli::elements::list(&test.site).await?;
        assert_eq!(listed.len(), 2, "{listed:?}");
        let new = listed
            .iter()
            .find(|row| !row.deprecated)
            .expect("element row present");
        assert_eq!(new.tag.as_deref(), Some("new-widget"));
        assert_eq!(new.methods, vec!["connected"]);
        assert!(
            listed.iter().any(|row| row.deprecated),
            "component row disappeared: {listed:?}",
        );

        // Each concept's own query sees ONLY its own rows: the
        // component's module is not visible as an element, and the
        // element's methods are not visible as a component.
        let elements = tonk_cli::data_ops::query(&test.site, "element", false).await?;
        assert!(elements.contains("new-widget"), "{elements}");
        assert!(!elements.contains("old-widget"), "{elements}");
        let components = tonk_cli::data_ops::query(&test.site, "component", false).await?;
        assert!(components.contains("old-widget"), "{components}");
        assert!(!components.contains("new-widget"), "{components}");
        Ok(())
    }

    /// The pin is what lets the `element` declaration grow later
    /// without stranding rows: an unpinned concept is keyed by its own
    /// declaration digest, so an edit would mint a different entity
    /// and every existing row would stop matching.
    #[dialog_common::test]
    async fn it_pins_both_concepts_so_a_later_declaration_keeps_its_rows() -> Result<()> {
        let test = TestSite::new().await?;
        for (name, pinned) in [("element", "tonk:element"), ("component", "tonk:component")] {
            let entity = tonk_cli::views::entity_for_name(&test.site, name)
                .await?
                .unwrap_or_else(|| panic!("{name} should resolve"));
            assert_eq!(entity.to_string(), pinned, "{name} is not pinned");
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_tag_no_browser_would_register() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::element_add(
            &test.site,
            "widget",
            &methods(&[("connected", "(self) => {}")]),
            Default::default(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("hyphen"), "{err}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_method_that_would_shadow_an_html_element_member() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            &methods(&[("remove", "(self) => {}")]),
            Default::default(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("shadow"), "{err}");
        Ok(())
    }
}
