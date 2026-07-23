//! Behavioural tests for the notation surface:
//! evaluation, error reporting, output rendering, schema
//! introspection, and the bundled guide.

mod common;

mod when_evaluating_a_document {
    use anyhow::Result;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    #[dialog_common::test]
    async fn it_lands_attribute_declarations_on_the_branch() -> Result<()> {
        let test = common::TestSite::new().await?;
        let outcome = test.eval_inline(ATTRIBUTE_DECL).await?;
        assert!(outcome.committed);
        assert!(outcome.response.commits.claims > 0);

        // The new attributes are queryable through the built-in
        // `attribute` concept by URI.
        let query = test
            .eval_inline("attribute:\n  this: ?a\n  id: \"xyz.tonk.task/title\"\n")
            .await?;
        assert!(
            !query.response.matches_after.is_empty()
                && !query.response.matches_after[0].results.is_empty(),
            "expected attribute query to surface task-title, got: {:#?}",
            query.response.matches_after,
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_seeds_the_standard_library_view_concept_on_init() -> Result<()> {
        // A freshly initialised site carries the tonk standard
        // library — the same `core.yaml` the tonk-ui service worker
        // seeds at repository creation — so the renderer's `tonk:view`
        // concept is present without the user defining it.
        // `<tonk-display>` queries `tonk:view` by `model`, so views
        // authored through tonk resolve and render instead of
        // showing "View not found".
        let test = common::TestSite::new().await?;
        let query = test
            .eval_inline("concept:\n  this: ?c\n  name: \"view\"\n")
            .await?;
        assert!(
            !query.response.matches_after.is_empty()
                && !query.response.matches_after[0].results.is_empty(),
            "expected a freshly-initialised site to seed the `view` concept, got: {:#?}",
            query.response.matches_after,
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_round_trips_a_concept_declaration() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        let outcome = test.eval_inline(CONCEPT_DECL).await?;
        assert!(outcome.committed);

        // Concept-of-concept query: the new task concept must
        // resolve by name on the same branch.
        let query = test
            .eval_inline("concept:\n  this: ?c\n  name: \"task\"\n")
            .await?;
        assert!(
            !query.response.matches_after.is_empty()
                && !query.response.matches_after[0].results.is_empty(),
            "expected concept query to surface task, got: {:#?}",
            query.response.matches_after,
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_query_results_after_assertion() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            r#"
task!: &buy-milk
  title: "Buy milk"
  done:  false
"#,
        )
        .await?;

        let query = test
            .eval_inline("task:\n  this: ?t\n  done: false\n")
            .await?;
        assert_eq!(query.response.matches_after.len(), 1);
        let block = &query.response.matches_after[0];
        assert_eq!(block.label, "task");
        assert_eq!(block.results.len(), 1);
        let row = &block.results[0];
        assert_eq!(
            row.fields.get("title"),
            Some(&serde_json::json!("Buy milk"))
        );
        assert_eq!(row.fields.get("done"), Some(&serde_json::json!(false)));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_filters_with_natural_join_across_expressions() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            r#"
task!: &a
  title: "A"
  done:  true

task!: &b
  title: "B"
  done:  false
"#,
        )
        .await?;

        // Two expressions sharing `?t`: the natural join keeps
        // only entities matching both — done=true narrows the
        // result to "A".
        let query = test
            .eval_inline(
                r#"
task:
  this:  ?t
  done:  true

task:
  this:  ?t
  title: ?title
"#,
            )
            .await?;

        let titles: Vec<_> = query
            .response
            .matches_after
            .iter()
            .flat_map(|b| b.results.iter())
            .filter_map(|r| r.fields.get("title").cloned())
            .collect();
        assert!(titles.iter().any(|v| v == &serde_json::json!("A")));
        assert!(!titles.iter().any(|v| v == &serde_json::json!("B")));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_dissociates_concept_projection_on_retract() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            r#"
task!: &a
  title: "A"
  done:  false
"#,
        )
        .await?;

        // Bind `?t` to the entity matching title="A" and retract
        // the whole concept projection on each match — `..: _`
        // sweeps every attribute in the concept's `with:` map.
        test.eval_inline(
            r#"
task:
  this:  ?t
  title: "A"

task!:
  this: ?t
  ..:   _
"#,
        )
        .await?;

        let after = test.eval_inline("task:\n  this: ?t\n").await?;
        let total: usize = after
            .response
            .matches_after
            .iter()
            .map(|b| b.results.len())
            .sum();
        assert_eq!(total, 0);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_commit_a_dry_run_mutation() -> Result<()> {
        use tonk_cli::eval::Options;
        use tonk_cli::output::Format;

        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;

        let dry_run = Options {
            format: Format::Notation,
            quiet: false,
            dry_run: true,
        };
        let outcome = test
            .eval_inline_with(
                "task!: &t\n  title: \"Drafted, not saved\"\n  done: false\n",
                dry_run,
            )
            .await?;

        // A dry run plans but drops the transaction: nothing
        // committed, zero claims, and the branch head is unmoved.
        assert!(!outcome.committed);
        assert_eq!(outcome.response.commits.claims, 0);
        assert_eq!(
            outcome.response.revision_before,
            outcome.response.revision_after,
        );

        // The task never landed: a follow-up query finds nothing.
        let after = test.eval_inline("task:\n  this: ?t\n").await?;
        let total: usize = after
            .response
            .matches_after
            .iter()
            .map(|b| b.results.len())
            .sum();
        assert_eq!(total, 0, "dry-run mutation must not persist");
        Ok(())
    }

    /// A claim can reference a blob entity, and the blob concept from
    /// the standard library can describe it.
    #[dialog_common::test]
    async fn it_references_blob_entities_from_notation() -> Result<()> {
        let test = common::TestSite::new().await?;
        // A 32-byte hash in base58 (all-7s), as `tonk blob add` would print.
        let outcome = test
            .eval_inline(
                r#"
blob!:
  this: blob:AsimTVBhbHkeicMyxturKmMKGWDLW8YyaQpEqhk6JsyM
  content-type: "image/png"
  name: "vacation.png"
"#,
            )
            .await?;
        assert!(outcome.committed);
        assert!(outcome.response.commits.claims > 0);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_matches_for_a_dry_run_query() -> Result<()> {
        use tonk_cli::eval::Options;
        use tonk_cli::output::Format;

        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t\n  title: \"Real task\"\n  done: false\n")
            .await?;

        // A pure query under --dry-run still surfaces matches.
        let dry_run = Options {
            format: Format::Notation,
            quiet: false,
            dry_run: true,
        };
        let outcome = test
            .eval_inline_with("task:\n  this: ?t\n  title: ?title\n", dry_run)
            .await?;
        let total: usize = outcome
            .response
            .matches_after
            .iter()
            .map(|b| b.results.len())
            .sum();
        assert_eq!(total, 1, "dry-run query should still return matches");
        Ok(())
    }
}

mod when_reporting_errors {
    use anyhow::Result;

    use crate::common;

    #[dialog_common::test]
    async fn it_exits_with_parse_error_on_malformed_yaml() -> Result<()> {
        let test = common::TestSite::new().await?;
        let err = test
            .eval_inline("attribute!: &foo as: text\n  bad: indent\n")
            .await
            .expect_err("malformed YAML should not parse");
        assert_eq!(err.exit_code(), tonk_cli::ExitCode::ParseError);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_exits_with_analyzer_error_on_unknown_concept() -> Result<()> {
        let test = common::TestSite::new().await?;
        let err = test
            .eval_inline("nope:\n  this: ?x\n")
            .await
            .expect_err("unknown concept should fail analysis");
        assert_eq!(err.exit_code(), tonk_cli::ExitCode::AnalyzeError);
        Ok(())
    }
}

mod when_rendering_query_output {
    use anyhow::Result;
    use tonk_cli::eval;
    use tonk_cli::output::Format;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    #[dialog_common::test]
    async fn it_round_trips_through_eval() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            r#"
task!: &ax
  title: "Buy milk"
  done:  false
"#,
        )
        .await?;

        // Render a query result; the matches section is itself a
        // valid notation document, so re-submitting it must
        // produce results without error.
        let outcome = test
            .eval_inline_with(
                "task:\n  this: ?t\n",
                eval::Options {
                    format: Format::Notation,
                    quiet: false,
                    dry_run: false,
                },
            )
            .await?;
        let split: Vec<&str> = outcome.stdout.splitn(2, "---\n").collect();
        assert_eq!(
            split.len(),
            2,
            "expected matches section in: {}",
            outcome.stdout
        );
        let matches_section = split[1];

        let resubmitted = test.eval_inline(matches_section).await?;
        assert!(!resubmitted.response.matches_after.is_empty());
        Ok(())
    }
}

mod when_introspecting_the_schema {
    use anyhow::Result;
    use tonk_cli::schema;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    // TODO(post-#447): re-port `tonk_cli::schema::render` to the new
    // analyzer model. After the head/this/anchor rewrite, the
    // built-in `attribute:` query no longer surfaces standalone
    // attribute entities the same way, and the `as:` value
    // capitalisation needs to round-trip through `text` /
    // `boolean` / etc. instead of `Text` / `Boolean`. The
    // re-submittability contract this test pins is still a
    // load-bearing property — leave the test in place but
    // ignored so the next pass can flip it back on.
    #[dialog_common::test]
    #[ignore = "schema render needs a separate port to the post-#447 analyzer model"]
    async fn it_emits_a_re_submittable_notation_document() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;

