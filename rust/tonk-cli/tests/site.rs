//! Behavioural tests for the site lifecycle:
//! initialization, profile identity, and migration from carry.

mod common;

mod when_initializing_a_site {
    use anyhow::Result;
    use tonk_cli::site::TonkSite;

    use crate::common;

    #[dialog_common::test]
    async fn it_creates_a_dialog_repository_with_a_stable_did() -> Result<()> {
        let test = common::TestSite::new().await?;
        // The repo's DID is a real ed25519 key, not the empty
        // string — site init produced a usable repo backed by
        // the on-disk `.tonk/` directory.
        let did = test.site.repository.did().to_string();
        assert!(did.starts_with("did:key:"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_is_idempotent_when_a_site_already_exists() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;

        let first = TonkSite::init_with(&parent, config.clone()).await?;
        let did_first = first.repository.did();

        let second = TonkSite::init_with(&parent, config).await?;
        // Re-running init returns the same repository — the
        // user can call it defensively at the start of any task.
        assert_eq!(did_first, second.repository.did());
        Ok(())
    }
}

mod when_managing_remotes {
    use anyhow::Result;
    use tonk_cli::remote::{self, RemoteError};

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
        let session = test.site.branch().await?;
        assert!(session.handle().upstream().is_some());
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

mod when_resolving_a_remote {
    use anyhow::Result;
    use tonk_cli::remote::{self, RemoteError};

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";
    const OTHER: &str = "https://other.example.test/ucan/";

    #[dialog_common::test]
    async fn it_resolves_nothing_when_no_remote_is_registered() -> Result<()> {
        let test = common::TestSite::new().await?;
        assert!(remote::resolve(&test.site, None).await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_the_only_registered_remote() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        let resolved = remote::resolve(&test.site, None).await?;
        assert_eq!(resolved.expect("a lone remote resolves").endpoint, ENDPOINT);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_the_named_remote_when_several_exist() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;

        let resolved = remote::resolve(&test.site, Some("backup")).await?;
        assert_eq!(resolved.expect("named remote resolves").endpoint, OTHER);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_guess_between_several_remotes() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;

        match remote::resolve(&test.site, None).await {
            Err(RemoteError::AmbiguousRemote(names)) => {
                assert!(names.contains("origin"), "names both: {names}");
                assert!(names.contains("backup"), "names both: {names}");
            }
            other => panic!("expected AmbiguousRemote, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_on_a_name_that_is_not_registered() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        match remote::resolve(&test.site, Some("missing")).await {
            Err(RemoteError::UnknownRemote(name)) => assert_eq!(name, "missing"),
            other => panic!("expected UnknownRemote, got: {other:?}"),
        }
        Ok(())
    }
}

mod when_the_invite_remote_is_not_the_upstream {
    use anyhow::Result;
    use tonk_cli::remote;

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";
    const OTHER: &str = "https://other.example.test/ucan/";

    #[dialog_common::test]
    async fn it_reports_no_upstream_remote_on_a_freshly_added_remote() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        // `remote::add` only registers; it never touches the upstream,
        // so there is nothing to diverge from and nothing to warn
        // about. (`tonk remote add`, the command, layers a default
        // set-upstream on top when no upstream is configured yet.)
        assert!(remote::upstream_remote(&test.site).await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_names_the_remote_the_branch_tracks() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;
        remote::set_upstream(&test.site, "origin").await?;

        let upstream = remote::upstream_remote(&test.site).await?;
        assert_eq!(upstream.as_deref(), Some("origin"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_differs_from_a_remote_the_branch_does_not_track() -> Result<()> {
        let test = common::TestSite::new().await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        remote::add(&test.site, "backup", OTHER, None).await?;
        remote::set_upstream(&test.site, "origin").await?;

        let upstream = remote::upstream_remote(&test.site).await?;

        let backup = remote::resolve(&test.site, Some("backup"))
            .await?
            .expect("named remote resolves");
        assert_ne!(upstream.as_deref(), Some(backup.name.as_str()));

        let origin = remote::resolve(&test.site, Some("origin"))
            .await?
            .expect("named remote resolves");
        assert_eq!(upstream.as_deref(), Some(origin.name.as_str()));
        Ok(())
    }
}

mod when_shortening_an_invite {
    #[cfg(feature = "integration-tests")]
    use anyhow::Result;
    #[cfg(feature = "integration-tests")]
    use tonk_access_service::helpers::AccessServiceAddress;
    #[cfg(feature = "integration-tests")]
    use tonk_cli::invite;
    #[cfg(feature = "integration-tests")]
    use tonk_cli::remote;

    #[cfg(feature = "integration-tests")]
    use crate::common;

    /// The relay these fixtures' remote publishes revocations to — an
    /// invite that embeds a remote must name one. It stays off the live
    /// service on purpose: neither the mint nor the claim calls it, and in
    /// production it is the artifact host, not the access endpoint.
    #[cfg(feature = "integration-tests")]
    const REVOCATION_RELAY: &str = "https://artifacts.example.test/revocations/";

    /// Full loop against a live local shortcut service: mint the long
    /// URL, shorten it (`PUT /@` on the link's own origin), then claim
    /// the short link — the claim resolves the 301 by hand, splicing
    /// the seed fragment back on the way a browser would.
    #[dialog_common::test]
    async fn it_shortens_and_claims_a_minted_invite(env: AccessServiceAddress) -> Result<()> {
        let endpoint = env.access_service_url.as_str();
        let inviter = common::TestSite::new().await?;
        remote::add_with_revocation(
            &inviter.site,
            "origin",
            endpoint,
            None,
            Some(REVOCATION_RELAY),
        )
        .await?;

        let base = format!("{endpoint}/join");
        let outcome = invite::mint_with_relay(
            &inviter.site,
            Some(&base),
            Some(endpoint),
            Some(REVOCATION_RELAY),
        )
        .await?;
        assert!(
            outcome.url.contains("access="),
            "long form: {}",
            outcome.url
        );

        let short = invite::shorten(&outcome.url).await?;
        assert!(
            short.starts_with(&format!("{endpoint}/@/")),
            "short form on the link's origin: {short}"
        );
        assert!(short.contains('#'), "seed fragment survives: {short}");
        assert!(
            !short.contains("access="),
            "chain stays off the link: {short}"
        );

        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome = invite::claim(&claimer_root, &short, claimer_config).await?;
        assert_eq!(claim_outcome.subject, inviter.site.repository.did());
        assert!(claim_outcome.remote_url.is_some());
        Ok(())
    }

    /// The regression: with no explicit base, the link must land on
    /// the remote's own origin — the only origin whose same-origin
    /// shortcut service can answer, and the deployment actually
    /// serving the repo.
    #[dialog_common::test]
    async fn it_derives_the_base_from_the_remote_and_shortens(
        env: AccessServiceAddress,
    ) -> Result<()> {
        let endpoint = env.access_service_url.as_str();
        let inviter = common::TestSite::new().await?;
        remote::add_with_revocation(
            &inviter.site,
            "origin",
            endpoint,
            None,
            Some(REVOCATION_RELAY),
        )
        .await?;

        let resolved = remote::resolve(&inviter.site, None)
            .await?
            .expect("the lone remote resolves");
        let base = invite::base_url_for_remote(&resolved.endpoint)?;
        assert_eq!(base, format!("{endpoint}/join"));

        // Relay off the resolved record, the way the command does it.
        let outcome = invite::mint_with_relay(
            &inviter.site,
            Some(&base),
            Some(&resolved.endpoint),
            resolved.revocation_url.as_deref(),
        )
        .await?;
        let short = invite::shorten(&outcome.url).await?;
        assert!(
            short.starts_with(&format!("{endpoint}/@/")),
            "short link sits on the remote's origin: {short}"
        );
        Ok(())
    }
}

mod when_claiming_before_account_linking {
    #[cfg(feature = "integration-tests")]
    use anyhow::Result;
    #[cfg(feature = "integration-tests")]
    use tonk_access_service::helpers::AccessServiceAddress;
    #[cfg(feature = "integration-tests")]
    use tonk_cli::{invite, remote, sync};

    #[cfg(feature = "integration-tests")]
    use crate::common;

    /// The invite's profile-ending chain, not an account session, authorizes
    /// the initial pull. Account linking later makes the profile portable; it
    /// is not a precondition for receiving the shared space.
    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn it_pulls_with_invite_authority(env: AccessServiceAddress) -> Result<()> {
        let endpoint = env.access_service_url.as_str();
        let inviter = common::TestSite::new().await?;
        env.provision_subject(inviter.site.repository.did().as_str())
            .await?;
        remote::add(&inviter.site, "origin", endpoint, None).await?;
        remote::set_upstream(&inviter.site, "origin").await?;
        sync::push(&inviter.site).await?;

        let base = format!("{endpoint}/join");
        let invitation = invite::mint(&inviter.site, Some(&base), Some(endpoint)).await?;
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let mut claimer_config = common::isolated_config(&claimer_parent)?;
        claimer_config.require_account = true;

        let outcome = invite::claim(&claimer_root, &invitation.url, claimer_config).await?;

        assert!(
            outcome.synced,
            "the invite authority should pull before any account is linked"
        );
        Ok(())
    }
}

/// The library's own copy of what the binary no longer enforces: a mint that
/// embeds a remote used to be refused unless the remote named a relay for its
/// revocations. A revocation is an ordinary `ucan/revoke` invocation now,
/// addressed to the access service the invite already carries, so there is
/// nothing left to demand. Offline: the check that is gone ran before any
/// network, so its absence shows without one.
mod when_minting_an_invite_that_embeds_a_relay_less_remote {
    use anyhow::Result;
    use tonk_cli::invite;

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";

    #[dialog_common::test]
    async fn it_mints_without_demanding_a_relay() -> Result<()> {
        let inviter = common::TestSite::new().await?;

        let outcome = invite::mint(&inviter.site, None, Some(ENDPOINT))
            .await
            .expect("a relay-less remote still mints");

        assert!(!outcome.url.is_empty());
        Ok(())
    }
}

mod when_claiming_an_invite_with_a_remote {
    use anyhow::Result;
    use tonk_cli::invite;
    use tonk_cli::remote;
    use tonk_cli::site::TonkSite;

    use crate::common;

    const ENDPOINT: &str = "https://access.example.test/ucan/";
    const REVOCATION_RELAY: &str = "https://artifacts.example.test/revocations/";

    #[dialog_common::test]
    async fn it_auto_configures_the_embedded_remote_on_the_joined_site() -> Result<()> {
        // Inviter site has a remote registered, mints an invite
        // referencing it.
        let inviter = common::TestSite::new().await?;
        remote::add_with_revocation(
            &inviter.site,
            "origin",
            ENDPOINT,
            None,
            Some(REVOCATION_RELAY),
        )
        .await?;
        let invite_outcome =
            invite::mint_with_relay(&inviter.site, None, Some(ENDPOINT), Some(REVOCATION_RELAY))
                .await?;
        // The endpoint rides inside the signed chain, not the URL.
        assert!(!invite_outcome.url.contains("remote="));
        let parsed = tonk_invite::Invite::parse_url(&invite_outcome.url).await?;
        assert_eq!(
            parsed.remote_url.as_ref().map(url::Url::as_str),
            Some(ENDPOINT)
        );

        // Claimer joins into a fresh tempdir.
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;

        // Claim returned the embedded URL and surfaced the
        // auto-configured remote name.
        assert!(claim_outcome.remote_url.is_some());
        assert_eq!(
            claim_outcome.auto_configured_remote.as_deref(),
            Some("origin")
        );

        // Open the joined site and assert the remote landed in
        // its meta branch and main's upstream is wired.
        let joined = TonkSite::open_with(&claimer_root, claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "origin");
        assert_eq!(listed[0].endpoint, ENDPOINT);
        assert_eq!(listed[0].revocation_url.as_deref(), Some(REVOCATION_RELAY));
        // Subject is the inviter's DID — tonk holds delegated
        // authority on the inviter's repo.
        assert_eq!(listed[0].subject, inviter.site.repository.did());
        let session = joined.branch().await?;
        assert!(session.handle().upstream().is_some());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_skips_remote_setup_when_invite_has_no_remote() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;
        assert!(!invite_outcome.url.contains("remote="));

        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;

        assert!(claim_outcome.auto_configured_remote.is_none());
        let joined = TonkSite::open_with(&claimer_root, claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert!(listed.is_empty());
        Ok(())
    }
}

mod when_recording_roster_facts {
    use anyhow::Result;
    use dialog_query::{Output as _, Query, Term};
    use tonk_cli::invite;
    use tonk_cli::site::TonkSite;
    use tonk_invite::Invite;
    use tonk_schema::prelude::DidExt as _;
    use tonk_schema::{Invitation, InvitedVia, MemberRole, Membership};

    use crate::common;

    /// The mint records an invitation on the inviter's copy of the
    /// repo meta; the claim records the membership + provenance on
    /// the claimer's copy. Both reference the same content-derived
    /// invitation entity.
    #[dialog_common::test]
    async fn it_records_roster_facts_on_mint_and_claim() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;
        let parsed = Invite::parse_url(&invite_outcome.url).await?;
        let expected =
            Invitation::from_chain(&parsed.chain).expect("invite chains have a specific subject");

        // Inviter side: invitation recorded at mint.
        let inviter_meta = inviter
            .site
            .repository
            .branch(tonk_cli::remote::META_BRANCH)
            .open()
            .perform(&inviter.site.operator)
            .await?;
        let invitations: Vec<Invitation> = inviter_meta
            .query()
            .select(Query::<Invitation> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                inviter: Term::var("inviter"),
                audience: Term::var("audience"),
            })
            .perform(&inviter.site.operator)
            .try_vec()
            .await?;
        assert_eq!(invitations.len(), 1);
        assert_eq!(invitations[0].this, expected.this);

        // Claim into a fresh site.
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;
        let joined = TonkSite::open_with(&claimer_root, claimer_config).await?;

        // Claimer side: membership + stamp referencing the same invitation
        // entity, on the *content* branch. Only upstreamed branches sync, so
        // a roster row on `meta` would never reach the space's owner — and
        // the content branch is where every reader of the roster looks.
        let claimer_session = joined.branch().await?;
        let claimer_content = claimer_session.handle();
        let memberships: Vec<Membership> = claimer_content
            .query()
            .select(Query::<Membership> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                member: Term::var("member"),
            })
            .perform(&joined.operator)
            .try_vec()
            .await?;
        assert_eq!(memberships.len(), 1);
        let roles: Vec<MemberRole> = claimer_content
            .query()
            .select(Query::<MemberRole> {
                this: Term::var("this"),
                role: Term::var("role"),
            })
            .perform(&joined.operator)
            .try_vec()
            .await?;
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].this, memberships[0].this);
        assert_eq!(roles[0].role.0.to_string(), MemberRole::MEMBER);
        // The member the claim recorded is the joiner's onboarding
        // account — the durable identity an unlinked device has.
        let root_bytes = joined
            .profile
            .credential()
            .site(tonk_cli::onboarding::ONBOARDING_GRANT_SITE)
            .load::<Vec<u8>>()
            .perform(&joined.operator)
            .await?;
        let chain = dialog_ucan_core::DelegationChain::try_from(root_bytes.as_slice())
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let root_did: dialog_varsig::Did = chain.issuer().clone();
        assert_eq!(memberships[0].member.0, root_did.this());

        let stamps: Vec<InvitedVia> = claimer_content
            .query()
            .select(Query::<InvitedVia> {
                this: Term::var("this"),
                invitation: Term::var("invitation"),
            })
            .perform(&joined.operator)
            .try_vec()
            .await?;
        assert_eq!(stamps.len(), 1);
        assert_eq!(stamps[0].this, *memberships[0].this());
        assert_eq!(stamps[0].invitation.0, expected.this);

        // The claimed chain is retained on the content branch, so the hop
        // that admits this member is provable from the space itself: what
        // an admin revokes to remove them alone.
        let proof = claimer_content
            .delegations()
            .prove(
                root_did.clone(),
                dialog_ucan::Scope {
                    subject: dialog_ucan_core::subject::Subject::Specific(
                        joined.repository.did().clone(),
                    ),
                    command: dialog_ucan_core::command::Command::parse("/use")?,
                    parameters: dialog_ucan::Parameters::default(),
                },
            )
            .perform(&joined.operator)
            .await?;
        let leaf = proof
            .proofs
            .last()
            .expect("the claimed chain reaches the member");
        assert_eq!(leaf.0.audience(), &root_did, "the leaf admits the member");
        assert!(
            proof.proofs.len() > 1,
            "the member's own hop sits below the invite hop"
        );

        Ok(())
    }
}

mod when_minting_and_claiming_an_invite {
    use anyhow::Result;
    use tonk_cli::invite::{self, InviteError};
    use tonk_cli::site::{self, TonkSite};

    use crate::common;

    #[dialog_common::test]
    async fn it_round_trips_an_invite_between_two_sites() -> Result<()> {
        // Inviter: a fully initialised tonk site.
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
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;
        // The claimer's site now targets the inviter's subject.
        assert_eq!(claim_outcome.subject, inviter.site.repository.did());
        // No remote was attached at mint, so the outcome
        // surfaces None.
        assert!(claim_outcome.remote_url.is_none());

        // Re-opening the joined site lands the same subject.
        let joined = TonkSite::open_with(&claimer_root, claimer_config).await?;
        assert_eq!(joined.repository.did(), inviter.site.repository.did());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_to_join_when_a_site_already_exists() -> Result<()> {
        let inviter = common::TestSite::new().await?;
        let invite_outcome = invite::mint(&inviter.site, None, None).await?;

        // Claim once into a fresh root, then try to claim again
        // against the same root — the existing site should block.
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        invite::claim(&claimer_root, &invite_outcome.url, claimer_config.clone()).await?;

        let result = invite::claim(&claimer_root, &invite_outcome.url, claimer_config).await;
        match result {
            Err(err @ InviteError::SiteAlreadyExists(_)) => {
                assert!(
                    err.to_string().contains("a site already exists"),
                    "unexpected message: {err}"
                );
            }
            other => panic!("expected SiteAlreadyExists, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_rejects_a_malformed_invite_url() -> Result<()> {
        let claimer_tmp = tempfile::tempdir()?;
        let claimer_parent = claimer_tmp.path().canonicalize()?;
        let claimer_root = claimer_parent.join("joined-site");
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let result = invite::claim(&claimer_root, "not-a-url", claimer_config).await;
        match result {
            Err(InviteError::InvalidInvite(_)) => {}
            other => panic!("expected InvalidInvite, got: {other:?}"),
        }
        // No site directory should have been created on a parse-failure path.
        assert!(!claimer_root.exists());
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
        let config = site::default_config().expect("default config");
        assert_eq!(config.profile_name, site::PROFILE_NAME);
    }
}

mod when_syncing_with_an_upstream {
    use anyhow::Result;
    use dialog_artifacts::{Artifact, Instruction, Value};
    use dialog_repository::Branch;
    use futures_util::stream;
    use tonk_cli::sync::{self, SyncError};

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
        let session = test.site.branch().await?;
        session
            .handle()
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

        let session = test.site.branch().await?;
        let local_tree = session
            .handle()
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

    // (Removed `it_reports_nothing_to_push_when_main_is_empty`: `tonk
    // init` now seeds the standard library, so main is never empty.
    // No-op sync is covered by
    // `it_reports_already_up_to_date_after_a_round_trip`.)

    #[dialog_common::test]
    async fn it_pulls_upstream_claims_into_main() -> Result<()> {
        let test = common::TestSite::new().await?;
        wire_local_upstream(&test).await?;

        // Push main's standard-library seed up first so the upstream
        // shares main's base; the pull then fast-forwards cleanly.
        sync::push(&test.site).await?;
        // Re-load the upstream handle so it reflects the pushed seed.
        let upstream = test
            .site
            .repository
            .branch("upstream")
            .load()
            .perform(&test.site.operator)
            .await?;

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

mod when_authoring_an_html_view {
    use anyhow::{Result, anyhow};
    use dialog_artifacts::{Attribute, Value};
    use dialog_query::{AttributeQuery, Output as _, Term, attribute};

    use crate::common::{self, VIEW_DECL};

    /// Pull every `(text/html, ?of, ?is)` claim on `main`.
    async fn select_text_html_claims(
        site: &tonk_cli::site::TonkSite,
    ) -> Result<Vec<dialog_query::Claim>> {
        let the: Attribute = "text/html"
            .parse()
            .map_err(|e| anyhow!("text/html should be a valid attribute URI: {e:?}"))?;
        let the_term: attribute::The = the.into();
        let session = site.branch().await?;
        session
            .handle()
            .query()
            .select(AttributeQuery::new(
                Term::from(the_term),
                Term::<dialog_artifacts::Entity>::var("of"),
                Term::<dialog_query::Any>::var("is"),
                Term::<attribute::Cause>::blank(),
                None,
            ))
            .perform(&site.operator)
            .try_vec()
            .await
            .map_err(|e| anyhow!("text/html query failed: {e:?}"))
    }

    /// De-risk the Phase-2 design. Confirms three properties:
    /// 1. `attribute! the: text/html` is accepted by parse +
    ///    analyzer (the dialog layer permits `text/html` even
    ///    though the domain is dotless).
    /// 2. A `view!` head whose body field references that
    ///    attribute lands as a literal `(text/html, ?, body)`
    ///    claim — i.e. the URI on the wire really is `text/html`,
    ///    not a synthesised concept-namespace URI.
    /// 3. Re-asserting the same body is idempotent (Phase-2's
    ///    "git-tag" semantics).
    #[dialog_common::test]
    async fn it_round_trips_a_view_through_the_seed_schema() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(VIEW_DECL).await?;
        test.eval_inline(
            r#"view!: &my-view
  body: "<h1>hi</h1>"
"#,
        )
        .await?;

        let claims = select_text_html_claims(&test.site).await?;
        assert_eq!(
            claims.len(),
            1,
            "expected exactly one text/html claim, got: {claims:?}",
        );
        let claim = &claims[0];
        // The wire-format attribute really is `text/html` — the
        // host route's `(the=text/html, of=<entity>)` selector
        // will match this row.
        assert_eq!(claim.the.to_string(), "text/html");
        match &claim.is {
            Value::String(s) => assert_eq!(s, "<h1>hi</h1>"),
            other => panic!("expected String body, got: {other:?}"),
        }

        // Re-asserting the same body is a no-op: same content →
        // same entity, same claim. The claim count must stay 1.
        test.eval_inline(
            r#"view!: &my-view
  body: "<h1>hi</h1>"
"#,
        )
        .await?;
        let claims = select_text_html_claims(&test.site).await?;
        assert_eq!(
            claims.len(),
            1,
            "re-asserting identical body should be idempotent",
        );
        Ok(())
    }
}

mod when_listing_concepts {
    use anyhow::Result;
    use tonk_cli::schema;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    #[dialog_common::test]
    async fn it_includes_user_defined_concepts() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;

        let concepts = schema::list_concepts(&test.site).await?;
        let task = concepts
            .iter()
            .find(|c| c.name == "task")
            .expect("the user-defined `task` concept should appear in the listing");
        // Concept descriptions don't round-trip through the
        // anonymous-concept dispatch path the listing uses; see
        // the fidelity-gap note on `tonk_cli::schema`. Asserting
        // `None` pins that behaviour so a future fix lights up
        // the test as a reminder to revisit.
        assert!(task.description.is_none());
        let mut fields = task.fields.clone();
        fields.sort();
        assert_eq!(fields, vec!["done", "title"]);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_omits_the_runtime_vocabulary_a_fresh_site_seeds() -> Result<()> {
        // Site init lowers core.yaml and the analyzer registers its
        // built-ins, so a fresh branch carries forty-odd concepts the
        // author never wrote. None of them belongs in the listing.
        let test = common::TestSite::new().await?;
        let concepts = schema::list_concepts(&test.site).await?;
        assert!(
            concepts.is_empty(),
            "a fresh site defines no concepts of its own; saw {:?}",
            concepts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        // Each of these is a different source of noise: an analyzer
        // built-in, a standard-library concept, and a standard-library
        // command. Naming them pins the filter to all three.
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        let listed: Vec<_> = schema::list_concepts(&test.site)
            .await?
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(listed, vec!["task".to_string()]);
        for omitted in ["command", "view", "tonk/agents", "tonk/invite"] {
            assert!(
                !listed.contains(&omitted.to_string()),
                "{omitted} is runtime vocabulary and should not be listed"
            );
            // Omitted from the listing, but still addressable by name:
            // the filter is presentational, not a scoping rule.
            assert!(
                schema::find_concept(&test.site, omitted).await?.is_some(),
                "{omitted} should still resolve by name"
            );
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_excludes_user_defined_concepts_absent_on_a_fresh_site() -> Result<()> {
        // A fresh site has no user-defined `task` concept — only
        // built-ins are seeded. This pins that the user-defined
        // concept the sibling test relies on is genuinely
        // user-introduced, not present by default.
        let test = common::TestSite::new().await?;
        let concepts = schema::list_concepts(&test.site).await?;
        assert!(
            !concepts.iter().any(|c| c.name == "task"),
            "fresh site should not list a user-defined `task` concept; saw {:?}",
            concepts.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        Ok(())
    }
}

mod when_listing_views {
    use anyhow::Result;
    use tonk_cli::views;

    use crate::common::{self, VIEW_DECL};

    #[dialog_common::test]
    async fn it_returns_empty_on_a_branch_with_no_text_html_claims() -> Result<()> {
        let test = common::TestSite::new().await?;
        let listed = views::list(&test.site).await?;
        assert!(listed.is_empty());
        Ok(())
    }

    /// `view add` writes a `show` entry; the listing used to
    /// select on `text/html` alone and so came back empty right after
    /// a successful add.
    #[dialog_common::test]
    async fn it_lists_a_view_authored_through_view_add() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(common::ATTRIBUTE_DECL).await?;
        test.eval_inline(common::CONCEPT_DECL).await?;
        tonk_cli::data_ops::view_add(
            &test.site,
            "task",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{title}</b>",
            false,
            Default::default(),
        )
        .await?;

        let listed = views::list(&test.site).await?;
        let row = listed
            .iter()
            .find(|row| row.name.as_deref() == Some("task"))
            .expect("the authored view should be listed");
        assert_eq!(row.model.as_deref(), Some("task"));
        // The `display: |` block scalar keeps its trailing newline.
        assert_eq!(row.body_bytes, "<b>{title}</b>\n".len());
        Ok(())
    }

    /// The standard library seeds twenty-five views. They are branch
    /// data like any other, so only the pin filter keeps them out.
    #[dialog_common::test]
    async fn it_omits_the_views_the_standard_library_seeds() -> Result<()> {
        let test = common::TestSite::new().await?;
        assert!(views::list(&test.site).await?.is_empty());

        test.eval_inline(common::ATTRIBUTE_DECL).await?;
        test.eval_inline(common::CONCEPT_DECL).await?;
        tonk_cli::data_ops::view_add(
            &test.site,
            "task",
            tonk_cli::authoring::ViewKind::Detail,
            "<b>{title}</b>",
            false,
            Default::default(),
        )
        .await?;

        let listed = views::list(&test.site).await?;
        assert!(
            listed
                .iter()
                .all(|row| row.entity.to_string() != "tonk:blob/media-view"),
            "seeded views should stay out of the listing: {:?}",
            listed
                .iter()
                .map(|row| row.entity.to_string())
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_surfaces_every_bookmarked_view() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(VIEW_DECL).await?;
        test.eval_inline(
            r#"view!: &todo-list
  body: "<ul><li>buy milk</li></ul>"
"#,
        )
        .await?;
        test.eval_inline(
            r#"view!: &welcome
  body: "<h1>hi</h1>"
"#,
        )
        .await?;

        let listed = views::list(&test.site).await?;
        assert_eq!(listed.len(), 2);
        // Alphabetical by name (matches the listing's sort).
        assert_eq!(listed[0].name.as_deref(), Some("todo-list"));
        assert_eq!(listed[1].name.as_deref(), Some("welcome"));
        assert_eq!(listed[0].body_bytes, "<ul><li>buy milk</li></ul>".len());
        assert_eq!(listed[1].body_bytes, "<h1>hi</h1>".len());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_resolves_a_bookmark_to_an_entity() -> Result<()> {
        let test = common::TestSite::new().await?;
        test.eval_inline(VIEW_DECL).await?;
        test.eval_inline(
            r#"view!: &welcome
  body: "<h1>hi</h1>"
"#,
        )
        .await?;

        let entity = views::entity_for_name(&test.site, "welcome")
            .await?
            .expect("welcome should resolve");
        assert!(views::entity_has_text_html(&test.site, &entity).await?);

        // Unknown bookmark returns None rather than erroring.
        assert!(views::entity_for_name(&test.site, "nope").await?.is_none());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_no_text_html_for_unrelated_entities() -> Result<()> {
        // A `task!` head produces an entity without a
        // `text/html` claim — `entity_has_text_html` should
        // return false for it.
        let test = common::TestSite::new().await?;
        test.eval_inline(common::ATTRIBUTE_DECL).await?;
        test.eval_inline(common::CONCEPT_DECL).await?;
        test.eval_inline(
            r#"task!: &buy-milk
  title: "Buy milk"
  done:  false
"#,
        )
        .await?;
        let task_entity = views::entity_for_name(&test.site, "buy-milk")
            .await?
            .expect("buy-milk should resolve");
        assert!(!views::entity_has_text_html(&test.site, &task_entity).await?);
        Ok(())
    }
}

mod when_a_device_has_no_account {
    use anyhow::Result;
    use tonk_cli::site::TonkSite;

    use crate::common;

    /// Creating a space mints durable authority: a `space → root` chain that a
    /// later `tonk invite` delegates from. Rooted in an anonymous key that
    /// chain has no owner, nothing can revoke it, and nothing backs up what it
    /// creates — so the account is the precondition, not the passkey.
    ///
    /// The error has to name the command that fixes it. `tonk identity link`,
    /// which the old message named, no longer exists.
    #[dialog_common::test]
    async fn it_refuses_to_create_a_space() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let mut config = common::isolated_config(&parent)?;
        config.require_account = true;

        let Err(error) = TonkSite::init_with(&parent, config).await else {
            panic!("a device with no account cannot create a space");
        };
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("tonk account login"),
            "the refusal must name the command that fixes it: {rendered}"
        );
        Ok(())
    }
}

mod when_migrating_from_carry {
    use anyhow::Result;
    use tonk_cli::migrate::{self, Mode};
    use tonk_cli::site::{SITE_DIRNAME, TonkSite};

    use crate::common;

    /// Build a real `.tonk/` then rename it `.carry/` so the
    /// test fixture is structurally identical to a carry-produced
    /// source — both tools share the dialog-repo on-disk layout.
    async fn carry_lookalike_at(parent: &std::path::Path) -> Result<dialog_capability::Did> {
        let config = common::isolated_config(parent)?;
        let original = TonkSite::init_with(parent, config).await?;
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

        let outcome =
            migrate::run_with(&parent, None, Mode::Copy, common::isolated_config(&parent)?).await?;
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
        let _site = TonkSite::init_with(&parent, config).await?;

        // Drop a placeholder .carry/ alongside it so the source
        // exists but is incompatible — the refuse-on-conflict
        // check happens before any copy work.
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)?;
        std::fs::write(carry_dir.join("placeholder"), b"")?;

        let result =
            migrate::run_with(&parent, None, Mode::Copy, common::isolated_config(&parent)?).await;
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

        let outcome =
            migrate::run_with(&parent, None, Mode::Move, common::isolated_config(&parent)?).await?;
        assert!(outcome.moved);
        assert_eq!(outcome.repo_did, original_did.to_string());
        assert!(!parent.join(".carry").exists());
        assert!(parent.join(SITE_DIRNAME).is_dir());
        Ok(())
    }
}

mod when_initializing_at_an_explicit_root {
    use anyhow::Result;
    use tonk_cli::site::TonkSite;

    use crate::common;

    /// Canonical spaces put repo blocks directly in the registered
    /// site directory — no `.tonk/` nesting. `init_at_with` must
    /// root the site at exactly the path it is given.
    #[dialog_common::test]
    async fn it_roots_the_site_at_the_given_directory() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;
        let root = parent.join("spaces").join("garden");

        let site = TonkSite::init_at_with(&root, config.clone()).await?;
        assert_eq!(site.root, root.canonicalize()?);
        assert!(!root.join(".tonk").exists(), "no nested .tonk");

        // Idempotent: a second init at the same root adopts the
        // existing repo instead of erroring or re-seeding.
        let reopened = TonkSite::init_at_with(&root, config.clone()).await?;
        assert_eq!(reopened.repository.did(), site.repository.did());

        // And a plain open works against it.
        let opened = TonkSite::open_with(&root, config).await?;
        assert_eq!(opened.repository.did(), site.repository.did());
        Ok(())
    }
}

mod when_mounting_account_authority {
    use anyhow::Result;
    use dialog_credentials::Ed25519Signer;
    use dialog_query::{Output as _, Query, Term};
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;
    use tonk_account::prefix::space_root_site;
    use tonk_cli::site::{self, TonkSite};
    use tonk_schema::{Invitation, InvitedVia, MemberName, MemberRole, Membership};

    use crate::common;

    /// The device's durable root: the onboarding account, read from its
    /// persisted grant's issuer — no passkey root exists in these tests.
    async fn local_root(site: &TonkSite) -> Result<dialog_varsig::Did> {
        let bytes = site
            .profile
            .credential()
            .site(tonk_cli::onboarding::ONBOARDING_GRANT_SITE)
            .load::<Vec<u8>>()
            .perform(&site.operator)
            .await?;
        let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(chain.issuer().clone())
    }

    async fn delegated_prefix(
        account_root: &dialog_varsig::Did,
    ) -> Result<(dialog_varsig::Did, DelegationChain)> {
        let space = Ed25519Signer::import(&[91; 32]).await?;
        let subject = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(account_root)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await?;
        Ok((subject, DelegationChain::new(delegation)))
    }

    #[dialog_common::test]
    async fn it_mounts_a_root_delegated_subject_without_inventing_roster_facts() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent = tmp.path().canonicalize()?;
        let config = common::isolated_config(&parent)?;
        let seed = TonkSite::init_at_with(&parent.join("seed"), config.clone()).await?;
        let account_root = local_root(&seed).await?;
        let (subject, chain) = delegated_prefix(&account_root).await?;
        let expected = chain.to_bytes()?;
        let root = parent.join("mounted");

        let mounted = site::mount_delegated_at(&root, chain, config).await?;
        assert_eq!(mounted.repository.did(), subject);
        let persisted = mounted
            .profile
            .credential()
            .site(space_root_site(&subject, &account_root))
            .load::<Vec<u8>>()
            .perform(&mounted.operator)
            .await?;
        assert_eq!(persisted, expected);

        let meta = mounted
            .repository
            .branch(tonk_cli::remote::META_BRANCH)
            .open()
            .perform(&mounted.operator)
            .await?;
        let memberships: Vec<Membership> = meta
            .query()
            .select(Query::<Membership> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                member: Term::var("member"),
            })
            .perform(&mounted.operator)
            .try_vec()
            .await?;
        let roles: Vec<MemberRole> = meta
            .query()
            .select(Query::<MemberRole> {
                this: Term::var("this"),
                role: Term::var("role"),
            })
            .perform(&mounted.operator)
            .try_vec()
            .await?;
        let names: Vec<MemberName> = meta
            .query()
            .select(Query::<MemberName> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&mounted.operator)
            .try_vec()
            .await?;
        let invitations: Vec<Invitation> = meta
            .query()
            .select(Query::<Invitation> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                inviter: Term::var("inviter"),
                audience: Term::var("audience"),
            })
            .perform(&mounted.operator)
            .try_vec()
            .await?;
        let provenance: Vec<InvitedVia> = meta
            .query()
            .select(Query::<InvitedVia> {
                this: Term::var("this"),
                invitation: Term::var("invitation"),
            })
            .perform(&mounted.operator)
            .try_vec()
            .await?;
        assert!(memberships.is_empty());
        assert!(roles.is_empty());
        assert!(names.is_empty());
        assert!(invitations.is_empty());
        assert!(provenance.is_empty());
        Ok(())
    }

    #[dialog_common::test]
    async fn it_recovers_a_pre_feature_prefix_from_profile_authority() -> Result<()> {
        let test = common::TestSite::new().await?;
        let root = local_root(&test.site).await?;
        let key = space_root_site(&test.site.repository.did(), &root);
        test.site
            .profile
            .credential()
            .site(key.clone())
            .save(Vec::<u8>::new())
            .perform(&test.site.operator)
            .await?;

        let recovered = site::account_root_prefix(&test.site, &root).await?;
        assert_eq!(recovered.subject(), Some(&test.site.repository.did()));
        assert_eq!(recovered.audience(), &root);
        let persisted = test
            .site
            .profile
            .credential()
            .site(key)
            .load::<Vec<u8>>()
            .perform(&test.site.operator)
            .await?;
        assert_eq!(persisted, recovered.to_bytes()?);
        Ok(())
    }

    /// A space created before this account existed has repository authority
    /// that stops at the profile and reaches no account root at all. The
    /// profile holds that authority, so it can extend it rather than
    /// stranding the space with nothing that can authorize its remote.
    #[dialog_common::test]
    async fn it_adopts_a_space_whose_authority_reaches_no_account_root() -> Result<()> {
        let test = common::TestSite::new().await?;
        let account_root = Ed25519Signer::import(&[73; 32]).await?.did();
        assert_ne!(local_root(&test.site).await?, account_root);

        let adopted = site::account_root_prefix(&test.site, &account_root).await?;

        assert_eq!(adopted.subject(), Some(&test.site.repository.did()));
        assert_eq!(adopted.audience(), &account_root);
        let persisted = test
            .site
            .profile
            .credential()
            .site(space_root_site(&test.site.repository.did(), &account_root))
            .load::<Vec<u8>>()
            .perform(&test.site.operator)
            .await?;
        assert_eq!(persisted, adopted.to_bytes()?);
        Ok(())
    }

    /// Adoption happens once: the second call must read the stored prefix
    /// rather than mint a second, differently-signed one.
    #[dialog_common::test]
    async fn it_reuses_an_adopted_prefix_on_later_authorizations() -> Result<()> {
        let test = common::TestSite::new().await?;
        let account_root = Ed25519Signer::import(&[74; 32]).await?.did();

        let first = site::account_root_prefix(&test.site, &account_root).await?;
        let second = site::account_root_prefix(&test.site, &account_root).await?;

        assert_eq!(first.to_bytes()?, second.to_bytes()?);
        Ok(())
    }
}
