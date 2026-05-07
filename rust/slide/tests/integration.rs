//! Integration tests for slide's library API.
//!
//! Each test sets up an isolated `.tonk/` site under a fresh
//! temp directory, redirects the profile dir into the same
//! temp tree, and exercises [`slide::eval::run_against_site`]
//! end-to-end.

use std::path::PathBuf;

use anyhow::Result;
use dialog_effects::storage::Directory;
use slide::eval::{self, Source};
use slide::output::Format;
use slide::site::{SiteConfig, SlideSite};
use tempfile::TempDir;

/// Build a fresh site under a tempdir with both the profile
/// directory and the `.tonk/` rooted in that tempdir, so two
/// concurrent tests don't trip over each other.
struct TestSite {
    site: SlideSite,
    // Holding the TempDir keeps the directory alive — Drop
    // removes it at the end of the test.
    _tmp: TempDir,
}

impl TestSite {
    async fn new() -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let cwd: PathBuf = tmp.path().to_path_buf();
        let profile_dir: PathBuf = tmp.path().join("_profile");
        std::fs::create_dir_all(&profile_dir)?;

        let suffix: u64 = rand::random();
        let config = SiteConfig {
            profile_name: format!("slide-test-{suffix:x}"),
            profile_directory: Directory::At(profile_dir.to_string_lossy().into_owned()),
        };

        let site = SlideSite::init_with(&cwd, config).await?;
        Ok(Self { site, _tmp: tmp })
    }

    async fn eval_inline(&self, doc: &str) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(
            &self.site,
            Source::Inline(doc.to_string()),
            eval::Options::default(),
        )
        .await
    }

    async fn eval_inline_with(
        &self,
        doc: &str,
        options: eval::Options,
    ) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(&self.site, Source::Inline(doc.to_string()), options).await
    }
}

const ATTRIBUTE_DECL: &str = r#"
attribute! task-title:
  description: "task title"
  the:         xyz.tonk.task/title
  as:          Text
  cardinality: one

attribute! task-done:
  description: "task done flag"
  the:         xyz.tonk.task/done
  as:          Boolean
  cardinality: one
"#;

const CONCEPT_DECL: &str = r#"
concept! task:
  description: "a task"
  with:
    title: .task-title
    done:  .task-done
"#;

#[tokio::test]
async fn init_idempotent_on_existing_site() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let suffix: u64 = rand::random();
    let profile_dir = tmp.path().join("_profile");
    std::fs::create_dir_all(&profile_dir)?;
    let config = SiteConfig {
        profile_name: format!("slide-test-{suffix:x}"),
        profile_directory: Directory::At(profile_dir.to_string_lossy().into_owned()),
    };

    let first = SlideSite::init_with(tmp.path(), config.clone()).await?;
    let did_first = first.repository.did();

    let second = SlideSite::init_with(tmp.path(), config).await?;
    assert_eq!(did_first, second.repository.did());
    Ok(())
}

#[tokio::test]
async fn attribute_declaration_lands_on_branch() -> Result<()> {
    let test = TestSite::new().await?;
    let outcome = test.eval_inline(ATTRIBUTE_DECL).await?;
    assert!(outcome.committed, "attribute decl should commit");
    assert!(outcome.response.commits.claims > 0, "claims emitted");

    // Re-querying via the built-in `attribute` concept must
    // surface both new entities by name.
    let query = test
        .eval_inline("attribute ?a:\n  id: \"xyz.tonk.task/title\"\n")
        .await?;
    assert!(
        !query.response.matches_after.is_empty()
            && !query.response.matches_after[0].results.is_empty(),
        "attribute should be queryable: {:#?}",
        query.response.matches_after
    );
    Ok(())
}

#[tokio::test]
async fn concept_declaration_round_trips() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    let outcome = test.eval_inline(CONCEPT_DECL).await?;
    assert!(outcome.committed);

    // Concept-of-concept query: ask for every concept by name.
    let query = test.eval_inline("concept ?c:\n  name: \"task\"\n").await?;
    assert!(
        !query.response.matches_after.is_empty()
            && !query.response.matches_after[0].results.is_empty(),
        "concept should resolve: {:#?}",
        query.response.matches_after
    );
    Ok(())
}

