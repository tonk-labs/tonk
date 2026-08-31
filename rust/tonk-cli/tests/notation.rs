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
            home: None,
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
            home: None,
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
    use tonk_cli::Coded;

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
                    home: None,
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
    use crate::common;
    use tonk_cli::eval::{self, Source};
    use tonk_cli::guide;

    #[derive(Debug)]
    struct GuideYamlFence {
        info: String,
        body: String,
        opening_line: usize,
    }

    fn guide_yaml_fences(topic: &str, body: &str) -> Vec<GuideYamlFence> {
        let lines: Vec<_> = body.lines().collect();
        let mut fences = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            let info = lines[index].trim_start();
            if !info.starts_with("```yaml") {
                index += 1;
                continue;
            }

            let opening_line = index + 1;
            let fence_info = info.trim_start_matches("```").to_string();
            index += 1;
            let body_start = index;
            while index < lines.len() && lines[index].trim() != "```" {
                index += 1;
            }
            assert!(
                index < lines.len(),
                "{topic}:{opening_line}: unterminated YAML fence"
            );
            fences.push(GuideYamlFence {
                info: fence_info,
                body: lines[body_start..index].join("\n"),
                opening_line,
            });
            index += 1;
        }

        fences
    }

    #[dialog_common::test]
    async fn guide_notation_examples_are_classified_and_executable() {
        for (topic, body) in [
            ("notation", guide::NOTATION),
            ("views", guide::VIEWS),
            ("events", guide::EVENTS),
        ] {
            for fence in guide_yaml_fences(topic, body) {
                let location = format!("{topic}:{}", fence.opening_line);
                match fence.info.as_str() {
                    "yaml tonk=parse" => {
                        let parsed = tonk_notation::parse(&fence.body);
                        assert!(
                            parsed.diagnostics.is_empty(),
                            "{location}: parse example has diagnostics: {:#?}",
                            parsed.diagnostics
                        );
                    }
                    "yaml tonk=eval" => {
                        let test = common::TestSite::new().await.unwrap_or_else(|error| {
                            panic!("{location}: create test site: {error}")
                        });
                        let outcome = eval::run_against_site(
                            &test.site,
                            Source::Inline(fence.body),
                            eval::Options {
                                dry_run: true,
                                ..eval::Options::default()
                            },
                        )
                        .await
                        .unwrap_or_else(|error| panic!("{location}: eval example failed: {error}"));
                        assert_eq!(
                            outcome.response.revision_before, outcome.response.revision_after,
                            "{location}: dry run changed the revision"
                        );
                        assert_eq!(
                            outcome.response.commits.claims, 0,
                            "{location}: dry run reported committed claims"
                        );
                    }
                    info if info.starts_with("yaml tonk=illustrative-")
                        && info.len() > "yaml tonk=illustrative-".len() => {}
                    _ => panic!(
                        "{location}: YAML fence must use tonk=eval, tonk=parse, or a specific tonk=illustrative-<reason> classification; got `{}`",
                        fence.info
                    ),
                }
            }
        }
    }

    #[dialog_common::test]
    fn it_returns_each_topic_body() {
        assert!(
            guide::topic("glossary")
                .unwrap()
                .contains("content-addressed")
        );
        assert!(guide::topic("notation").unwrap().contains("attribute!"));
        assert!(guide::topic("spaces").unwrap().contains("TONK_SPACE"));
        assert!(guide::topic("tutorial").unwrap().contains("tonk assert"));
        assert!(guide::topic("sync").unwrap().contains("upstream"));
        assert!(guide::topic("views").unwrap().contains("tonk:view"));
        assert!(guide::topic("events").unwrap().contains("rule!"));
        assert!(guide::topic("bogus").is_none());
    }

    #[dialog_common::test]
    fn it_returns_the_full_guide_for_all() {
        assert!(guide::GUIDE.starts_with("# Glossary"));
        for topic in guide::TOPICS {
            let heading = guide::topic(topic)
                .expect("topic body")
                .lines()
                .next()
                .unwrap();
            assert!(guide::GUIDE.contains(heading), "full guide omits {topic}");
        }
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
            assert!(
                guide::description(name).is_some(),
                "topic {name} lacks summary"
            );
        }
        assert!(guide::topic("bogus").is_none());
    }

    #[dialog_common::test]
    fn raw_view_examples_pin_the_entity_they_update() {
        for (topic, body) in [("views", guide::VIEWS), ("events", guide::EVENTS)] {
            let lines: Vec<_> = body.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if !line.starts_with("view") || !line.contains("!: &") {
                    continue;
                }
                let pins_entity = lines[index + 1..]
                    .iter()
                    .take_while(|line| line.is_empty() || line.starts_with("  "))
                    .any(|line| line.trim_start().starts_with("this:"));
                assert!(
                    pins_entity,
                    "{topic} guide has an unstable raw view assertion: {line}"
                );
            }
        }
    }
}

mod when_rejecting_overlapping_transient_commands {
    use anyhow::Result;
    use tonk_cli::eval::{self, EvalError, Source};

    use crate::common;

