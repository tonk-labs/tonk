//! Behavioural tests for the site lifecycle:
//! initialization, profile identity, and migration from carry.

mod common;

mod when_initializing_a_site {
    use anyhow::Result;
    use slide::site::SlideSite;

    use crate::common;

    #[dialog_common::test]
    async fn it_creates_a_dialog_repository_with_a_stable_did() -> Result<()> {
        let test = common::TestSite::new().await?;
        // The repo's DID is a real ed25519 key, not the empty
        // string — running `slide init` produced a usable repo
        // backed by the on-disk `.tonk/` directory.
        let did = test.site.repository.did().to_string();
        assert!(did.starts_with("did:key:"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_is_idempotent_when_a_site_already_exists() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        let first = SlideSite::init_with(&parent, config.clone()).await?;
        let did_first = first.repository.did();

        let second = SlideSite::init_with(&parent, config).await?;
        // Re-running init returns the same repository — the
        // user can call it defensively at the start of any task.
        assert_eq!(did_first, second.repository.did());
        Ok(())
    }
}

mod when_managing_remotes {
    use anyhow::Result;
    use slide::remote::{self, RemoteError};

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";

    #[dialog_common::test]
    async fn it_registers_a_remote_and_lists_it_back() -> Result<()> {
        let test = common::TestSite::new().await?;
        let outcome = remote::add(&test.site, "origin", ENDPOINT, None).await?;
        assert_eq!(outcome.name, "origin");
        assert_eq!(outcome.endpoint, ENDPOINT);
        // Default subject == local repo's DID.
        assert_eq!(outcome.subject, test.site.repository.did());

        let listed = remote::list(&test.site).await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "origin");
        assert_eq!(listed[0].endpoint, ENDPOINT);
        assert_eq!(listed[0].subject, test.site.repository.did());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_finds_a_remote_by_name() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        let found = remote::find(&test.site, "origin").await?;
        assert!(found.is_some());
        assert_eq!(found.unwrap().endpoint, ENDPOINT);

        let missing = remote::find(&test.site, "nope").await?;
        assert!(missing.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_sets_main_upstream_to_the_remote_branch() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        let outcome = remote::set_upstream(&test.site, "origin").await?;
        assert_eq!(outcome.local_branch, "main");
        assert_eq!(outcome.remote, "origin");
        assert_eq!(outcome.remote_branch, "main");

        // Dialog now reports an upstream on the local main.
        assert!(test.site.branch.upstream().is_some());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_setting_upstream_for_an_unknown_remote() -> Result<()> {
        let test = common::TestSite::new().await?;
        let result = remote::set_upstream(&test.site, "missing").await;
        match result {
            Err(RemoteError::UnknownRemote(name)) => {
                assert_eq!(name, "missing");
            }
            other => panic!("expected UnknownRemote, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_lists_nothing_on_a_fresh_site() -> Result<()> {
        let test = common::TestSite::new().await?;
        let listed = remote::list(&test.site).await?;
        assert!(listed.is_empty());
        Ok(())
    }
}

mod when_claiming_an_invite_with_a_remote {
    use anyhow::Result;
    use slide::invite;
    use slide::remote;
    use slide::site::{SITE_DIRNAME, SlideSite};

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";

    #[dialog_common::test]
    async fn it_auto_configures_the_embedded_remote_on_the_joined_site() -> Result<()> {
        // Inviter site has a remote registered, mints an invite
        // referencing it.
        let inviter = common::TestSite::new().await?;
        remote::add(&inviter.site, "origin", ENDPOINT, None).await?;
        let invite_outcome = invite::mint(&inviter.site, None, Some(ENDPOINT)).await?;
        assert!(invite_outcome.url.contains("remote="));

        // Claimer joins into a fresh tempdir.
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_parent, &invite_outcome.url, claimer_config.clone()).await?;

        // Claim returned the embedded URL and surfaced the
        // auto-configured remote name.
        assert!(claim_outcome.remote_url.is_some());
        assert_eq!(
            claim_outcome.auto_configured_remote.as_deref(),
            Some("origin")
        );

        // Open the joined site and assert the remote landed in
        // its meta branch and main's upstream is wired.
        let joined =
            SlideSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "origin");
        assert_eq!(listed[0].endpoint, ENDPOINT);
        // Subject is the inviter's DID — slide holds delegated
        // authority on the inviter's repo.
        assert_eq!(listed[0].subject, inviter.site.repository.did());
        assert!(joined.branch.upstream().is_some());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_skips_remote_setup_when_invite_has_no_remote() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;
        assert!(!invite_outcome.url.contains("remote="));

        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_parent, &invite_outcome.url, claimer_config.clone()).await?;

        assert!(claim_outcome.auto_configured_remote.is_none());
        let joined =
            SlideSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert!(listed.is_empty());
        Ok(())
    }
}

mod when_minting_and_claiming_an_invite {
    use anyhow::Result;
    use slide::invite::{self, InviteError};
    use slide::site::{self, SITE_DIRNAME, SlideSite};

    use crate::common;

    #[dialog_common::test]
    async fn it_round_trips_an_invite_between_two_sites() -> Result<()> {
        // Inviter: a fully initialised slide site.
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;
        // Subject DID matches the inviter's repo, audience DID
        // is the freshly minted ephemeral signer's.
        assert_eq!(invite_outcome.subject, inviter.site.repository.did());
        assert!(invite_outcome.url.contains("?access="));
        assert!(
            invite_outcome.url.contains('#'),
            "audience-open invites carry the seed in the URL fragment",
        );

        // Claimer: a separate tempdir with its own profile.
        // Bootstrapping happens inside `invite::claim` itself,
        // so we don't pre-init the site.
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_parent, &invite_outcome.url, claimer_config.clone()).await?;
        // The claimer's site now targets the inviter's subject.
        assert_eq!(claim_outcome.subject, inviter.site.repository.did());
        // No remote was attached at mint, so the outcome
        // surfaces None.
        assert!(claim_outcome.remote_url.is_none());

        // Re-opening the joined site lands the same subject.
        let joined =
            SlideSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
        assert_eq!(joined.repository.did(), inviter.site.repository.did());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_join_when_a_site_already_exists() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;

        // Stand up a claimer site, then try to join into the
        // same parent — the existing `.tonk/` should block.
        let claimer = common::TestSite::new().await?;
        let result =
            invite::claim(&claimer.parent, &invite_outcome.url, claimer.config.clone()).await;
        match result {
            Err(InviteError::SiteAlreadyExists(path)) => {
                assert_eq!(path, claimer.parent.join(SITE_DIRNAME));
            }
            other => panic!("expected SiteAlreadyExists, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_a_malformed_invite_url() -> Result<()> {
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let result = invite::claim(&claimer_parent, "not-a-url", claimer_config).await;
        match result {
            Err(InviteError::InvalidInvite(_)) => {}
            other => panic!("expected InvalidInvite, got: {other:?}"),
        }
        // No `.tonk/` should have been created on a parse-failure path.
        assert!(!claimer_parent.join(SITE_DIRNAME).exists());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_uses_the_default_base_url_when_unspecified() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let outcome = invite::mint(&inviter.site, None, None).await?;
        assert!(
            outcome.url.starts_with(invite::DEFAULT_BASE_URL),
            "expected URL to start with {}, got {}",
            invite::DEFAULT_BASE_URL,
            outcome.url,
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_honors_a_custom_base_url() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let outcome = invite::mint(&inviter.site, Some("https://example.test/join"), None).await?;
        assert!(outcome.url.starts_with("https://example.test/join"));
        Ok(())
    }

    /// Site::default_config / build_profile_and_operator are
    /// in-crate bridges; nothing about them is invite-specific
    /// but `claim` is the first caller that depends on the
    /// pub(crate) re-export, so a quick smoke check belongs
    /// here.
    #[dialog_common::test]
    fn default_config_uses_the_canonical_profile_name() {
        let config = site::default_config();
        assert_eq!(config.profile_name, site::PROFILE_NAME);
    }
}

mod when_syncing_with_an_upstream {
    use anyhow::Result;
    use dialog_artifacts::{Artifact, Instruction, Value};
    use dialog_repository::Branch;
    use futures_util::stream;
    use slide::sync::{self, SyncError};

    use crate::common::{self, ATTRIBUTE_DECL};

    /// Open a sibling branch on the same repo and wire `main` to
    /// push/pull against it. The "remote" is in-process so tests
    /// don't need an access service. Returns the upstream handle
    /// so callers can seed it before pulling.
    async fn wire_local_upstream(test: &common::TestSite) -> Result<Branch> {
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
        Ok(upstream)
    }

    /// Commit a single string-valued fact to `branch` via the
    /// dialog primitives. Used to seed an upstream we then pull
    /// from.
    async fn commit_fact(
        test: &common::TestSite,
        branch: &Branch,
        the: &str,
        of: &str,
        is: &str,
    ) -> Result<()> {
        let artifact = Artifact {
            the: the.parse()?,
            of: of.parse()?,
            is: Value::String(is.to_owned()),
            cause: None,
        };
        branch
            .commit(stream::iter(vec![Instruction::Assert(artifact)]))
            .perform(&test.site.operator)
            .await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_pushes_local_claims_to_the_upstream_branch() -> Result<()> {
        let test = common::TestSite::new().await?;
        wire_local_upstream(&test).await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;

        let local_tree = test
            .site
            .branch
            .revision()
            .expect("main should have committed claims")
            .tree;

        let outcome = sync::push(&test.site).await?;
        assert!(outcome.advanced, "push should advance the upstream");
        // Push doesn't move local — both ends of the outcome
        // should match.
        assert_eq!(outcome.before, outcome.after);

        // Re-load the upstream handle to see the post-push state.
        let upstream_after = test
            .site
            .repository
            .branch("upstream")
            .load()
            .perform(&test.site.operator)
            .await?;
        assert_eq!(
            upstream_after
                .revision()
                .expect("upstream should have a revision after push")
                .tree,
            local_tree,
            "upstream tree must match local tree after fast-forward push",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_nothing_to_push_when_main_is_empty() -> Result<()> {
        let test = common::TestSite::new().await?;
        wire_local_upstream(&test).await?;

        // No assertion against main — its revision is None.
        let outcome = sync::push(&test.site).await?;
        assert!(!outcome.advanced);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_pulls_upstream_claims_into_main() -> Result<()> {
        let test = common::TestSite::new().await?;
        let upstream = wire_local_upstream(&test).await?;

        // Seed the upstream with a fact main hasn't seen.
        commit_fact(
            &test,
            &upstream,
            "xyz.tonk.demo/marker",
            "did:key:z6MkfakeEntityForUpstreamTest11111111111111",
            "hello-from-upstream",
        )
        .await?;
        let upstream_tree = upstream
            .revision()
            .expect("upstream should have a revision after seed")
            .tree;

        let outcome = sync::pull(&test.site).await?;
        assert!(outcome.advanced, "pull should report the merged change");

        // Re-load main to get the post-pull revision.
        let main_after = test
            .site
            .repository
            .branch("main")
            .load()
            .perform(&test.site.operator)
            .await?;
        assert_eq!(
            main_after
                .revision()
                .expect("main should have a revision after pull")
                .tree,
            upstream_tree,
            "main tree must match upstream tree after fast-forward pull",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_already_up_to_date_after_a_round_trip() -> Result<()> {
        // Push then pull: the second pull has nothing new to merge.
        let test = common::TestSite::new().await?;
        wire_local_upstream(&test).await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        sync::push(&test.site).await?;

        let outcome = sync::pull(&test.site).await?;
        assert!(!outcome.advanced);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_with_upstream_not_configured_when_unset() -> Result<()> {
        let test = common::TestSite::new().await?;
        // No upstream wiring at all.
        let result = sync::push(&test.site).await;
        match result {
            Err(SyncError::UpstreamNotConfigured { branch }) => {
                assert_eq!(branch, "main");
            }
            other => panic!("expected UpstreamNotConfigured, got: {other:?}"),
        }
        let result = sync::pull(&test.site).await;
        match result {
            Err(SyncError::UpstreamNotConfigured { branch }) => {
                assert_eq!(branch, "main");
            }
            other => panic!("expected UpstreamNotConfigured, got: {other:?}"),
        }
        Ok(())
    }
}

mod when_migrating_from_carry {
    use anyhow::Result;
    use slide::migrate::{self, Mode};
    use slide::site::{SITE_DIRNAME, SlideSite};

    use crate::common;

    /// Build a real `.tonk/` then rename it `.carry/` so the
    /// test fixture is structurally identical to a carry-produced
    /// source — both tools share the dialog-repo on-disk layout.
    async fn carry_lookalike_at(parent: &std::path::Path) -> Result<dialog_capability::Did> {
        let config = common::isolated_config(parent)?;
        let original = SlideSite::init_with(parent, config).await?;
        let did = original.repository.did();
        drop(original);
        std::fs::rename(parent.join(SITE_DIRNAME), parent.join(".carry"))?;
        Ok(did)
    }

    #[dialog_common::test]
    async fn it_copies_the_directory_and_verifies_the_repository_loads() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let original_did = carry_lookalike_at(&parent).await?;

        let outcome = migrate::run(&parent, None, Mode::Copy).await?;
        assert_eq!(outcome.source, parent.join(".carry"));
        assert_eq!(outcome.destination, parent.join(SITE_DIRNAME));
        assert!(!outcome.moved);
        assert_eq!(outcome.repo_did, original_did.to_string());
        // Both sides exist after a copy.
        assert!(parent.join(".carry").is_dir());
        assert!(parent.join(SITE_DIRNAME).is_dir());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_overwrite_an_existing_destination() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;

        // Bootstrap a real .tonk/ — that's the destination we'd
        // be about to clobber.
        let config = common::isolated_config(&parent)?;
        let _site = SlideSite::init_with(&parent, config).await?;

        // Drop a placeholder .carry/ alongside it so the source
        // exists but is incompatible — the refuse-on-conflict
        // check happens before any copy work.
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)?;
        std::fs::write(carry_dir.join("placeholder"), b"")?;

        let result = migrate::run(&parent, None, Mode::Copy).await;
        let err = result.expect_err("expected refuse-on-conflict").to_string();
        assert!(
            err.contains("refusing to overwrite"),
            "unexpected error message: {err}"
        );
        // Destination is preserved untouched.
        assert!(parent.join(SITE_DIRNAME).is_dir());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_removes_the_source_when_moving() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let original_did = carry_lookalike_at(&parent).await?;

        let outcome = migrate::run(&parent, None, Mode::Move).await?;
        assert!(outcome.moved);
        assert_eq!(outcome.repo_did, original_did.to_string());
        assert!(!parent.join(".carry").exists());
        assert!(parent.join(SITE_DIRNAME).is_dir());
        Ok(())
    }
}
