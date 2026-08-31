mod common;

use anyhow::Result;

use crate::common::{
    ATTRIBUTE_DECL, CONCEPT_DECL, NOTE_ATTRIBUTE_DECL, NOTE_CONCEPT_DECL, TestSite,
};

// The verbs are exercised through the library handlers, not the binary,
// to avoid spawning a subprocess. Each handler returns its rendered
// stdout + an ExitCode.

/// Pull every raw `(the, ?of, ?is)` claim for a bare attribute URI
/// directly off `main` — bypasses the concept query entirely, so it
/// can tell a single-field retraction (one attribute's claims gone,
/// the rest untouched) apart from a whole-instance retraction (every
/// attribute's claims gone). A concept-completeness query can't make
/// that distinction: it requires every `with:` field bound, so it
/// stops matching the row the instant *any* field is missing.
async fn select_claims(test: &TestSite, the: &str) -> Result<Vec<dialog_query::Claim>> {
    use anyhow::anyhow;
    use dialog_artifacts::Attribute;
    use dialog_query::{AttributeQuery, Output as _, Term, attribute};

    let attr: Attribute = the
        .parse()
        .map_err(|e| anyhow!("{the} should be a valid attribute URI: {e:?}"))?;
    let the_term: attribute::The = attr.into();
    let session = test.site.branch().await?;
    session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(the_term),
            Term::<dialog_artifacts::Entity>::var("of"),
            Term::<dialog_query::Any>::var("is"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&test.site.operator)
        .try_vec()
        .await
        .map_err(|e| anyhow!("{the} query failed: {e:?}"))
}

mod when_rendering_a_concept_schema_subset {
    use super::*;
    use crate::common::VIEW_DECL;

    #[dialog_common::test]
    async fn it_emits_only_the_named_concepts_notation() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // A second concept on the branch that must NOT appear.
        test.eval_inline(VIEW_DECL).await?;
        let out = tonk_cli::data_ops::schema_subset(&test.site, "task").await?;
        assert!(
            out.contains("concept!: &task"),
            "subset should carry the concept block:\n{out}"
        );
        assert!(
            out.contains("attribute!: &task-title"),
            "subset should carry the referenced attribute decls:\n{out}"
        );
        assert!(
            !out.contains("&view") && !out.contains("html-body"),
            "subset must not leak other concepts:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resubmits_cleanly_on_a_fresh_site() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let out = tonk_cli::data_ops::schema_subset(&test.site, "task").await?;
        // Same format as `tonk show --notation`: the subset is a valid
        // notation document a fresh branch accepts wholesale.
        let fresh = TestSite::new().await?;
        fresh.eval_inline(&out).await?;
        let described = tonk_cli::data_ops::schema_subset(&fresh.site, "task").await?;
        assert!(
            described.contains("concept!: &task"),
            "re-submitted subset should reconstruct the concept:\n{described}"
        );
        Ok(())
    }
}

mod when_the_concept_is_unknown {
    use super::*;

    #[dialog_common::test]
    async fn it_errors_with_a_known_concepts_list() -> anyhow::Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let err = tonk_cli::data_ops::schema_subset(&test.site, "widget")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("task"),
            "unknown-concept error should list known concepts: {msg}"
        );
        Ok(())
    }
}

mod when_reading_instances {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_all_instances_of_a_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            "task!:\n  title: \"alpha\"\n  done: false\ntask!:\n  title: \"beta\"\n  done: false\n",
        )
        .await?;
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            out.contains("alpha") && out.contains("beta"),
            "list should show both:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_emits_json_when_requested() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!:\n  title: \"gamma\"\n  done: false\n")
            .await?;
        let out = tonk_cli::data_ops::query(&test.site, "task", true).await?;
        assert!(
            out.trim_start().starts_with('{') || out.trim_start().starts_with('['),
            "json output:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_gets_a_single_instance_by_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let outcome = test
            .eval_inline("task!: &t\n  title: \"delta\"\n  done: false\n")
            .await?;
        let entity = outcome
            .response
            .commits
            .entities
            .get("t")
            .expect("entity 't' should be bound in the commit summary")
            .clone();

        let out = tonk_cli::data_ops::get(&test.site, "task", &entity, false).await?;
        assert!(
            out.contains("delta"),
            "get should show the fetched instance:\n{out}"
        );
        Ok(())
    }
}

