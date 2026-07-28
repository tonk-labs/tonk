//! Spot management ops: create/register/bind/list/remove against
//! an isolated store. These exercise the `spot` module's ops layer
//! the way the `tonk use` / `tonk spot *` commands drive it —
//! nothing here touches process env or the user's data dir.

mod common;

use anyhow::Result;
use tonk_cli::site::TonkSite;
use tonk_cli::spot::{self, SpotStore};

mod when_creating_a_spot {
    use super::*;

    #[dialog_common::test]
    async fn it_creates_registers_and_binds_in_the_canonical_dir() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        let outcome =
            spot::create(&store, "garden", None, Some(tmp.path()), config.clone()).await?;
        assert_eq!(outcome.site, store.canonical_site("garden").canonicalize()?);

        // Registered and bound: resolution from that directory finds it.
        let resolved = store.resolve(None, None, Some(tmp.path()))?;
        assert_eq!(resolved.name, "garden");
        assert_eq!(resolved.site, outcome.site);

        // And the site actually opens.
        let opened = TonkSite::open_with(&resolved.site, config).await?;
        assert_eq!(opened.repository.did().to_string(), outcome.did);
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

        let outcome = spot::create(
            &store,
            "proj",
            Some(&legacy_root),
            Some(&parent),
            config.clone(),
        )
        .await?;
        assert_eq!(outcome.did, legacy.repository.did().to_string());
        assert_eq!(outcome.site, legacy_root.canonicalize()?);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_duplicate_name() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;

        spot::create(&store, "garden", None, Some(tmp.path()), config.clone()).await?;
        let err = spot::create(&store, "garden", None, Some(tmp.path()), config)
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
        let created = spot::create(&store, "garden", None, Some(tmp.path()), config).await?;

        let outcome = spot::remove(&store, "garden", false)?;
        assert!(!outcome.deleted);
        assert!(created.site.exists(), "data kept");
        // Entry and its binding are gone.
        assert!(store.load()?.spots.is_empty());
        assert!(store.load()?.bindings.is_empty());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_deletes_the_site_dir_with_delete() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let created = spot::create(&store, "garden", None, Some(tmp.path()), config).await?;

        let outcome = spot::remove(&store, "garden", true)?;
        assert!(outcome.deleted);
        assert!(!created.site.exists(), "data removed");
        Ok(())
    }
}

mod when_binding_and_listing {
    use super::*;

    #[dialog_common::test]
    async fn it_binds_by_name_and_lists_with_the_active_spot() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = SpotStore::at(tmp.path().join("state"));
        let config = common::isolated_config(&tmp.path().canonicalize()?)?;
        let a_dir = tmp.path().join("a-work");
        let b_dir = tmp.path().join("b-work");
        std::fs::create_dir_all(&a_dir)?;
        std::fs::create_dir_all(&b_dir)?;
        spot::create(&store, "a", None, Some(&a_dir), config.clone()).await?;
        spot::create(&store, "b", None, Some(&b_dir), config).await?;

        let bound = spot::bind(&store, "a", &b_dir)?;
        assert_eq!(bound.name, "a");

        let listing = spot::listing(&store, None, None, Some(&b_dir))?;
        assert_eq!(
            listing
                .rows
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(listing.active.as_ref().map(|c| c.name.as_str()), Some("a"));

        let err = spot::bind(&store, "nope", &b_dir).expect_err("unknown");
        assert!(matches!(err, spot::SpotError::Unknown { .. }), "{err}");
        Ok(())
    }
}
