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

/// The reason `element` exists beside the deprecated `component`: a
/// tag you can repoint. A `component!:` with no `this:` is keyed by
/// its body digest and nothing names it, so editing it writes a SECOND
/// row and the realm loads both. An `element!: &<tag>` is keyed by its
/// body too, but the anchor publishes `id:<tag>` over the result — so
/// an edit mints a new value AND moves the tag onto it, and everything
/// resolving by name follows.
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
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[
                    ("connected", "(self) => { self.textContent = 'v1'; }"),
                    ("disconnected", "(self) => {}"),
                ]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        // The library seeds elements of its own; the one this test
        // authored is what it asserts on.
        let listed = tonk_cli::elements::list(&test.site).await?;
        let ours: Vec<_> = listed
            .iter()
            .filter(|row| row.tag.as_deref() == Some("tally-widget"))
            .collect();
        assert_eq!(ours.len(), 1, "{listed:?}");
        assert_eq!(ours[0].methods, vec!["connected", "disconnected"]);
        assert!(!ours[0].deprecated);
        Ok(())
    }

    /// Authoring one method through the CLI leaves the others
    /// standing — and, because the entity is derived from the whole
    /// body, does so by minting a NEW element and repointing the tag,
    /// not by patching the old one.
    ///
    /// Both halves are load-bearing and each fails differently. If the
    /// entity did not move, methods would not be in the digest and two
    /// elements could collide. If the methods were not carried
    /// forward, the new value would have only `connected` and the tag
    /// would resolve to a crippled element.
    #[dialog_common::test]
    async fn it_carries_the_other_methods_onto_the_new_definition() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[
                    ("connected", "(self) => { self.textContent = 'v1'; }"),
                    ("disconnected", "(self) => { self.dataset.gone = '1'; }"),
                    ("bump", "(self) => 1"),
                ]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let before = tonk_cli::views::entity_for_name(&test.site, "tally-widget").await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => { self.textContent = 'v2'; }")]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let after = tonk_cli::views::entity_for_name(&test.site, "tally-widget").await?;
        assert_ne!(
            before, after,
            "a different set of methods is a different element, so the \
             tag should have been repointed at a new entity",
        );

        let now = tonk_cli::elements::methods_of(&test.site, "tally-widget").await?;
        let keys: Vec<&str> = now.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["bump", "connected", "disconnected"]);
        assert!(
            now.iter()
                .any(|(k, v)| k == "connected" && v.contains("v2"))
        );
        assert!(!now.iter().any(|(_, v)| v.contains("v1")));
        Ok(())
    }

    /// The finer-grained road the CLI cannot take: name the entity and
    /// the body derives nothing, so a single `method:` entry supersedes
    /// exactly that one fact and the entity stays put — the way
    /// re-authoring one view facet leaves the rest of `show` alone.
    ///
    /// This is what an author editing notation by hand gets, and it is
    /// why putting the methods in the digest costs nothing: derivation
    /// and editing are separate roads, and only the first one hashes.
    #[dialog_common::test]
    async fn it_supersedes_one_method_in_place_when_the_entity_is_named() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[
                    ("connected", "(self) => { self.textContent = 'v1'; }"),
                    ("bump", "(self) => 1"),
                ]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let entity = tonk_cli::views::entity_for_name(&test.site, "tally-widget")
            .await?
            .expect("the tag should resolve");

        test.eval_inline(&format!(
            "element!:\n  this: {entity}\n  method:\n    connected: |\n      (self) => {{ self.textContent = 'v2'; }}\n"
        ))
        .await?;

        let after = tonk_cli::views::entity_for_name(&test.site, "tally-widget").await?;
        assert_eq!(
            after.as_ref(),
            Some(&entity),
            "naming the entity should edit it in place, not repoint the tag",
        );
        let now = tonk_cli::elements::methods_of(&test.site, "tally-widget").await?;
        let keys: Vec<&str> = now.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["bump", "connected"], "{now:?}");
        assert!(
            now.iter()
                .any(|(k, v)| k == "connected" && v.contains("v2")),
            "{now:?}"
        );
        assert!(!now.iter().any(|(_, v)| v.contains("v1")), "{now:?}");
        Ok(())
    }

    /// Two elements that say different things are different entities,
    /// and the methods are what say it. This is the regression the
    /// digest change exists for: while nested fields were dropped from
    /// the body digest, every element whose body was only `method:`
    /// derived the same empty body, and the last tag authored silently
    /// took over the others.
    #[dialog_common::test]
    async fn it_gives_each_definition_an_entity_of_its_own() -> Result<()> {
        let test = TestSite::new().await?;
        for tag in ["a-one", "b-two"] {
            tonk_cli::data_ops::element_add(
                &test.site,
                tag,
                "The same description on purpose",
                &tonk_cli::authoring::ElementParts {
                    methods: &methods(&[("connected", &format!("(self) => '{tag}'"))]),
                    attributes: &[],
                    ..Default::default()
                },
                Default::default(),
            )
            .await?;
        }
        let a = tonk_cli::views::entity_for_name(&test.site, "a-one").await?;
        let b = tonk_cli::views::entity_for_name(&test.site, "b-two").await?;
        assert_ne!(
            a, b,
            "the descriptions match, so only the METHODS distinguish \
             these two — if they collapsed, the digest is dropping them",
        );
        let ours = tonk_cli::elements::list(&test.site)
            .await?
            .into_iter()
            .filter(|row| matches!(row.tag.as_deref(), Some("a-one") | Some("b-two")))
            .count();
        assert_eq!(ours, 2, "both definitions list, beside the library's own");
        Ok(())
    }

    /// And the converse, which is the same rule read forwards: two
    /// tags whose definitions are identical name one value. Nothing is
    /// lost — each tag resolves to the same methods — and it is what
    /// "the entity is derived from the body" means. Recorded here so
    /// the aliasing is a decision rather than a surprise.
    #[dialog_common::test]
    async fn it_lets_two_tags_share_one_identical_definition() -> Result<()> {
        let test = TestSite::new().await?;
        for tag in ["a-one", "b-two"] {
            tonk_cli::data_ops::element_add(
                &test.site,
                tag,
                "Identical in every respect",
                &tonk_cli::authoring::ElementParts {
                    methods: &methods(&[("connected", "(self) => {}")]),
                    attributes: &[],
                    ..Default::default()
                },
                Default::default(),
            )
            .await?;
        }
        let a = tonk_cli::views::entity_for_name(&test.site, "a-one").await?;
        let b = tonk_cli::views::entity_for_name(&test.site, "b-two").await?;
        assert_eq!(a, b, "identical definitions are one value");
        assert!(a.is_some());
        for tag in ["a-one", "b-two"] {
            let now = tonk_cli::elements::methods_of(&test.site, tag).await?;
            let keys: Vec<&str> = now.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(keys, vec!["connected"], "<{tag}>: {now:?}");
        }
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
            "The new shape",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => { self.textContent = 'new'; }")]),
                // A default, so this row answers the generic concept
                // query below — see
                // `it_answers_the_generic_concept_query_only_with_defaults`.
                attributes: &[("tone".to_owned(), "new".to_owned())],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;

        // The library seeds elements of its own; the rows this test
        // authored are what it asserts on.
        let listed = tonk_cli::elements::list(&test.site).await?;
        let ours: Vec<_> = listed
            .iter()
            .filter(|row| row.deprecated || row.tag.as_deref() == Some("new-widget"))
            .collect();
        assert_eq!(ours.len(), 2, "{listed:?}");
        let new = ours
            .iter()
            .find(|row| !row.deprecated)
            .expect("element row present");
        assert_eq!(new.tag.as_deref(), Some("new-widget"));
        assert_eq!(new.methods, vec!["connected"]);
        assert!(
            listed.iter().any(|row| row.deprecated),
            "component row disappeared: {listed:?}",
        );

        // Each shape's facts stay its own: the component's module is
        // not visible under the element's domains, and the element's
        // methods are not visible as a component.
        //
        // Read through the per-domain queries rather than through
        // `tonk query element`, which needs every dictionary set — see
        // `it_answers_the_generic_concept_query_only_when_every_map_is_set`.
        assert!(
            !tonk_cli::elements::methods_of(&test.site, "new-widget")
                .await?
                .is_empty(),
        );
        assert!(
            tonk_cli::elements::methods_of(&test.site, "old-widget")
                .await?
                .is_empty(),
            "the legacy module must not read back as element methods",
        );
        let components = tonk_cli::data_ops::query(&test.site, "component", false).await?;
        assert!(components.contains("old-widget"), "{components}");
        assert!(!components.contains("The new shape"), "{components}");
        Ok(())
    }

    /// Attribute defaults land as their own dictionary, read back
    /// under the tag, and an element that declares none reads as none
    /// rather than as an error.
    #[dialog_common::test]
    async fn it_stores_attribute_defaults_under_the_tag() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => {}")]),
                attributes: &[
                    ("color".to_owned(), "red".to_owned()),
                    ("size".to_owned(), String::new()),
                ],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let defaults = tonk_cli::elements::attributes_of(&test.site, "tally-widget").await?;
        assert_eq!(
            defaults,
            vec![
                ("color".to_owned(), "red".to_owned()),
                ("size".to_owned(), String::new()),
            ],
            "an empty default is a real one — `size=\"\"` is a state an \
             attribute can be in, distinct from declaring nothing",
        );

        tonk_cli::data_ops::element_add(
            &test.site,
            "plain-widget",
            "Declares no defaults",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => {}")]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        assert!(
            tonk_cli::elements::attributes_of(&test.site, "plain-widget")
                .await?
                .is_empty(),
        );
        // And it is still a first-class element: the methods read back,
        // and the tag resolves.
        assert!(
            !tonk_cli::elements::methods_of(&test.site, "plain-widget")
                .await?
                .is_empty(),
        );
        Ok(())
    }

    /// Editing one method leaves the defaults alone, and editing one
    /// default leaves the methods alone — the CLI carries each map
    /// forward independently.
    #[dialog_common::test]
    async fn it_carries_methods_and_defaults_forward_independently() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => 'v1'"), ("bump", "(self) => 1")]),
                attributes: &[("color".to_owned(), "red".to_owned())],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;

        // Author one method: the other method AND the default survive.
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => 'v2'")]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let keys: Vec<String> = tonk_cli::elements::methods_of(&test.site, "tally-widget")
            .await?
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, vec!["bump", "connected"]);
        assert_eq!(
            tonk_cli::elements::attributes_of(&test.site, "tally-widget").await?,
            vec![("color".to_owned(), "red".to_owned())],
            "authoring a method should not drop a default",
        );

        // Author one default: the methods survive and the default
        // supersedes.
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &[],
                attributes: &[("color".to_owned(), "blue".to_owned())],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        assert_eq!(
            tonk_cli::elements::attributes_of(&test.site, "tally-widget").await?,
            vec![("color".to_owned(), "blue".to_owned())],
        );
        let keys: Vec<String> = tonk_cli::elements::methods_of(&test.site, "tally-widget")
            .await?
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, vec!["bump", "connected"]);
        Ok(())
    }

    /// `description` is the only field an `element!:` body must set.
    /// Both dictionaries are keyed collections, which are zero-or-more,
    /// so a body that omits them is COMPLETE, not partial — it declares
    /// an element with no methods and no defaults.
    ///
    /// Pinned because the completeness check on a derived entity used
    /// to count a collection as missing: this body was refused, and so
    /// was every `element!:` that did not declare a default. The tag
    /// still resolves and the row still lists; the browser just leaves
    /// the tag inert, since there is nothing to register.
    #[dialog_common::test]
    async fn it_accepts_an_element_that_declares_only_a_description() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline("element!: &bare-widget\n  description: \"Nothing but a name\"\n")
            .await?;

        assert!(
            tonk_cli::views::entity_for_name(&test.site, "bare-widget")
                .await?
                .is_some(),
            "the anchor should still publish the tag",
        );
        assert!(
            tonk_cli::elements::methods_of(&test.site, "bare-widget")
                .await?
                .is_empty(),
        );
        assert!(
            tonk_cli::elements::attributes_of(&test.site, "bare-widget")
                .await?
                .is_empty(),
        );
        Ok(())
    }

    /// The complement: `description` is a plain required field, and a
    /// collection cannot stand in for it. A body carrying methods but
    /// no description derives an entity that says nothing about what
    /// the element is for, which is the thing the field exists to stop.
    #[dialog_common::test]
    async fn it_requires_a_description_even_when_methods_are_given() -> Result<()> {
        let test = TestSite::new().await?;
        for body in [
            "element!: &m-only\n  method:\n    connected: |\n      (self) => {}\n",
            "element!: &a-only\n  attribute:\n    color: \"red\"\n",
        ] {
            let err = test
                .eval_inline(body)
                .await
                .expect_err("a body with no description should be refused");
            let text = err.to_string();
            assert!(text.contains("description"), "{text}");
        }
        Ok(())
    }

    /// Collections are exempt from the completeness check because
    /// omitting one means zero entries. BLANKING one is different: `_`
    /// retracts, and retracting every field of an entity that does not
    /// exist yet sets nothing at all — on a derived entity every such
    /// body digests alike and collapses onto one subject.
    ///
    /// Uses a concept whose `with:` is nothing but a collection, since
    /// `element`'s required `description` would trip the check first.
    #[dialog_common::test]
    async fn it_refuses_a_body_that_only_blanks_its_collections() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(
            "concept!: &thing\n  description: \"A thing\"\n  with:\n    bit:\n      description: \"Bits\"\n      the: xyz.probe.thing.bit\n      as: {[symbol]: text}\n      cardinality: one\n",
        )
        .await?;

        // One entry is a complete body.
        test.eval_inline("thing!: &t-one\n  bit:\n    a: \"1\"\n")
            .await?;

        let err = test
            .eval_inline("thing!: &t-two\n  bit: _\n")
            .await
            .expect_err("a body that sets nothing should be refused");
        let text = err.to_string();
        assert!(
            text.contains("sets only some of the concept's fields"),
            "{text}",
        );
        Ok(())
    }

    /// Accessors round-trip as their own dictionaries, and each
    /// carries forward independently of the others.
    #[dialog_common::test]
    async fn it_stores_getters_and_setters_under_the_tag() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "counter-widget",
            "Counts, and says so through a property",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => {}")]),
                getters: &methods(&[("total", "(self) => Number(self.dataset.n ?? 0)")]),
                setters: &methods(&[(
                    "total",
                    "(self, next) => { self.dataset.n = String(next); }",
                )]),
                ..Default::default()
            },
            Default::default(),
        )
        .await?;

        let getters =
            tonk_cli::elements::entries_of(&test.site, "counter-widget", "getter").await?;
        let setters =
            tonk_cli::elements::entries_of(&test.site, "counter-widget", "setter").await?;
        assert_eq!(getters.len(), 1, "{getters:?}");
        assert_eq!(setters.len(), 1, "{setters:?}");
        assert!(getters[0].1.contains("Number("), "{getters:?}");
        assert!(setters[0].1.contains("String(next)"), "{setters:?}");
        // The two are distinct facts under distinct domains: reading
        // one must not answer with the other, which is the mistake a
        // shared domain or a swapped argument would produce.
        assert_ne!(getters[0].1, setters[0].1);

        // Author only the getter: the setter and the methods survive.
        tonk_cli::data_ops::element_add(
            &test.site,
            "counter-widget",
            "Counts, and says so through a property",
            &tonk_cli::authoring::ElementParts {
                getters: &methods(&[("total", "(self) => 99")]),
                ..Default::default()
            },
            Default::default(),
        )
        .await?;
        let getters =
            tonk_cli::elements::entries_of(&test.site, "counter-widget", "getter").await?;
        assert!(getters[0].1.contains("99"), "{getters:?}");
        assert_eq!(
            tonk_cli::elements::entries_of(&test.site, "counter-widget", "setter")
                .await?
                .len(),
            1,
            "editing a getter should not drop the setter",
        );
        assert!(
            !tonk_cli::elements::methods_of(&test.site, "counter-widget")
                .await?
                .is_empty(),
            "editing a getter should not drop the methods",
        );
        Ok(())
    }

    /// A dictionary the schema does not declare reads empty rather
    /// than erroring, so a caller out of step with the list does not
    /// look like a branch failure.
    #[dialog_common::test]
    async fn it_reads_an_unknown_dictionary_as_empty() -> Result<()> {
        let test = TestSite::new().await?;
        assert!(
            tonk_cli::elements::entries_of(&test.site, "tally-widget", "no-such-field")
                .await?
                .is_empty(),
        );
        Ok(())
    }

    /// What the four dictionaries cost: a GENERIC concept query binds
    /// every field the concept declares, and a keyed collection with no
    /// entries binds nothing — so `tonk query element` answers only for
    /// an element that declares ALL FOUR, which almost none do.
    ///
    /// Recorded rather than worked around, and worth stating plainly
    /// because it got worse as dictionaries were added: with `method`
    /// alone the generic query worked, with `attribute` it needed a
    /// default, and with accessors it needs all four. Fixing it means
    /// teaching the query layer to widen an empty collection, which is
    /// a dialog-query change, not a schema one.
    ///
    /// It is why `tonk element` reads the domains directly rather than
    /// going through `tonk query element`, and why the browser registry
    /// runs one query per dictionary instead of one for the concept.
    #[dialog_common::test]
    async fn it_answers_the_generic_concept_query_only_when_every_map_is_set() -> Result<()> {
        let test = TestSite::new().await?;
        let full = tonk_cli::authoring::ElementParts {
            methods: &methods(&[("connected", "(self) => {}")]),
            attributes: &methods(&[("color", "red")]),
            getters: &methods(&[("total", "(self) => 0")]),
            setters: &methods(&[("total", "(self, next) => {}")]),
        };
        tonk_cli::data_ops::element_add(
            &test.site,
            "every-map",
            "Declares all four dictionaries",
            &full,
            Default::default(),
        )
        .await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "methods-only",
            "Declares only methods",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => {}")]),
                ..Default::default()
            },
            Default::default(),
        )
        .await?;

        let generic = tonk_cli::data_ops::query(&test.site, "element", false).await?;
        assert!(generic.contains("Declares all four"), "{generic}");
        assert!(
            !generic.contains("Declares only methods"),
            "a collection with no entries binds nothing, so this row \
             cannot answer a query that pins every field: {generic}",
        );

        // The listing people actually use is unaffected: it reads the
        // method domain, which both elements have.
        // Among the library's own elements, the two this test authored.
        let listed = tonk_cli::elements::list(&test.site).await?;
        let tags: Vec<Option<&str>> = listed
            .iter()
            .map(|row| row.tag.as_deref())
            .filter(|tag| matches!(tag, Some("every-map") | Some("methods-only")))
            .collect();
        assert_eq!(tags, vec![Some("every-map"), Some("methods-only")]);
        Ok(())
    }

    /// The queries the BROWSER registry runs, executed here against a
    /// real branch and the real query engine.
    ///
    /// Everything else about the registry is exercised in a browser
    /// against a fake host, which proves the DOM half but takes the
    /// wire shapes on trust. These two queries are the seam between the
    /// two halves, and the ways they can be wrong — a `the:` written as
    /// an attribute where the schema declares a domain, a keyed
    /// collection missing its key operand — all read as "this element
    /// has no methods" rather than as an error. So run them for real.
    #[dialog_common::test]
    async fn the_browser_queries_resolve_against_a_real_branch() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[
                    ("connected", "(self) => { self.textContent = 'hi'; }"),
                    ("attribute-changed", "(self, name, before, after) => {}"),
                    ("bump", "(self) => 1"),
                ]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await?;

        // Hop one, exactly as the registry runs it: the tag's name to
        // the entity it currently means.
        let named = run(&test, tonk_template::resolve::name_query("tally-widget")).await?;
        let entity = named
            .first()
            .and_then(|row| row.fields.get("entity"))
            .and_then(ipld_string)
            .expect("the tag should resolve to an entity");

        // Hop two: that entity's whole method dictionary, one flat row
        // per entry, which the registry folds into the method table.
        let rows = run(
            &test,
            tonk_template::resolve::element_method_query(&entity).expect("the method query builds"),
        )
        .await?;
        let mut found: Vec<(String, String)> = Vec::new();
        for row in &rows {
            let Some(ipld_core::ipld::Ipld::Map(entries)) = row.fields.get("method") else {
                continue;
            };
            for (key, value) in entries {
                if let Some(source) = ipld_string(value) {
                    found.push((key.clone(), source));
                }
            }
        }
        found.sort();
        let keys: Vec<&str> = found.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["attribute-changed", "bump", "connected"],
            "every method should come back, keyed — a dictionary query \
             missing its key operand returns entries with no key",
        );
        assert!(
            found
                .iter()
                .any(|(key, source)| key == "connected" && source.contains("'hi'")),
            "the source should come back with the key: {found:?}",
        );
        Ok(())
    }

    /// The registry resolves through the NAME, so an element the tag no
    /// longer points at must not answer for it.
    #[dialog_common::test]
    async fn the_name_hop_follows_the_current_binding() -> Result<()> {
        let test = TestSite::new().await?;
        for tag in ["a-one", "b-two"] {
            tonk_cli::data_ops::element_add(
                &test.site,
                tag,
                &format!("The <{tag}> element"),
                &tonk_cli::authoring::ElementParts {
                    methods: &methods(&[("connected", &format!("(self) => '{tag}'"))]),
                    attributes: &[],
                    ..Default::default()
                },
                Default::default(),
            )
            .await?;
        }
        let a = run(&test, tonk_template::resolve::name_query("a-one")).await?;
        let b = run(&test, tonk_template::resolve::name_query("b-two")).await?;
        let entity_of = |rows: &Vec<tonk_schema::conclusion::Conclusion>| {
            rows.first()
                .and_then(|row| row.fields.get("entity"))
                .and_then(ipld_string)
        };
        let (a, b) = (entity_of(&a), entity_of(&b));
        assert!(a.is_some() && b.is_some(), "both tags should resolve");
        assert_ne!(a, b, "each tag must resolve to its own element");

        // And a name nothing has published resolves to nothing, which
        // is what leaves an unknown tag inert rather than erroring.
        let missing = run(&test, tonk_template::resolve::name_query("no-such")).await?;
        assert!(missing.is_empty(), "{missing:?}");
        Ok(())
    }

    /// Run a wire query the way the browser host would, against the
    /// real engine.
    async fn run(
        test: &TestSite,
        query: tonk_schema::query::Query,
    ) -> Result<Vec<tonk_schema::conclusion::Conclusion>> {
        use tonk_render::QueryBackend as _;
        let concept_query = query
            .into_concept_query()
            .map_err(|e| anyhow::anyhow!("query should lower: {e:?}"))?;
        test.site
            .query(concept_query)
            .await
            .map_err(|e| anyhow::anyhow!("query failed: {e}"))
    }

    fn ipld_string(value: &ipld_core::ipld::Ipld) -> Option<String> {
        match value {
            ipld_core::ipld::Ipld::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Both concepts are anchored, not pinned — their entities are
    /// content-addressed from their declarations. What coexistence
    /// actually needs is only that the two are DISTINCT and that each
    /// name resolves to its own, so neither can shadow the other.
    #[dialog_common::test]
    async fn it_resolves_each_concept_name_to_its_own_entity() -> Result<()> {
        let test = TestSite::new().await?;
        let element = tonk_cli::views::entity_for_name(&test.site, "element")
            .await?
            .expect("element should resolve");
        let component = tonk_cli::views::entity_for_name(&test.site, "component")
            .await?
            .expect("component should resolve");
        assert_ne!(
            element, component,
            "the two concepts collapsed onto one entity"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_tag_no_browser_would_register() -> Result<()> {
        let test = TestSite::new().await?;
        let err = tonk_cli::data_ops::element_add(
            &test.site,
            "widget",
            "No hyphen, no element",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("connected", "(self) => {}")]),
                attributes: &[],
                ..Default::default()
            },
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
            "A running tally",
            &tonk_cli::authoring::ElementParts {
                methods: &methods(&[("remove", "(self) => {}")]),
                attributes: &[],
                ..Default::default()
            },
            Default::default(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err}").contains("shadow"), "{err}");
        Ok(())
    }
}