// `task` (see `CONCEPT_DECL`) has two fields declared under `with:`
// — `title` (Text) and `done` (Boolean) — and neither is declared
// via a `maybe:` block, so `ConceptFieldDescriptor::is_optional()`
// is `false` for both: `add`'s dynamic clap `Command` (built with
// `all_required=true`) makes *both* required args. A `--title`-only
// add therefore fails clap's required-argument check before
// anything is built or committed, so the happy-path test below
// supplies both flags.
mod when_asserting_a_new_instance {
    use super::*;

    #[dialog_common::test]
    async fn it_commits_an_instance_from_typed_flags() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec![
            "--title".to_string(),
            "Write the plan".to_string(),
            "--done".to_string(),
            "false".to_string(),
        ];
        tonk_cli::data_ops::assert_op(&test.site, "task", None, &argv).await?;
        // Verify it landed: list should now show the title.
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            out.contains("Write the plan"),
            "added instance should appear in list:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_enumerating_valid_flags_on_unknown_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec!["--nope".to_string(), "x".to_string()];
        let err = tonk_cli::data_ops::assert_op(&test.site, "task", None, &argv)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("title"),
            "error should enumerate valid flags:\n{msg}"
        );
        Ok(())
    }

    /// The usage line is what an agent reads on every mis-shaped
    /// write, so it has to be a command that runs, not a sketch of
    /// one. It used to render `tonk … task`, with a literal ellipsis
    /// where the verb belongs.
    #[dialog_common::test]
    async fn it_renders_a_runnable_usage_line() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let argv = vec!["--nope".to_string(), "x".to_string()];

        let mint = tonk_cli::data_ops::assert_op(&test.site, "task", None, &argv)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            mint.contains("Usage: tonk assert task "),
            "mint form should name the verb and the concept:\n{mint}"
        );
        assert!(
            mint.contains("--title <TEXT>"),
            "mint form should name the required flags:\n{mint}"
        );
        assert!(!mint.contains('…'), "no placeholder ellipsis:\n{mint}");

        // The supersede form takes the entity between the concept and
        // the flags, so its usage line has to show that too.
        test.eval_inline("task!: &chore\n  title: \"Chore\"\n  done: false\n")
            .await?;
        let supersede = tonk_cli::data_ops::assert_op(&test.site, "task", Some("chore"), &argv)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            supersede.contains("Usage: tonk assert task <ENTITY>"),
            "supersede form should show where the entity goes:\n{supersede}"
        );
        assert!(
            !supersede.contains('…'),
            "no placeholder ellipsis:\n{supersede}"
        );
        Ok(())
    }
}

// `task` has two required fields (`title`, `done`; see the note
// above `when_adding_an_instance`), so a bare `&anchor` seed with a
// name but no `this:` field trips the analyzer's "no incomplete
// fresh-entity assertion" rule unless every required field is
// supplied up front — every seed below sets both.
mod when_superseding_and_retracting {
    use super::*;

