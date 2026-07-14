//! Shared fixtures for tonk's integration tests. Each test
//! lands its own tempdir-rooted site with a unique profile name
//! so parallel runs don't trip over the user's real
//! `~/Library/Application Support/dialog/` profile or each other.

#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;
use dialog_effects::storage::Directory;
use tempfile::TempDir;
use tonk_cli::eval::{self, Source};
use tonk_cli::site::{SiteConfig, TonkSite};

pub struct TestSite {
    pub site: TonkSite,
    pub config: SiteConfig,
    pub parent: PathBuf,
    pub tmp: TempDir,
}

impl TestSite {
    pub async fn new() -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = isolated_config(&parent)?;
        let site = TonkSite::init_with(&parent, config.clone()).await?;
        Ok(Self {
            site,
            config,
            parent,
            tmp,
        })
    }

    pub async fn eval_inline(&self, doc: &str) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(
            &self.site,
            Source::Inline(doc.to_string()),
            eval::Options::default(),
        )
        .await
    }

    pub async fn eval_inline_with(
        &self,
        doc: &str,
        options: eval::Options,
    ) -> Result<eval::Outcome, eval::EvalError> {
        eval::run_against_site(&self.site, Source::Inline(doc.to_string()), options).await
    }
}

/// Build a [`SiteConfig`] whose profile lives entirely inside
/// `parent` so the test never touches the user's data dir.
pub fn isolated_config(parent: &std::path::Path) -> Result<SiteConfig> {
    let profile_dir = parent.join("_profile");
    std::fs::create_dir_all(&profile_dir)?;
    let suffix: u64 = rand::random();
    Ok(SiteConfig {
        profile_name: format!("tonk-test-{suffix:x}"),
        profile_directory: Directory::At(profile_dir.to_string_lossy().into_owned()),
    })
}

/// Notation declaration of the `task-title` and `task-done`
/// attributes — used by every test that needs a task schema.
pub const ATTRIBUTE_DECL: &str = r#"
attribute!: &task-title
  description: "task title"
  the:         xyz.tonk.task/title
  as:          text
  cardinality: one

attribute!: &task-done
  description: "task done flag"
  the:         xyz.tonk.task/done
  as:          boolean
  cardinality: one
"#;

/// Notation declaration of a `task` concept referencing the
/// attributes above.
pub const CONCEPT_DECL: &str = r#"
concept!: &task
  description: "a task"
  with:
    title: task-title
    done:  task-done
"#;

/// Attributes for the cardinality-many lock tests: a one-cardinality
/// `body` and a many-cardinality `tag`.
pub const NOTE_ATTRIBUTE_DECL: &str = r#"
attribute!: &note-body
  description: "note body"
  the:         xyz.tonk.note/body
  as:          text
  cardinality: one

attribute!: &note-tag
  description: "a tag on a note"
  the:         xyz.tonk.note/tag
  as:          text
  cardinality: many
"#;

/// A `note` concept referencing the attributes above — one required
/// one-cardinality field plus one required many-cardinality field.
pub const NOTE_CONCEPT_DECL: &str = r#"
concept!: &note
  description: "a tagged note"
  with:
    body: note-body
    tag:  note-tag
"#;

/// Seed schema for HTML views: a `text/html`-bound attribute and
/// a `view` concept that uses it. Pasted at the top of any test
/// that needs `view!` heads. Shared with [`tonk_cli::guide`] so the
/// agent-facing reference matches what the tests exercise.
pub const VIEW_DECL: &str = r#"
attribute!: &html-body
  description: "HTML body of a tonk-authored view"
  the:         "text/html"
  as:          text
  cardinality: many

concept!: &view
  description: "An HTML view, served via the host route"
  with:
    body: html-body
"#;
