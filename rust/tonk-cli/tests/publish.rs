//! `tonk publish`: a directory of notation documents is asserted in
//! path order and delivered once, an unchanged directory publishes
//! nothing, `!include/blob` stores referenced files as blobs, and a
//! push that finds the upstream moved pulls and pushes again.

mod common;

use std::path::Path;

use anyhow::Result;
use dialog_query::the;
use dialog_repository::Revision;
use tonk_cli::publish::{self, Options, PublishError};
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
/// the page includes, and return the directory to publish.
fn site(test: &TestSite) -> Result<std::path::PathBuf> {
    let root = test.parent.join("site");
    std::fs::create_dir_all(root.join("pages"))?;
    std::fs::create_dir_all(root.join("assets"))?;
    std::fs::write(root.join("00-schema.yaml"), SCHEMA)?;
    std::fs::write(root.join("pages/home.yaml"), PAGE)?;
    std::fs::write(root.join("assets/hero.png"), b"not really a png")?;
    Ok(root)
}

/// Wire `main`'s upstream to a sibling branch in the same repo, the
/// in-process stand-in for a remote.
async fn wire_sibling_upstream(test: &TestSite) -> Result<()> {
    let upstream = test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?;
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

mod when_publishing_a_directory {
    use super::*;

    #[dialog_common::test]
    async fn it_evaluates_documents_in_path_order_skipping_hidden_entries() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;
        std::fs::create_dir_all(root.join(".github"))?;
        std::fs::write(root.join(".github/ci.yml"), "not: notation\n")?;
        std::fs::write(root.join("README.md"), "not notation either")?;

        let found = publish::documents(&root)?;
        let relative: Vec<_> = found
            .iter()
            .map(|path| path.strip_prefix(&root).unwrap().to_path_buf())
            .collect();
        assert_eq!(
            relative,
            vec![
                Path::new("00-schema.yaml").to_path_buf(),
                Path::new("pages/home.yaml").to_path_buf()
            ]
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_asserts_every_document_and_pushes_once() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;

        let outcome = publish::run(&test.site, &root, Options::default()).await?;
        assert!(outcome.changed, "a fresh space gains the documents' facts");
        assert!(outcome.pushed, "the commit reaches the upstream");
        assert_eq!(outcome.attempts, 1);
        assert_eq!(outcome.documents.len(), 2);

        let local = test.site.branch().await?.handle().revision();
        assert_eq!(
            upstream_revision(&test).await?.map(|r| r.tree),
            local.map(|r| r.tree),
            "the upstream holds exactly what was published"
        );

        let found = test
            .eval_inline("page:\n  this: id:page-home\n  title: ?title\n  hero: ?hero\n")
            .await?;
        assert!(found.stdout.contains("Home"), "{}", found.stdout);
        let hero = blob::ls(&test.site).await?;
        assert_eq!(hero.len(), 1);
        assert!(
            found.stdout.contains(hero[0].entity.as_str()),
            "the page refers to the stored blob: {}",
            found.stdout
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_stores_included_blobs_with_their_metadata() -> Result<()> {
        let test = TestSite::new().await?;
        let root = site(&test)?;

        publish::run(&test.site, &root, Options::default()).await?;

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
    async fn it_publishes_nothing_when_the_directory_is_unchanged() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        publish::run(&test.site, &root, Options::default()).await?;
        let before = upstream_revision(&test).await?;

        let again = publish::run(&test.site, &root, Options::default()).await?;
        assert!(!again.changed, "re-asserting held facts mints no revision");
        assert!(!again.pushed, "and so there is nothing to deliver");
        assert_eq!(again.attempts, 0);
        assert_eq!(upstream_revision(&test).await?, before);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_publishes_an_edited_document() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        publish::run(&test.site, &root, Options::default()).await?;

        std::fs::write(
            root.join("pages/home.yaml"),
            PAGE.replace("Home", "Welcome"),
        )?;
        let edited = publish::run(&test.site, &root, Options::default()).await?;
        assert!(edited.changed && edited.pushed);

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

    #[dialog_common::test]
    async fn it_commits_nothing_on_a_dry_run() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let root = site(&test)?;
        let before = test.site.branch().await?.handle().revision();

        let outcome = publish::run(
            &test.site,
            &root,
            Options {
                dry_run: true,
                ..Options::default()
            },
        )
        .await?;
        assert!(!outcome.changed && !outcome.pushed);
        assert_eq!(test.site.branch().await?.handle().revision(), before);
        assert!(upstream_revision(&test).await?.is_none());
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
            "page!:\n  this: id:broken\n  hero: !include/blob ../assets/missing.png\n",
        )?;

        let before = test.site.branch().await?.handle().revision();

        let error = publish::run(&test.site, &root, Options::default())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, PublishError::Document { path, .. } if path.ends_with("pages/broken.yaml")),
            "{error}"
        );
        assert_eq!(
            test.site.branch().await?.handle().revision(),
            before,
            "the documents before it are not committed either"
        );
        assert!(upstream_revision(&test).await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_directory_without_documents() -> Result<()> {
        let test = TestSite::new().await?;
        let root = test.parent.join("empty");
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("notes.md"), "no notation here")?;

        let error = publish::run(&test.site, &root, Options::default())
            .await
            .unwrap_err();
        assert!(matches!(error, PublishError::Empty(_)), "{error}");
        Ok(())
    }
}

mod when_the_upstream_moved {
    use super::*;

    /// Publish, then let another writer advance the upstream while this
    /// replica commits an edit of its own without pulling, so the two
    /// have diverged.
    async fn diverge(test: &TestSite) -> Result<()> {
        wire_sibling_upstream(test).await?;
        let root = site(test)?;
        publish::run(&test.site, &root, Options::default()).await?;

        let entity: dialog_artifacts::Entity = "id:page-elsewhere".parse()?;
        upstream(test)
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
        tonk_cli::eval::run_against_site(
            &test.site,
            tonk_cli::eval::Source::File(root.join("pages/home.yaml")),
            tonk_cli::eval::Options::default(),
        )
        .await?;
        Ok(())
    }

    /// The first push is refused as a non-fast-forward; delivery pulls,
    /// merging both writers' facts, and the second push lands.
    #[dialog_common::test]
    async fn it_pulls_and_pushes_again() -> Result<()> {
        let test = TestSite::new().await?;
        diverge(&test).await?;
        assert!(
            matches!(
                sync::push(&test.site).await,
                Err(sync::SyncError::NonFastForward)
            ),
            "the moved upstream refuses a plain push"
        );

        let (pushed, attempts) = publish::deliver(&test.site, 3).await?;
        assert!(pushed);
        assert_eq!(attempts, 2, "one refused push, then a pull and a push");

        let local = test.site.branch().await?.handle().revision();
        assert_eq!(
            upstream_revision(&test).await?.map(|r| r.tree),
            local.map(|r| r.tree)
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

    #[dialog_common::test]
    async fn it_gives_up_after_the_attempts_it_was_given() -> Result<()> {
        let test = TestSite::new().await?;
        diverge(&test).await?;
        let before = upstream_revision(&test).await?;

        let error = publish::deliver(&test.site, 1).await.unwrap_err();
        assert!(matches!(error, PublishError::Contended(1)), "{error}");
        assert_eq!(upstream_revision(&test).await?, before);
        Ok(())
    }
}
