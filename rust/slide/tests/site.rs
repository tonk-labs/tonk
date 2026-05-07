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
