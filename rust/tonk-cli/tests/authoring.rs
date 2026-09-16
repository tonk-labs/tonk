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
        assert_eq!(listed[0].methods, vec!["connected", "disconnected"]);
        assert!(!listed[0].deprecated);
        Ok(())
    }

    /// The property the `method:` dictionary exists for: authoring one
    /// method leaves the others standing. It works without a pin
    /// because the tag reaches the digest as a scalar (so the entity
    /// is this element's own) while the methods do not (so it stays
    /// put as they change).
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
        let before = tonk_cli::views::entity_for_name(&test.site, "tally-widget").await?;
        tonk_cli::data_ops::element_add(
            &test.site,
            "tally-widget",
            &methods(&[("connected", "(self) => { self.textContent = 'v2'; }")]),
            Default::default(),
        )
        .await?;
        let after = tonk_cli::views::entity_for_name(&test.site, "tally-widget").await?;
        assert_eq!(before, after, "editing a method should not move the entity");

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

    /// Every element needs its own entity. The methods are a nested
    /// map and carry no content identity, so without the scalar `name`
    /// every element on a branch digests to the same empty body and
    /// the last one authored silently replaces all the others.
    #[dialog_common::test]
    async fn it_gives_each_tag_an_entity_of_its_own() -> Result<()> {
        let test = TestSite::new().await?;
        for tag in ["a-one", "b-two"] {
            tonk_cli::data_ops::element_add(
                &test.site,
                tag,
                &methods(&[("connected", "(self) => {}")]),
                Default::default(),
            )
            .await?;
        }
        let a = tonk_cli::views::entity_for_name(&test.site, "a-one").await?;
        let b = tonk_cli::views::entity_for_name(&test.site, "b-two").await?;
        assert_ne!(a, b, "two tags collapsed onto one entity");
        assert_eq!(tonk_cli::elements::list(&test.site).await?.len(), 2);
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
            &methods(&[
                ("connected", "(self) => { self.textContent = 'hi'; }"),
                ("attribute-changed", "(self, name, before, after) => {}"),
                ("bump", "(self) => 1"),
            ]),
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
                &methods(&[("connected", &format!("(self) => '{tag}'"))]),
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