    #[dialog_common::test]
    async fn it_overwrites_a_field_on_a_named_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // Seed a named task `t1` with an anchor so it is addressable
        // by name across separate eval calls.
        test.eval_inline("task!: &t1\n  title: \"old\"\n  done: false\n")
            .await?;
        let updated = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            Some("t1"),
            &["--title".into(), "new".into()],
        )
        .await?;
        assert!(updated.contains("current state:"), "{updated}");
        assert!(updated.contains("title: \"new\""), "{updated}");
        assert!(!updated.contains("title: \"old\""), "{updated}");
        let out = tonk_cli::data_ops::get(&test.site, "task", "t1", false).await?;
        assert!(
            out.contains("new") && !out.contains("old"),
            "set should overwrite:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_set_with_no_fields_supplied() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t1b\n  title: \"old\"\n  done: false\n")
            .await?;
        let err = tonk_cli::data_ops::assert_op(&test.site, "task", Some("t1b"), &[])
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("at least one"),
            "empty set should be rejected:\n{msg}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_a_single_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t2\n  title: \"retract-me\"\n  done: false\n")
            .await?;
        tonk_cli::data_ops::retract(&test.site, "task", "t2", Some("title"), Default::default())
            .await?;
        // After retracting its only declared field, the concept
        // query no longer matches it (a concept query requires
        // every field present).
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            !out.contains("retract-me"),
            "retracted field should drop the row from the concept query:\n{out}"
        );
        // The `list`/`get` check above can't tell a single-field
        // retraction apart from a whole-instance wipe — the
        // concept-completeness query drops the row either way.
        // Go around it and check the *other* field's raw claim
        // directly: it must survive a `title: _` retraction.
        let title_claims = select_claims(&test, "xyz.tonk.task/title").await?;
        assert!(
            title_claims.is_empty(),
            "retracted `title` field should leave no claim: {title_claims:?}"
        );
        let done_claims = select_claims(&test, "xyz.tonk.task/done").await?;
        assert_eq!(
            done_claims.len(),
            1,
            "untouched `done` field's claim must survive a single-field `title: _` retraction, got: {done_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_a_whole_instance() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t3\n  title: \"gone-entirely\"\n  done: false\n")
            .await?;
        tonk_cli::data_ops::retract(&test.site, "task", "t3", None, Default::default()).await?;
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            !out.contains("gone-entirely"),
            "whole-instance rm should drop the row:\n{out}"
        );
        // `..: _` must still take out every field, not just the
        // ones the retraction-target fix leaves alone for
        // per-field `_`.
        let title_claims = select_claims(&test, "xyz.tonk.task/title").await?;
        assert!(
            title_claims.is_empty(),
            "whole-instance rm should leave no `title` claim: {title_claims:?}"
        );
        let done_claims = select_claims(&test, "xyz.tonk.task/done").await?;
        assert!(
            done_claims.is_empty(),
            "whole-instance rm should leave no `done` claim: {done_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_enumerating_valid_fields_on_an_unknown_rm_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t4\n  title: \"x\"\n  done: false\n")
            .await?;
        let err =
            tonk_cli::data_ops::retract(&test.site, "task", "t4", Some("nope"), Default::default())
                .await
                .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("title"),
            "error should enumerate valid fields:\n{msg}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_superseding_a_nonexistent_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // No instance seeded: a typo'd (or missing) entity must not
        // silently mint a partial orphan — the validation-backdoor
        // test from the spec.
        let err = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            Some("no-such-task"),
            &["--title".into(), "x".into()],
        )
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no task instance at 'no-such-task'"),
            "supersede against a nonexistent entity must fail loudly:\n{msg}"
        );
        assert!(
            msg.contains("tonk query task"),
            "the error should point at `tonk query`:\n{msg}"
        );
        // And nothing landed: the branch has no task rows at all.
        let out = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(
            !out.contains("\"x\""),
            "no partial orphan may be minted:\n{out}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_hints_the_supersede_form_on_missing_required_fields() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // Mint form with a required field missing — the agent who
        // meant to supersede and forgot the entity lands here, so
        // the error must point at the right fix.
        let err = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            None,
            &["--title".into(), "only-title".into()],
        )
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("pass the entity before the flags"),
            "missing-required error should hint the supersede form:\n{msg}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_no_instance_before_no_fields_for_a_bad_entity() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        // A misplaced bare token with no flags at all: the entity
        // check must win, so the error names the real problem (no
        // such instance), not the missing flags.
        let err = tonk_cli::data_ops::assert_op(&test.site, "task", Some("ghost"), &[])
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no task instance at 'ghost'"),
            "a nonexistent entity should surface NoInstance even with zero flags:\n{msg}"
        );
        Ok(())
    }
}

// Dialog semantics on a many-cardinality attribute: an assert
// APPENDS a value; it does not supersede. These tests lock that
// behavior through the typed-flag surface — if the analyzer turns
// out not to append, the failure here is a finding to surface, not
// to paper over (see the spec's cardinality-many section).
mod when_asserting_many_cardinality_fields {
    use super::*;

