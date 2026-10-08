//! `tonk eval` with several files: they are evaluated in the order given
//! as one commit, a set of unchanged files commits nothing,
//! `!include/blob` stores referenced files as blobs in that commit, and
//! the push after the write pulls and pushes again when another writer
//! moved the upstream.

mod common;

use std::path::{Path, PathBuf};

use anyhow::Result;
use dialog_query::the;
use dialog_repository::Revision;
use tonk_cli::auto_sync::WriteSession;
use tonk_cli::eval::{self, EvalError, Options, Outcome, Source};
use tonk_cli::{blob, sync};

use crate::common::TestSite;

const SCHEMA: &str = r#"
attribute!: &page-title
  description: "page title"
  the:         xyz.example.page/title
  as:          text
  cardinality: one

attribute!: &page-hero
  description: "page hero image"
  the:         xyz.example.page/hero
  as:          entity
  cardinality: one

concept!: &page
  description: "a page"
  with:
    title: page-title
    hero:  page-hero

concept!: &heading
  description: "anything with a page title"
  with:
    title: page-title
"#;

const PAGE: &str = r#"
page!:
  this: id:page-home
  title: Home
  hero: !include/blob ../assets/hero.png
"#;

/// Lay out `site/00-schema.yaml`, `site/pages/home.yaml` and the asset
/// the page includes, and return the directory to evaluate.
fn site(test: &TestSite) -> Result<PathBuf> {
    let root = test.parent.join("site");
    std::fs::create_dir_all(root.join("pages"))?;
    std::fs::create_dir_all(root.join("assets"))?;
    std::fs::write(root.join("00-schema.yaml"), SCHEMA)?;
    std::fs::write(root.join("pages/home.yaml"), PAGE)?;
    std::fs::write(root.join("assets/hero.png"), b"not really a png")?;
    Ok(root)
}

/// The site's documents, schema first, then every page by name: the
/// order a caller such as `tonk eval 00-schema.yaml pages/*.yaml` gives.
fn documents(root: &Path) -> Result<Vec<Source>> {
    let mut pages: Vec<_> = std::fs::read_dir(root.join("pages"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    pages.sort();
    Ok(std::iter::once(root.join("00-schema.yaml"))
        .chain(pages)
        .map(Source::File)
        .collect())
}

/// Evaluate the site's documents the way `tonk eval` does with several
/// paths, syncing around the commit.
async fn publish(test: &TestSite, root: &Path) -> Result<Outcome> {
    let session = WriteSession::begin(&test.site, true).await;
    let outcome = eval::run_documents(&test.site, documents(root)?, Options::default()).await?;
    session.finish(outcome.committed).await;
    Ok(outcome)
}

/// Wire `main`'s upstream to a sibling branch in the same repo, the
/// in-process stand-in for a remote.
async fn wire_sibling_upstream(test: &TestSite) -> Result<()> {
    let upstream = upstream(test).await?;
    test.site
        .branch()
        .await?
        .handle()
        .set_upstream(&upstream)
        .perform(&test.site.operator)
        .await?;
    Ok(())
}

async fn upstream(test: &TestSite) -> Result<dialog_repository::Branch> {
    Ok(test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?)
}

async fn upstream_revision(test: &TestSite) -> Result<Option<Revision>> {
    Ok(upstream(test).await?.revision())
}

async fn local_revision(test: &TestSite) -> Result<Option<Revision>> {
    Ok(test.site.branch().await?.handle().revision())
}

mod when_evaluating_several_documents {
    use super::*;

    /// The page uses a concept the schema document declares, which only
    /// resolves because both are evaluated in one transaction.
    #[dialog_common::test]
    async fn it_commits_every_document_once_and_pushes() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;

        let outcome = publish(&test, &root).await?;
        assert!(outcome.committed);
        let after = local_revision(&test).await?.expect("committed");
        assert_eq!(
            after.edition,
            outcome.response.revision_after.unwrap().edition
        );
        assert_eq!(
            upstream_revision(&test).await?.map(|r| r.tree),
            Some(after.tree),
            "the upstream holds exactly what was evaluated"
        );

        let found = test
            .eval_inline("page:\n  this: id:page-home\n  title: ?title\n  hero: ?hero\n")
            .await?;
        assert!(found.stdout.contains("Home"), "{}", found.stdout);
        let blobs = blob::ls(&test.site).await?;
        assert_eq!(blobs.len(), 1);
        assert!(
            found.stdout.contains(blobs[0].entity.as_str()),
            "the page refers to the stored blob: {}",
            found.stdout
        );
        Ok(())
    }

    /// Order is the caller's: a page given before the schema that
    /// declares its concept cannot resolve it.
    #[dialog_common::test]
    async fn it_evaluates_documents_in_the_order_given() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;
        let mut reversed = documents(&root)?;
        reversed.reverse();

        let error = eval::run_documents(&test.site, reversed, Options::default())
            .await
            .unwrap_err();
        assert!(matches!(error, EvalError::Analyze(_)), "{error}");
        assert!(error.to_string().contains("home.yaml"), "{error}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_stores_included_blobs_with_their_metadata() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;

        publish(&test, &root).await?;

        let rows = blob::ls(&test.site).await?;
        let hero = rows
            .iter()
            .find(|row| row.name.as_deref() == Some("hero.png"))
            .expect("the included asset is listed as a blob");
        assert_eq!(hero.content_type.as_deref(), Some("image/png"));
        let mut bytes = Vec::new();
        blob::cat(&test.site, hero.entity.as_str(), &mut bytes).await?;
        assert_eq!(bytes, b"not really a png");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_commits_nothing_when_the_documents_are_unchanged() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        publish(&test, &root).await?;
        let local = local_revision(&test).await?;
        let pushed = upstream_revision(&test).await?;

        let again = publish(&test, &root).await?;
        assert_eq!(
            again.response.revision_after, again.response.revision_before,
            "re-asserting held facts mints no revision"
        );
        assert_eq!(local_revision(&test).await?, local);
        assert_eq!(upstream_revision(&test).await?, pushed);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_supersedes_an_edited_field() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;
        publish(&test, &root).await?;

        std::fs::write(
            root.join("pages/home.yaml"),
            PAGE.replace("Home", "Welcome"),
        )?;
        publish(&test, &root).await?;

        let found = test
            .eval_inline("page:\n  this: id:page-home\n  title: ?title\n  hero: ?hero\n")
            .await?;
        assert!(found.stdout.contains("Welcome"), "{}", found.stdout);
        assert!(
            !found.stdout.contains("title: Home"),
            "a one-cardinality field is superseded: {}",
            found.stdout
        );
        Ok(())
    }

    /// A dry run spans documents like a real one (the page resolves the
    /// schema's concept) and stores nothing, blobs included.
    #[dialog_common::test]
    async fn it_commits_nothing_on_a_dry_run() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;
        let before = local_revision(&test).await?;

        let outcome = eval::run_documents(
            &test.site,
            documents(&root)?,
            Options {
                dry_run: true,
                ..Options::default()
            },
        )
        .await?;
        assert!(!outcome.committed);
        assert_eq!(local_revision(&test).await?, before);
        assert!(blob::ls(&test.site).await?.is_empty());
        Ok(())
    }
}

