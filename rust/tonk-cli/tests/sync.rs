//! Auto-sync behaviour for `tonk eval`: a write reaches the
//! upstream on its own, and `--no-sync` suppresses it.

mod common;

use anyhow::Result;
use dialog_repository::Revision;
use tonk_cli::auto_sync;
use tonk_cli::eval::{Options, Source};
use tonk_cli::sync;
use tonk_schema::SyncState;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};

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
    let session = test.site.branch().await?;
    session
        .handle()
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

mod when_checking_for_an_upstream {
    use super::*;
    use tonk_cli::remote;

    #[dialog_common::test]
    async fn it_reports_whether_main_tracks_an_upstream() -> Result<()> {
        let test = TestSite::new().await?;
        assert!(
            !remote::upstream_configured(&test.site).await?,
            "a fresh site has no upstream"
        );
        wire_sibling_upstream(&test).await?;
        assert!(
            remote::upstream_configured(&test.site).await?,
            "wiring an upstream flips the check"
        );
        Ok(())
    }
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
        let session = test.site.branch().await?;
        let local = session.handle().revision().expect("eval committed locally");
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

mod when_asserting_with_an_upstream {
    use super::*;
    use dialog_query::{Output as _, Query, Term};
    use tonk_cli::agents;
    use tonk_schema::RepositoryAgents;

    #[dialog_common::test]
    async fn it_auto_pushes_an_assert_to_the_upstream() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let before = upstream_revision(&test).await?;

        tonk_cli::data_ops::assert_op(
            &test.site,
            "task",
            None,
            &[
                "--title".into(),
                "synced".into(),
                "--done".into(),
                "false".into(),
            ],
        )
        .await?;

        let after = upstream_revision(&test).await?;
        assert_ne!(
            before, after,
            "a committing assert must push to the upstream like eval does"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_carries_agent_context_on_the_synced_content_branch() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        let expected = "# Shared space context\n";

        let stored = agents::set(&test.site, expected, true).await?;
        let upstream = test
            .site
            .repository
            .branch("upstream")
            .open()
            .perform(&test.site.operator)
            .await?;
        let claims: Vec<RepositoryAgents> = upstream
            .query()
            .select(Query::<RepositoryAgents> {
                this: Term::var("this"),
                agents: Term::var("agents"),
            })
            .perform(&test.site.operator)
            .try_vec()
            .await?;

        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].this.to_string(), stored.entity);
        assert_eq!(claims[0].agents.0, expected);
        Ok(())
    }
}

mod when_minting_an_invite {
    use super::*;
    use tonk_cli::invite;

    #[dialog_common::test]
    async fn it_pushes_local_state_to_the_upstream_before_minting() -> Result<()> {
        let test = TestSite::new().await?;
        wire_sibling_upstream(&test).await?;
        // Commit something locally that has not been pushed, mirroring the
        // stdlib seed sitting unpushed on a freshly-init'd repo.
        test.eval_inline(ATTRIBUTE_DECL).await?;
        assert!(
            upstream_revision(&test).await?.is_none(),
            "upstream starts empty — the local commit has not been pushed yet"
        );

        // Minting a local-only invite (no embedded remote URL) must still
        // push, because the branch has an upstream.
        invite::mint(&test.site, None, None).await?;

        assert!(
            upstream_revision(&test).await?.is_some(),
            "mint must push the unpushed local state to the upstream"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_is_a_noop_push_when_no_upstream_is_configured() -> Result<()> {
        // No upstream wired: mint must still succeed (local-only invite),
        // not error trying to push.
        let test = TestSite::new().await?;
        let outcome = invite::mint(&test.site, None, None).await?;
        assert!(
            !outcome.url.is_empty(),
            "a local-only invite still mints a URL"
        );
        Ok(())
    }
}

mod when_claiming_an_invite {
    use super::*;
    use tonk_cli::inventory::{self, SpaceRole};
    use tonk_cli::invite;
    use tonk_cli::site::TonkSite;

    /// The claim's roster row has to land where the roster is read: on the
    /// content branch. On `meta` it would never sync — the owner would never
    /// see the member, and this device's own listing would show a space it
    /// legitimately joined as one whose roster holds no row of ours.
    #[dialog_common::test]
    async fn it_records_the_membership_where_the_roster_is_read() -> Result<()> {
        let inviter = TestSite::new().await?;
        let url = invite::mint(&inviter.site, None, None).await?.url;

        // A separate parent means a separate profile: claiming with the
        // inviter's own profile would be a self-claim, which is a different
        // path.
        let joiner = tempfile::tempdir()?;
        let joiner_parent = joiner.path().canonicalize()?;
        let joiner_config = common::isolated_config(&joiner_parent)?;
        let root = joiner_parent.join("joined");
        invite::claim(&root, &url, joiner_config.clone()).await?;

        let joined = TonkSite::open_with(&root, joiner_config).await?;
        let roster = inventory::read_roster(&joined).await?;

        assert!(roster.notes.is_empty(), "{:?}", roster.notes);
        let row = roster
            .members
            .first()
            .expect("the claim writes this member's roster row");
        assert_eq!(row.role.as_deref(), Some(tonk_schema::MemberRole::MEMBER));
        assert_eq!(
            inventory::role_for_site(&joined).await?,
            SpaceRole::Member,
            "a joined space reads as one this device is a member of"
        );
        Ok(())
    }

    /// `MemberRole` is cardinality-one on the membership entity, so a claim
    /// that asserted `member` unconditionally would demote whoever the row
    /// already names — including a founder claiming an invite to their own
    /// space, which is reachable because one profile is shared across every
    /// site on a machine.
    #[dialog_common::test]
    async fn it_leaves_an_existing_role_alone() -> Result<()> {
        let inviter = TestSite::new().await?;
        let url = invite::mint(&inviter.site, None, None).await?.url;

        let joiner = tempfile::tempdir()?;
        let joiner_parent = joiner.path().canonicalize()?;
        let joiner_config = common::isolated_config(&joiner_parent)?;
        let root = joiner_parent.join("joined");
        invite::claim(&root, &url, joiner_config.clone()).await?;

        // Promote the claimed row, then claim the same invite again into a
        // second site backed by the same profile.
        let joined = TonkSite::open_with(&root, joiner_config.clone()).await?;
        let membership = tonk_schema::Membership::new(
            inventory::read_roster(&joined).await?.members[0]
                .did
                .parse()?,
            joined.repository.did(),
        );
        let session = joined.branch().await?;
        session
            .handle()
            .transaction()
            .assert(tonk_schema::MemberRole::founder(membership.this().clone()))
            .commit()
            .perform(&joined.operator)
            .await?;
        drop(session);

        let again = joiner_parent.join("rejoined");
        invite::claim(&again, &url, joiner_config).await?;

        let roster = inventory::read_roster(&joined).await?;
        assert_eq!(
            roster.members[0].role.as_deref(),
            Some(tonk_schema::MemberRole::FOUNDER),
            "a second claim must not overwrite the role already on the row"
        );
        Ok(())
    }
}

mod when_reporting_status {
    use super::*;

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