    #[dialog_common::test]
    async fn it_appends_a_value_instead_of_superseding() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        // Anchor the mint through eval so the entity is addressable
        // by name across calls (the flag surface can't set anchors).
        test.eval_inline("note!: &n1\n  body: \"hello\"\n  tag: \"a\"\n")
            .await?;
        tonk_cli::data_ops::assert_op(
            &test.site,
            "note",
            Some("n1"),
            &["--tag".into(), "b".into()],
        )
        .await?;
        let tag_claims = select_claims(&test, "xyz.tonk.note/tag").await?;
        assert_eq!(
            tag_claims.len(),
            2,
            "asserting a second value on a many-cardinality field must append, got: {tag_claims:?}"
        );
        let body_claims = select_claims(&test, "xyz.tonk.note/body").await?;
        assert_eq!(
            body_claims.len(),
            1,
            "the untouched one-cardinality field must keep exactly one claim: {body_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retracts_every_value_of_a_many_field() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        test.eval_inline("note!: &n2\n  body: \"hello\"\n  tag: \"a\"\n  tag: \"b\"\n")
            .await?;
        tonk_cli::data_ops::retract(&test.site, "note", "n2", Some("tag"), Default::default())
            .await?;
        let tag_claims = select_claims(&test, "xyz.tonk.note/tag").await?;
        assert!(
            tag_claims.is_empty(),
            "retract --field on a many field must retract every value: {tag_claims:?}"
        );
        let body_claims = select_claims(&test, "xyz.tonk.note/body").await?;
        assert_eq!(
            body_claims.len(),
            1,
            "the sibling field must survive: {body_claims:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_marks_many_fields_in_the_dynamic_help() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(NOTE_ATTRIBUTE_DECL).await?;
        test.eval_inline(NOTE_CONCEPT_DECL).await?;
        let help =
            tonk_cli::data_ops::assert_op(&test.site, "note", None, &["--help".into()]).await?;
        assert!(
            help.contains("appends a value"),
            "many-cardinality fields should be marked in --help:\n{help}"
        );
        Ok(())
    }
}

mod when_previewing_a_write {
    use super::*;
    use tonk_cli::data_ops::WriteOptions;

    fn preview() -> WriteOptions {
        WriteOptions {
            notation: false,
            dry_run: true,
            ..Default::default()
        }
    }

    /// The whole promise of `--dry-run` is that the branch is where it was.
    /// Asserting on the revision rather than on a later read is what
    /// catches a write that landed and was then made invisible some other
    /// way.
    async fn revision(test: &TestSite) -> Result<String> {
        let session = test.site.branch().await?;
        Ok(format!("{:?}", session.handle().revision()))
    }

    #[dialog_common::test]
    async fn it_mints_nothing_when_asserting() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let before = revision(&test).await?;

