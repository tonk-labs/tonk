//! Editing is unrestricted and enforcement lives at the service boundary:
//! every registered replica opens whatever the account state, and a sync the
//! service refuses fails there, with copy composed from the space's roster.

mod common;

use anyhow::Result;
use tonk_cli::site::{SiteConfig, TonkSite};
use tonk_cli::space::{AccountRecord, SpaceStore};

const ACCOUNT_A: &str = "did:key:z6MkAccountAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ACCOUNT_B: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

fn fixture(tmp: &std::path::Path) -> Result<(SpaceStore, SiteConfig)> {
    let store = SpaceStore::at(tmp.join("state"));
    let mut config = common::isolated_config(tmp)?;
    config.account_store = store.clone();
    Ok((store, config))
}

/// Create a space and stamp `founder` as its founder on the content branch,
/// the way a hosted space carries its own roster.
async fn founded_by(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
    founder: &str,
    display_name: Option<&str>,
) -> Result<TonkSite> {
    use tonk_schema::{MemberName, MemberRole, Membership};

    let outcome = tonk_cli::space::create(store, name, None, None, config.clone()).await?;
    let site = TonkSite::open_with(&outcome.site, config.clone()).await?;
    let membership = Membership::new(founder.parse()?, site.repository.did());
    let session = site.branch().await?;
    let mut transaction = session
        .handle()
        .transaction()
        .assert(membership.clone())
        .assert(MemberRole::founder(membership.this().clone()));
    if let Some(display_name) = display_name {
        transaction = transaction.assert(MemberName::new(
            membership.this().clone(),
            display_name.to_owned(),
        ));
    }
    transaction.commit().perform(&site.operator).await?;
    Ok(site)
}

mod when_opening_a_replica {
    use super::*;

    /// Possession is not permission, but it is not a lock either: a replica
    /// on this device opens signed out, signed into its own account, and
    /// signed into somebody else's. Nothing consults the account slot.
    #[dialog_common::test]
    async fn every_registered_replica_opens_whatever_the_account_state() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        founded_by(&store, &config, "garden", ACCOUNT_A, None).await?;

        for signed_in in [None, Some(ACCOUNT_A), Some(ACCOUNT_B)] {
            store.set_account(signed_in.map(AccountRecord::new))?;

            let resolved = store.resolve(Some("garden"), None, None)?;
            assert_eq!(resolved.name, "garden");
            let site = TonkSite::open_with(&resolved.site, config.clone()).await?;
            assert!(site.branch().await.is_ok(), "signed in as {signed_in:?}");
        }
        Ok(())
    }

    /// Signing out clears the slot and touches nothing else — not the
    /// replicas, not what the space says about who owns it.
    #[dialog_common::test]
    async fn signing_out_leaves_the_roster_and_the_replicas_alone() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;
        let site = founded_by(&store, &config, "garden", ACCOUNT_A, None).await?;
        let before = tonk_cli::inventory::read_roster(&site).await?;

        store.set_account(None)?;

        let registry = store.load()?;
        assert!(registry.account.is_none());
        assert!(registry.spaces.contains_key("garden"));
        let site = TonkSite::open_with(&registry.spaces["garden"].site, config).await?;
        assert_eq!(tonk_cli::inventory::read_roster(&site).await?, before);
        Ok(())
    }

    /// The registry holds bindings and one signed-in account, and nothing
    /// per-space: there is no tag that could drift from the chains.
    #[dialog_common::test]
    async fn the_registry_records_nothing_about_a_space_but_its_site() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;
        founded_by(&store, &config, "garden", ACCOUNT_A, None).await?;

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.registry_path())?)?;

        assert_eq!(json["account"]["root"], ACCOUNT_A);
        let entry = json["spaces"]["garden"].as_object().expect("space entry");
        assert_eq!(
            entry.keys().collect::<Vec<_>>(),
            vec!["site"],
            "the entry is a binding and nothing else: {entry:?}"
        );
        Ok(())
    }
}

mod when_a_space_already_belongs_to_an_account {
    use super::*;

    /// The refusal explains itself in the product's own terms and names the
    /// owner it read from the space, with no protocol vocabulary leaking in.
    #[test]
    fn the_message_names_the_owner_and_the_way_forward() {
        let message = tonk_cli::space_link::already_owned_message("garden", ACCOUNT_A);

        assert!(message.contains("\"garden\" already belongs to an account"));
        assert!(message.contains("This keeps existing shares working."));
        assert!(message.contains("tonk invite"));
        assert!(message.contains(ACCOUNT_A));
        for forbidden in ["UCAN", "delegation", "prefix"] {
            assert!(!message.contains(forbidden), "copy leaked {forbidden}");
        }
    }
}

mod when_the_service_refuses_a_sync {
    use super::*;