#[tokio::test]
async fn assert_then_query_round_trip() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;

    test.eval_inline(
        r#"
task! buy-milk:
  title: "Buy milk"
  done:  false
"#,
    )
    .await?;

    let query = test.eval_inline("task ?t:\n  done: false\n").await?;
    assert_eq!(query.response.matches_after.len(), 1);
    let block = &query.response.matches_after[0];
    assert_eq!(block.label, "task");
    assert_eq!(block.results.len(), 1, "should match buy-milk");
    let row = &block.results[0];
    assert_eq!(
        row.fields.get("title"),
        Some(&serde_json::json!("Buy milk"))
    );
    assert_eq!(row.fields.get("done"), Some(&serde_json::json!(false)));
    Ok(())
}

#[tokio::test]
async fn multi_expression_join_filters_correctly() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    test.eval_inline(
        r#"
task! a:
  title: "A"
  done:  true

task! b:
  title: "B"
  done:  false
"#,
    )
    .await?;

    // Two expressions sharing `?t` natural-join: only the row
    // with done=true and title=? should come back.
    let query = test
        .eval_inline(
            r#"
task ?t:
  done: true

task ?t:
  title: ?title
"#,
        )
        .await?;

    let total: usize = query
        .response
        .matches_after
        .iter()
        .map(|b| b.results.len())
        .sum();
    assert!(total >= 1, "join should return at least one row");
    // Only the done=true task ("A") satisfies both expressions.
    let titles: Vec<_> = query
        .response
        .matches_after
        .iter()
        .flat_map(|b| b.results.iter())
        .filter_map(|r| r.fields.get("title").cloned())
        .collect();
    assert!(
        titles.iter().any(|v| v == &serde_json::json!("A")),
        "expected A in titles, got {titles:?}"
    );
    assert!(
        !titles.iter().any(|v| v == &serde_json::json!("B")),
        "B should not appear: {titles:?}"
    );
    Ok(())
}

#[tokio::test]
async fn retraction_by_query_result_dissociates() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    test.eval_inline(
        r#"
task! a:
  title: "A"
  done:  false
"#,
    )
    .await?;

    // Retract task `a`'s entire projection: query binds
    // `?t` to entities matching title="A", then the second
    // expression dissociates the whole concept on each.
    test.eval_inline(
        r#"
task ?t:
  title: "A"

task! ?t: _
"#,
    )
    .await?;

    let after = test.eval_inline("task ?t:\n").await?;
    let total: usize = after
        .response
        .matches_after
        .iter()
        .map(|b| b.results.len())
        .sum();
    assert_eq!(total, 0, "task should be retracted");
    Ok(())
}

#[tokio::test]
async fn parse_error_yields_parse_exit_code() -> Result<()> {
    let test = TestSite::new().await?;
    let err = test
        .eval_inline("attribute! foo: as: Text\n  bad: indent\n")
        .await
        .expect_err("parse should fail");
    assert_eq!(err.exit_code(), slide::ExitCode::ParseError);
    Ok(())
}

#[tokio::test]
async fn analyze_error_yields_analyze_exit_code() -> Result<()> {
    let test = TestSite::new().await?;
    // `nope` isn't a known concept and the document defines none.
    let err = test
        .eval_inline("nope ?x:\n")
        .await
        .expect_err("analyze should fail");
    assert_eq!(err.exit_code(), slide::ExitCode::AnalyzeError);
    Ok(())
}

#[tokio::test]
async fn notation_output_round_trips_through_eval() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    test.eval_inline(
        r#"
task! ax:
  title: "Buy milk"
  done:  false
"#,
    )
    .await?;

    // Render a query result, then resubmit the matches section
    // verbatim — it must parse and analyze (re-asserting
    // identical bodies is a no-op at the dialog layer).
    let outcome = test
        .eval_inline_with(
            "task ?t:\n",
            eval::Options {
                format: Format::Notation,
                quiet: false,
            },
        )
        .await?;
    let split: Vec<&str> = outcome.stdout.splitn(2, "---\n").collect();
    assert_eq!(
        split.len(),
        2,
        "stdout had no matches section: {}",
        outcome.stdout
    );
    let matches_section = split[1];

    // Reissuing the matches section as a query: head form
    // `task did:key:…:` is a valid notation query expression.
    let resubmitted = test.eval_inline(matches_section).await?;
    assert!(
        !resubmitted.response.matches_after.is_empty(),
        "resubmitted output failed to surface results:\n{matches_section}\n\n{resubmitted:#?}"
    );
    Ok(())
}