        let out = tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            None,
            &[
                "--title".into(),
                "Never committed".into(),
                "--done".into(),
                "false".into(),
                "--dry-run".into(),
            ],
        )
        .await?;

        assert!(out.contains("dry run"), "{out}");
        assert_eq!(revision(&test).await?, before);
        let listed = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(!listed.contains("Never committed"), "{listed}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_the_instance_when_retracting() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline("task!: &t1\n  title: \"Still here\"\n  done: false\n")
            .await?;
        let before = revision(&test).await?;

        let out =
            tonk_cli::data_ops::retract(&test.site, "task", "t1", Some("title"), preview()).await?;

        assert!(out.contains("dry run"), "{out}");
        assert_eq!(revision(&test).await?, before);
        let listed = tonk_cli::data_ops::query(&test.site, "task", false).await?;
        assert!(listed.contains("Still here"), "{listed}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_declares_no_concept() -> Result<()> {
        let test = TestSite::new().await?;
        let before = revision(&test).await?;

        let out = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            preview(),
        )
        .await?;

        assert!(out.contains("dry run"), "{out}");
        assert_eq!(revision(&test).await?, before);
        // The name is still free, which is the observable consequence:
        // a previewed declaration that reserved it would make the real
        // command fail with "already exists".
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_authors_no_view_and_leaves_the_home_alone() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await?;
        let before = revision(&test).await?;

        let out = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{name}</b>",
            false,
            preview(),
        )
        .await?;

        assert!(out.contains("dry run"), "{out}");
        // `view_add` auto-surfaces onto an unset home, so a preview that
        // committed would show up here even if the view itself did not.
        assert_eq!(revision(&test).await?, before);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_repoints_no_home() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await?;
        let before = revision(&test).await?;

        let out = tonk_cli::data_ops::home(&test.site, &["habit".into()], preview()).await?;

        assert!(out.contains("dry run"), "{out}");
        assert_eq!(revision(&test).await?, before);
        Ok(())
    }

    /// A concept with a field named like one of the switches keeps its
    /// field: the schema is the thing that cannot be spelled another way.
    #[dialog_common::test]
    async fn it_lets_a_field_win_a_name_collision() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "run",
            &["quiet:text:one".into()],
            None,
            Default::default(),
        )
        .await?;

        tonk_cli::data_ops::assert_op(&test.site, "run", None, &["--quiet".into(), "yes".into()])
            .await?;

        let listed = tonk_cli::data_ops::query(&test.site, "run", false).await?;
        assert!(listed.contains("yes"), "{listed}");
        Ok(())
    }

    #[dialog_common::test]
    async fn a_field_beginning_with_q_does_not_steal_the_quiet_short_flag() -> Result<()> {
        let test = TestSite::new().await?;
        tonk_cli::data_ops::concept_add(
            &test.site,
            "survey",
            &["question:text:one".into()],
            None,
            Default::default(),
        )
        .await?;

        tonk_cli::data_ops::assert_op(
            &test.site,
            "survey",
            None,
            &[
                "--question".into(),
                "yes".into(),
                "--dry-run".into(),
                "--no-sync".into(),
                "-q".into(),
            ],
        )
        .await?;
        Ok(())
    }
}

mod when_printing_notation_for_a_write {
    use super::*;
    use tonk_cli::data_ops::WriteOptions;

    fn notation() -> WriteOptions {
        WriteOptions {
            notation: true,
            ..Default::default()
        }
    }

    async fn revision(test: &TestSite) -> Result<String> {
        let session = test.site.branch().await?;
        Ok(format!("{:?}", session.handle().revision()))
    }

    #[dialog_common::test]
    async fn every_macro_returns_its_document_without_changing_the_branch() -> Result<()> {
        let test = TestSite::new().await?;

        let before = revision(&test).await?;
        let concept = tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            notation(),
        )
        .await?;
        assert!(concept.contains("concept!: &habit"), "{concept}");
        assert_eq!(revision(&test).await?, before);

        tonk_cli::data_ops::concept_add(
            &test.site,
            "habit",
            &["name:text:one".into()],
            None,
            Default::default(),
        )
        .await?;

        let before = revision(&test).await?;
        let asserted = tonk_cli::data_ops::assert_op(
            &test.site,
            "habit",
            None,
            &["--name".into(), "Read".into(), "--notation".into()],
        )
        .await?;
        assert!(asserted.contains("habit!:"), "{asserted}");
        assert!(asserted.contains("name: \"Read\""), "{asserted}");
        assert_eq!(revision(&test).await?, before);

        let view = tonk_cli::data_ops::view_add(
            &test.site,
            "habit",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{name}</b>",
            false,
            notation(),
        )
        .await?;
        assert!(view.contains("view!:\n  this: habit"), "{view}");
        assert_eq!(revision(&test).await?, before);

        let home = tonk_cli::data_ops::home(&test.site, &["habit".into()], notation()).await?;
        assert!(home.contains("tonk/space"), "{home}");
        assert_eq!(revision(&test).await?, before);

        test.eval_inline("habit!: &reading\n  name: \"Read\"\n")
            .await?;
        let before = revision(&test).await?;
        let retracted =
            tonk_cli::data_ops::retract(&test.site, "habit", "reading", None, notation()).await?;
        assert!(retracted.contains("habit!:"), "{retracted}");
        assert!(retracted.contains("..: _"), "{retracted}");
        assert_eq!(revision(&test).await?, before);
        Ok(())
    }
}
