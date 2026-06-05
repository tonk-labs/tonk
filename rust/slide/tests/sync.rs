//! Auto-sync behaviour for `slide eval`: a write reaches the
//! upstream on its own, and `--no-sync` suppresses it.

mod common;

use anyhow::Result;
use dialog_repository::Revision;
use slide::auto_sync;
use slide::eval::{Options, Source};
use slide::sync;
use tonk_schema::SyncState;

use crate::common::{ATTRIBUTE_DECL, TestSite};

/// Wire `main`'s upstream to a sibling branch in the same repo —
/// the cheap in-process stand-in for a real remote, so a push has
/// somewhere local to land.
async fn wire_sibling_upstream(test: &TestSite) -> Result<()> {
    let upstream = test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?;
    test.site
        .branch
        .set_upstream(&upstream)
        .perform(&test.site.operator)
        .await?;
    Ok(())
}

/// Read the sibling upstream branch's current head by re-opening it.
async fn upstream_revision(test: &TestSite) -> Result<Option<Revision>> {
    let upstream = test
        .site
        .repository
        .branch("upstream")
        .open()
        .perform(&test.site.operator)
        .await?;
    Ok(upstream.revision())
}

mod when_evaluating_with_an_upstream {
    use super::*;

    #[dialog_common::test]
    async fn it_auto_pushes_the_commit_to_the_upstream() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        assert!(
            upstream_revision(&test).await?.is_none(),
            "upstream starts empty"
        );

        auto_sync::run_eval(
            &test.site,
            Source::Inline(ATTRIBUTE_DECL.to_string()),
            Options::default(),
            true,
        )
        .await?;

        let pushed = upstream_revision(&test)
            .await?
            .expect("auto-sync should have pushed the commit");
        let local = test.site.branch.revision().expect("eval committed locally");
        assert_eq!(
            pushed.tree, local.tree,
            "the upstream head matches the local head after auto-push"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_push_when_sync_is_disabled() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;

        auto_sync::run_eval(
            &test.site,
            Source::Inline(ATTRIBUTE_DECL.to_string()),
            Options::default(),
            false,
        )
        .await?;

        assert!(
            upstream_revision(&test).await?.is_none(),
            "nothing reaches the upstream when sync is disabled"
        );
        Ok(())
    }
}

mod when_reporting_status {
    use super::*;
    use crate::common::CONCEPT_DECL;

    #[dialog_common::test]
    async fn it_reports_no_upstream_when_none_is_configured() -> Result<()> {
        let test = TestSite::new().await?;
        assert_eq!(sync::status(&test.site).await?, SyncState::NoUpstream);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_synced_after_pushing() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        sync::push(&test.site).await?;
        assert_eq!(sync::status(&test.site).await?, SyncState::Synced);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_ahead_when_local_has_unpushed_commits() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        // Establish a shared base on the upstream, then advance the
        // local branch past it.
        test.eval_inline(ATTRIBUTE_DECL).await?;
        sync::push(&test.site).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        assert_eq!(sync::status(&test.site).await?, SyncState::Ahead);
        Ok(())
    }
}