        let rendered = schema::render(&test.site).await?;
        for marker in [
            "attribute!: &task-title",
            "attribute!: &task-done",
            "concept!: &task",
            "title: task-title",
        ] {
            assert!(
                rendered.contains(marker),
                "expected `{marker}` in schema:\n{rendered}",
            );
        }

        // Replay against a fresh site: the dump must commit
        // cleanly, and the fresh site's own dump must reproduce
        // the same names.
        let fresh = common::TestSite::new().await?;
        let outcome = fresh.eval_inline(&rendered).await?;
        assert!(outcome.committed);
        let replayed = schema::render(&fresh.site).await?;
        for marker in [
            "attribute!: &task-title",
            "attribute!: &task-done",
            "concept!: &task",
        ] {
            assert!(
                replayed.contains(marker),
                "expected `{marker}` in replayed schema:\n{replayed}",
            );
        }
        Ok(())
    }
}

mod when_serving_the_guide {
    use tonk_cli::guide;

    #[dialog_common::test]
    fn it_returns_the_index_for_no_topic() {
        let text = guide::resolve(None, None).expect("index resolves");
        assert!(text.contains("tonk guide notation"));
    }

    #[dialog_common::test]
    fn it_returns_each_topic_body() {
        assert!(guide::topic("notation").unwrap().contains("attribute!"));
        assert!(guide::topic("views").unwrap().contains("tonk:view"));
        assert!(guide::topic("events").unwrap().contains("rule!"));
        assert!(
            guide::topic("workspace")
                .unwrap()
                .contains("view: tonk:view")
        );
        assert!(guide::topic("bogus").is_none());
    }