    #[dialog_common::test]
    async fn overlapping_transient_commands_fail_without_mutating() -> Result<()> {
        let test = common::TestSite::new().await?;
        let before = test.site.branch().await?.handle().revision();
        let doc = r#"concept!: &toggle-result
  with:
    todo: { description: The todo, the: xyz.tonk.result/todo, as: entity }
    checked: { description: The checked state, the: xyz.tonk.result/checked, as: boolean }

concept!: &remove-result
  with:
    todo: { description: The todo, the: xyz.tonk.result/todo, as: entity }

command!: &toggle-todo
  with:
    todo: { description: The todo, the: dom.event.current-target.dataset/todo, as: entity }
    checked: { description: The checked state, the: dom.event.current-target/checked, as: boolean }

command!: &remove-todo
  with:
    todo: { description: The todo, the: dom.event.current-target.dataset/todo, as: entity }

rule!:
  assert!: toggle-result
  when:
    - assert: toggle-todo
      where: { this: ?this, todo: ?todo, checked: ?checked }

rule!:
  assert!: remove-result
  when:
    - assert: remove-todo
      where: { this: ?this, todo: ?todo }
"#;

        let error = eval::run_against_site(
            &test.site,
            Source::Inline(doc.to_owned()),
            eval::Options {
                dry_run: true,
                ..eval::Options::default()
            },
        )
        .await
        .expect_err("unsafe command shapes must fail analysis");
        let EvalError::Analyze(message) = error else {
            panic!("expected analyzer error, got {error}");
        };
        assert!(message.contains("toggle-todo"), "{message}");
        assert!(message.contains("remove-todo"), "{message}");
        assert_eq!(test.site.branch().await?.handle().revision(), before);
        Ok(())
    }
}

mod eval_home {
    use anyhow::Result;
    use tonk_cli::eval::{self, Source};

    use crate::common;

    const TODO_DIRECTORY_VIEW: &str = r#"view!:
  this: todo
  show:
    directory: |
      <li>{title}</li>
"#;

    async fn seed_todo(test: &common::TestSite) -> Result<()> {
        tonk_cli::data_ops::concept_add(
            &test.site,
            "todo",
            &["title:text:one".into()],
            Some("a todo"),
            Default::default(),
        )
        .await?;
        tonk_cli::data_ops::assert_op(
            &test.site,
            "todo",
            None,
            &["--title".into(), "Write".into()],
        )
        .await?;
        Ok(())
    }

    async fn render_home(test: &common::TestSite) -> Result<String> {
        let replica = tonk_cli::data_ops::query(&test.site, "tonk/replica", false).await?;
        let entity = replica
            .lines()
            .find_map(|line| line.trim().strip_prefix("this: ").map(str::to_owned))
            .expect("a fresh site has a replica entity");
        let route = tonk_cli::render::RenderRoute::parse(&format!("{entity}@tonk/space"))?;
        Ok(tonk_cli::render::render(&test.site, &route).await?)
    }

    #[dialog_common::test]
    async fn it_installs_a_view_and_home_in_one_eval_commit() -> Result<()> {
        let test = common::TestSite::new().await?;
        seed_todo(&test).await?;
        let before = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("revision before eval home")
            .edition
            .value();

        eval::run_against_site(
            &test.site,
            Source::Inline(TODO_DIRECTORY_VIEW.to_owned()),
            eval::Options {
                home: Some("todo".to_owned()),
                ..eval::Options::default()
            },
        )
        .await?;

        let after = test
            .site
            .branch()
            .await?
            .handle()
            .revision()
            .expect("revision after eval home")
            .edition
            .value();
        assert_eq!(after, before + 1, "view and home must share one commit");

        let html = render_home(&test).await?;
        assert!(
            html.contains("Write"),
            "eval home did not render todo:\n{html}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_accepts_a_home_concept_declared_in_the_same_document() -> Result<()> {
        let test = common::TestSite::new().await?;
        let doc = format!(
            "concept!: &todo\n  with:\n    title: {{ description: The title, the: xyz.tonk.todo/title, as: text }}\n\ntodo!: &write\n  title: \"Write\"\n\n{TODO_DIRECTORY_VIEW}"
        );
        eval::run_against_site(
            &test.site,
            Source::Inline(doc),
            eval::Options {
                home: Some("todo".to_owned()),
                ..eval::Options::default()
            },
        )
        .await?;

        let html = render_home(&test).await?;
        assert!(html.contains("Write"), "same-document home failed:\n{html}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_previews_eval_home_without_mutating() -> Result<()> {
        let test = common::TestSite::new().await?;
        seed_todo(&test).await?;
        let before = test.site.branch().await?.handle().revision();

        let outcome = eval::run_against_site(
            &test.site,
            Source::Inline(TODO_DIRECTORY_VIEW.to_owned()),
            eval::Options {
                dry_run: true,
                home: Some("todo".to_owned()),
                ..eval::Options::default()
            },
        )
        .await?;

        assert!(!outcome.committed);
        assert_eq!(
            outcome.response.revision_before,
            outcome.response.revision_after
        );
        assert_eq!(test.site.branch().await?.handle().revision(), before);
        let html = render_home(&test).await?;
        assert!(
            !html.contains("Write"),
            "dry run replaced the home:\n{html}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_eval_home_for_an_unknown_concept_without_mutating() -> Result<()> {
        let test = common::TestSite::new().await?;
        seed_todo(&test).await?;
        let before = test.site.branch().await?.handle().revision();

        let error = eval::run_against_site(
            &test.site,
            Source::Inline(TODO_DIRECTORY_VIEW.to_owned()),
            eval::Options {
                home: Some("missing".to_owned()),
                ..eval::Options::default()
            },
        )
        .await
        .expect_err("an unknown home concept must fail analysis");

        assert!(matches!(error, eval::EvalError::Analyze(_)), "{error}");
        assert_eq!(test.site.branch().await?.handle().revision(), before);
        Ok(())
    }
}
