//! Spot management ops: create/register/resolve/list/remove against
//! an isolated store. These exercise the `spot` module's ops layer
//! the way the `tonk spot enter` / `tonk spot *` commands drive it —
//! nothing here touches process env or the user's data dir.

mod common;

use anyhow::Result;
use tonk_cli::site::TonkSite;
use tonk_cli::spot::{self, Source, SpotStore};

mod when_creating_a_spot {
    use super::*;

    #[dialog_common::test]
    async fn it_creates_and_registers_in_the_canonical_dir() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        let outcome = spot::create(&store, "garden", None, config.clone()).await?;
        assert_eq!(outcome.site, store.canonical_site("garden").canonicalize()?);

        // Registered: the name now resolves.
        let resolved = store.resolve_reference("garden", Source::Argument)?;
        assert_eq!(resolved.name, "garden");
        assert_eq!(resolved.site, outcome.site);

        // And the site actually opens.
        let opened = TonkSite::open_with(&resolved.site, config).await?;
        assert_eq!(opened.repository.did().to_string(), outcome.did);
        Ok(())
    }

    /// Creating must not select. A `tonk spot new` that silently
    /// repointed the session would be the machine-wide default
    /// coming back through the side door.
    #[dialog_common::test]
    async fn it_does_not_put_the_session_on_the_new_spot() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        spot::create(&store, "garden", None, config).await?;

        let err = store.resolve(None).expect_err("creating selects nothing");
        assert!(matches!(err, spot::SpotError::NoSelection), "{err}");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_adopts_an_existing_site_via_site_override() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        // A pre-existing site (the old cwd-`.tonk` migration case).
        let legacy_root = parent.join("proj").join(".tonk");
        let legacy = TonkSite::init_at_with(&legacy_root, config.clone()).await?;

        let outcome = spot::create(&store, "proj", Some(&legacy_root), config.clone()).await?;
        assert_eq!(outcome.did, legacy.repository.did().to_string());
        assert_eq!(outcome.site, legacy_root.canonicalize()?);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_duplicate_name() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        spot::create(&store, "garden", None, config.clone()).await?;
        let err = spot::create(&store, "garden", None, config)
            .await
            .expect_err("duplicate");
        assert!(matches!(err, spot::SpotError::Exists(_)), "{err}");
        Ok(())
    }
}

mod when_removing_a_spot {
    use super::*;

    #[dialog_common::test]
    async fn it_unregisters_but_keeps_data_by_default() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let created = spot::create(&store, "garden", None, config).await?;

        let outcome = spot::remove(&store, "garden", false)?;
        assert!(!outcome.deleted);
        assert!(created.site.exists(), "data kept");
        // The entry is gone.
        assert!(store.load()?.spots.is_empty());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_deletes_the_site_dir_with_delete() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let created = spot::create(&store, "garden", None, config).await?;

        let outcome = spot::remove(&store, "garden", true)?;
        assert!(outcome.deleted);
        assert!(!created.site.exists(), "data removed");
        Ok(())
    }
}

mod when_listing {
    use super::*;

    #[dialog_common::test]
    async fn it_lists_every_spot_and_marks_the_sessions_own() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        spot::create(&store, "a", None, config.clone()).await?;
        spot::create(&store, "b", None, config).await?;

        // The reference this session carries is what gets marked —
        // here the env form, standing in for `tonk spot enter a`.
        let listing = spot::listing(&store, Some("a"))?;
        assert_eq!(
            listing
                .rows
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        let active = listing.active.as_ref().expect("a resolves");
        assert_eq!((active.name.as_str(), active.source), ("a", Source::Env));

        // With no reference anywhere, the rows still list but
        // nothing is active — there is no machine-wide default to
        // fall back on.
        let listing = spot::listing(&store, None)?;
        assert_eq!(listing.rows.len(), 2);
        assert!(listing.active.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_no_active_spot_for_a_dangling_reference() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        spot::create(&store, "a", None, config).await?;

        let listing = spot::listing(&store, Some("nope"))?;
        assert_eq!(listing.rows.len(), 1);
        assert!(listing.active.is_none());
        Ok(())
    }
}

mod when_a_directory_holds_the_site {
    use super::*;

    /// A `.tonk` directory is usable, but only by being *named* —
    /// as a path reference, which is what bare `tonk spot enter`
    /// resolves and exports. It never resolves from the cwd on its
    /// own, so no command can act on a directory you merely walked
    /// into.
    #[dialog_common::test]
    async fn it_resolves_an_unregistered_site_only_when_named() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        let proj = parent.join("proj");
        let root = proj.join(tonk_cli::site::SITE_DIRNAME);
        let created = TonkSite::init_at_with(&root, config.clone()).await?;

        // Named as a path: resolves, with nothing in the registry.
        let named = root.to_str().expect("utf-8 site path");
        let resolved = store.resolve(Some(named))?;
        assert_eq!(resolved.source, Source::Env);
        assert_eq!(resolved.site, root.canonicalize()?);
        assert!(store.load()?.spots.is_empty(), "nothing registered");

        let opened = TonkSite::open_with(&resolved.site, config).await?;
        assert_eq!(
            opened.repository.did().to_string(),
            created.repository.did().to_string()
        );

        // Unnamed: the same directory is not a selection.
        assert!(store.resolve(None).is_err(), "cwd is not a selector");
        Ok(())
    }

    /// What bare `tonk spot enter` starts from — the only place a
    /// `.tonk` beside you is consulted, because running that
    /// command is itself the explicit act.
    #[dialog_common::test]
    async fn it_finds_the_local_site_for_a_bare_enter() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        let proj = parent.join("proj");
        let root = proj.join(tonk_cli::site::SITE_DIRNAME);
        TonkSite::init_at_with(&root, config).await?;

        assert_eq!(SpotStore::local_site(&proj), Some(root));
        // Never a parent's.
        let nested = proj.join("nested");
        std::fs::create_dir_all(&nested)?;
        assert_eq!(SpotStore::local_site(&nested), None);
        Ok(())
    }
}