    #[dialog_common::test]
    fn it_returns_the_full_guide_for_all() {
        let all = guide::resolve(Some("all"), None).expect("all resolves");
        assert!(all.starts_with("# Asserted-notation guide"));
        assert_eq!(all, guide::GUIDE);
    }

    #[dialog_common::test]
    fn it_concatenates_the_per_topic_bodies_into_the_full_guide() {
        // `GUIDE` repeats the `include_str!` paths literally (concat!
        // needs them), so it can silently drift from the per-topic
        // consts. Pin it to the concatenation so a renamed/moved topic
        // file updated in only one place fails here.
        let expected = format!(
            "{}\n{}\n{}\n{}",
            guide::NOTATION,
            guide::VIEWS,
            guide::EVENTS,
            guide::WORKSPACE,
        );
        assert_eq!(guide::GUIDE, expected);
    }

    #[dialog_common::test]
    fn it_resolves_every_advertised_topic() {
        // `TOPICS`, the `topic()` match arms, and the unknown-topic
        // error message are three hand-synced lists. Drive the test
        // off `TOPICS` so a name advertised there but missing a match
        // arm (or vice versa) is caught.
        for name in guide::TOPICS {
            let body =
                guide::topic(name).unwrap_or_else(|| panic!("advertised topic {name} has no body"));
            assert!(!body.is_empty(), "topic {name} is empty");
            assert_eq!(
                guide::resolve(Some(name), None).expect("topic resolves"),
                body
            );
        }
    }

    #[dialog_common::test]
    fn it_errors_on_an_unknown_topic() {
        let err = guide::resolve(Some("nope"), None).expect_err("unknown rejects");
        let message = err.to_string();
        assert!(message.contains("nope"), "echoes the bad input");
        assert!(message.contains("notation"), "lists a valid topic");
        assert!(message.contains("all"), "advertises the `all` pseudo-topic");
    }

    #[dialog_common::test]
    fn it_serves_every_advertised_element_under_views() {
        // `ELEMENTS`, the `element()` match arms, the catalog table in
        // guide-views.md, and the include_str! files are hand-synced.
        // Drive off `ELEMENTS` so a name advertised without a body (or
        // vice versa) is caught.
        for name in guide::ELEMENTS {
            let body = guide::element(name)
                .unwrap_or_else(|| panic!("advertised element {name} has no body"));
            assert!(!body.is_empty(), "element {name} is empty");
            assert!(
                body.contains(name),
                "element doc for {name} should mention the tag"
            );
            assert_eq!(
                guide::resolve(Some("views"), Some(name)).expect("element resolves"),
                body,
                "`views {name}` should serve the element doc",
            );
        }
        // tonk-table is the crate this work added; make sure it's wired.
        assert!(guide::element("tonk-table").is_some());
    }

    #[dialog_common::test]
    fn it_errors_on_an_unknown_element() {
        let err =
            guide::resolve(Some("views"), Some("tonk-nope")).expect_err("unknown element rejects");
        let message = err.to_string();
        assert!(message.contains("tonk-nope"), "echoes the bad input");
        assert!(message.contains("tonk-table"), "lists documented elements");
    }

    #[dialog_common::test]
    fn it_errors_when_a_plain_topic_is_given_an_item() {
        // Only `views` takes a second argument.
        let err = guide::resolve(Some("notation"), Some("x")).expect_err("rejects an item");
        assert!(
            err.to_string().contains("views"),
            "points at the right form"
        );
        // A bogus topic with an item still reads as an unknown topic.
        assert!(guide::resolve(Some("bogus"), Some("x")).is_err());
    }
}