mod when_a_document_is_rejected {
    use super::*;

    #[dialog_common::test]
    async fn it_names_the_document_and_commits_nothing() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        std::fs::write(
            root.join("pages/broken.yaml"),
            "page!:\n  this: id:broken\n  title: Broken\n  hero: !include/blob ../assets/missing.png\n",
        )?;
        let before = local_revision(&test).await?;

        let error = publish(&test, &root)
            .await
            .unwrap_err()
            .downcast::<EvalError>()?;
        assert!(matches!(error, EvalError::Parse(_)), "{error}");
        assert!(error.to_string().contains("broken.yaml"), "{error}");
        assert_eq!(
            local_revision(&test).await?,
            before,
            "the documents before it are not committed either"
        );
        assert!(upstream_revision(&test).await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_names_the_document_an_analysis_error_is_in() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;
        std::fs::write(
            root.join("pages/unknown.yaml"),
            "nonesuch!:\n  this: id:x\n  title: X\n",
        )?;

        let error = publish(&test, &root)
            .await
            .unwrap_err()
            .downcast::<EvalError>()?;
        assert!(matches!(error, EvalError::Analyze(_)), "{error}");
        assert!(error.to_string().contains("unknown.yaml"), "{error}");
        Ok(())
    }
}

mod when_the_upstream_moves_during_a_write {
    use super::*;

    /// Another writer advances the upstream after the write's pull: the
    /// first push is refused as a non-fast-forward, so the push after the
    /// write pulls, merging both writers' facts, and pushes again.
    #[dialog_common::test]
    async fn it_pulls_and_pushes_again() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        publish(&test, &root).await?;

        let session = WriteSession::begin(&test.site, true).await;
        let entity: dialog_artifacts::Entity = "id:page-elsewhere".parse()?;
        upstream(&test)
            .await?
            .transaction()
            .assert(
                the!("xyz.example.page/title")
                    .of(entity)
                    .is("Written elsewhere".to_owned()),
            )
            .commit()
            .publish()
            .perform(&test.site.operator)
            .await?;
        std::fs::write(
            root.join("pages/home.yaml"),
            PAGE.replace("Home", "Welcome"),
        )?;
        let outcome =
            eval::run_documents(&test.site, documents(&root)?, Options::default()).await?;
        assert!(
            matches!(
                sync::push(&test.site).await,
                Err(sync::SyncError::NonFastForward)
            ),
            "the moved upstream refuses a plain push"
        );

        let report = session.finish(outcome.committed).await;
        assert_eq!(report.receipt()["push"], "pushed", "{:?}", report.receipt());
        assert_eq!(
            upstream_revision(&test).await?.map(|r| r.tree),
            local_revision(&test).await?.map(|r| r.tree)
        );
        let found = test
            .eval_inline("heading:\n  this: ?page\n  title: ?title\n")
            .await?;
        assert!(found.stdout.contains("Welcome"), "{}", found.stdout);
        assert!(
            found.stdout.contains("Written elsewhere"),
            "{}",
            found.stdout
        );
        Ok(())
    }
}