    /// A space somebody else owns: the fix is signing into that account, and
    /// the copy leads with it rather than with revocation.
    #[dialog_common::test]
    async fn it_names_the_owning_account_and_leads_with_signing_in() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let site = founded_by(&store, &config, "roadmap", ACCOUNT_A, Some("Ada Lovelace")).await?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_B)))?;

        let report =
            tonk_cli::sync::rejection_report(&site, "roadmap", "subject is not provisioned").await;

        assert_eq!(
            report,
            // Both roots share the eight-character default, so the
            // abbreviation lengthens until they can be told apart.
            "could not sync 'roadmap': this device holds no authority its access \
             service accepts\nthe access service said: subject is not \
             provisioned\n'roadmap' is owned by Ada Lovelace (z6MkAccountA); \
             you are signed in as z6MkAccountB. sign into the owning account with \
             `tonk account login`, or ask a member for an invite and claim it \
             with `tonk join <URL>`"
        );
        Ok(())
    }

    /// A space the signed-in account owns: signing in cannot be the fix, so
    /// the honest explanation is revocation and the copy points at the
    /// device list.
    #[dialog_common::test]
    async fn it_points_at_the_device_list_for_a_space_this_account_owns() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let site = founded_by(&store, &config, "garden", ACCOUNT_A, None).await?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;

        let report =
            tonk_cli::sync::rejection_report(&site, "garden", "principal is revoked").await;

        assert_eq!(
            report,
            "could not sync 'garden': the access service rejected this device's \
             authority\nthe access service said: principal is revoked\nthis \
             device may have been revoked; check `tonk account devices`, or ask \
             a member for a new invite and claim it with `tonk join <URL>`"
        );
        Ok(())
    }

    /// Whatever the CLI infers about the fix, the boundary's own words reach
    /// the person: they are the only part of the message that came from the
    /// thing that actually said no.
    #[dialog_common::test]
    async fn it_carries_the_services_reason_through_verbatim() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let owned = founded_by(&store, &config, "garden", ACCOUNT_A, None).await?;
        let foreign = founded_by(&store, &config, "roadmap", ACCOUNT_B, None).await?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;

        for (site, name) in [(&owned, "garden"), (&foreign, "roadmap")] {
            let report = tonk_cli::sync::rejection_report(site, name, "policy 'is member'").await;

            assert!(
                report.contains("the access service said: policy 'is member'"),
                "{report}"
            );
        }
        Ok(())
    }

    /// A member's sync is refused the same way an owner's is, but the fix is
    /// not: they cannot sign into the account that owns the space, and being
    /// told to would send them somewhere they can never get to. A roster row
    /// of their own is what tells the two apart.
    #[dialog_common::test]
    async fn it_points_a_member_at_revocation_not_at_the_owners_account() -> Result<()> {
        use tonk_schema::{MemberRole, Membership};

        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let site = founded_by(&store, &config, "roadmap", ACCOUNT_A, Some("Ada")).await?;
        store.set_account(Some(AccountRecord::new(ACCOUNT_B)))?;

        // Before the membership row exists, the space has never heard of us.
        let stranger = tonk_cli::sync::rejection_report(&site, "roadmap", "denied").await;
        assert!(stranger.contains("is owned by Ada"), "{stranger}");

        let membership = Membership::new(ACCOUNT_B.parse()?, site.repository.did());
        let session = site.branch().await?;
        session
            .handle()
            .transaction()
            .assert(membership.clone())
            .assert(MemberRole::member(membership.this().clone()))
            .commit()
            .perform(&site.operator)
            .await?;
        drop(session);

        let member = tonk_cli::sync::rejection_report(&site, "roadmap", "denied").await;

        assert!(member.contains("may have been revoked"), "{member}");
        assert!(!member.contains("sign into the owning account"), "{member}");
        Ok(())
    }

    /// A space with no roster at all cannot blame another account either.
    #[dialog_common::test]
    async fn it_falls_back_to_revocation_when_the_space_names_nobody() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let outcome =
            tonk_cli::space::create(&store, "scratch", None, None, config.clone()).await?;
        let site = TonkSite::open_with(&outcome.site, config).await?;

        let report = tonk_cli::sync::rejection_report(&site, "scratch", "no such subject").await;

        assert!(report.contains("may have been revoked"), "{report}");
        assert!(!report.contains("is owned by"), "{report}");
        Ok(())
    }

    /// Live: the access service refuses a subject it was never asked to
    /// serve, and that refusal arrives as an access decision rather than as
    /// an opaque transport failure.
    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn a_service_refusal_arrives_as_an_access_decision(
        env: tonk_access_service::helpers::AccessServiceAddress,
    ) -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let (store, config) = fixture(tmp.path())?;
        let outcome = tonk_cli::space::create(&store, "garden", None, None, config.clone()).await?;
        let site = TonkSite::open_with(&outcome.site, config).await?;
        tonk_cli::remote::add(
            &site,
            tonk_cli::remote::DEFAULT_REMOTE,
            &env.access_service_url,
            Some(site.repository.did()),
        )
        .await?;
        tonk_cli::remote::set_upstream(&site, tonk_cli::remote::DEFAULT_REMOTE).await?;

        let error = tonk_cli::sync::push(&site)
            .await
            .expect_err("an unprovisioned subject is not served");

        assert!(
            matches!(error, tonk_cli::sync::SyncError::Rejected { .. }),
            "{error:?}"
        );
        Ok(())
    }
}
