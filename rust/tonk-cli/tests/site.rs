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
        // string — running `tonk init` produced a usable repo
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

    /// Full loop against a live local shortcut service: mint the long
    /// URL, shorten it (`PUT /@` on the link's own origin), then claim
    /// the short link — the claim resolves the 301 by hand, splicing
    /// the seed fragment back on the way a browser would.
    #[dialog_common::test]
    async fn it_shortens_and_claims_a_minted_invite(env: AccessServiceAddress) -> Result<()> {
        let endpoint = env.access_service_url.as_str();
        let inviter = common::TestSite::new().await?;
        remote::add(&inviter.site, "origin", endpoint, None).await?;

        let base = format!("{endpoint}/join");
        let outcome = invite::mint(&inviter.site, Some(&base), Some(endpoint)).await?;
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
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome = invite::claim(&claimer_parent, &short, claimer_config).await?;
        assert_eq!(claim_outcome.subject, inviter.site.repository.did());
        assert!(claim_outcome.remote_url.is_some());
        Ok(())
    }
}

mod when_claiming_an_invite_with_a_remote {
    use anyhow::Result;
    use tonk_cli::invite;
    use tonk_cli::remote;
    use tonk_cli::site::{SITE_DIRNAME, TonkSite};

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
            TonkSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "origin");
        assert_eq!(listed[0].endpoint, ENDPOINT);
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
        let claimer_config = common::isolated_config(&claimer_parent)?;
        let claim_outcome =
            invite::claim(&claimer_parent, &invite_outcome.url, claimer_config.clone()).await?;

        assert!(claim_outcome.auto_configured_remote.is_none());
        let joined =
            TonkSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
        let listed = remote::list(&joined).await?;
        assert!(listed.is_empty());
        Ok(())
    }
}

mod when_recording_roster_facts {
    use anyhow::Result;
    use dialog_query::{Output as _, Query, Term};
    use tonk_cli::invite;
    use tonk_cli::site::{SITE_DIRNAME, TonkSite};
    use tonk_invite::Invite;
    use tonk_schema::prelude::DidExt as _;
    use tonk_schema::{Invitation, InvitedVia, Membership};

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
        let claimer_config = common::isolated_config(&claimer_parent)?;
        invite::claim(&claimer_parent, &invite_outcome.url, claimer_config.clone()).await?;
        let joined =
            TonkSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;

        // Claimer side: membership + stamp referencing the same
        // invitation entity.
        let claimer_meta = joined
            .repository
            .branch(tonk_cli::remote::META_BRANCH)
            .open()
            .perform(&joined.operator)
            .await?;
        let memberships: Vec<Membership> = claimer_meta
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
        assert_eq!(memberships[0].member.0, joined.profile.did().this());

        let stamps: Vec<InvitedVia> = claimer_meta
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

        Ok(())
    }
}

mod when_minting_and_claiming_an_invite {
    use anyhow::Result;
    use tonk_cli::invite::{self, InviteError};
    use tonk_cli::site::{self, SITE_DIRNAME, TonkSite};

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
            TonkSite::open_with(&claimer_parent.join(SITE_DIRNAME), claimer_config).await?;
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

mod when_sharing_a_view {
    use anyhow::Result;
    use dialog_repository::Branch;
    use tonk_cli::remote;
    use tonk_cli::share::{self, ShareError, ShareOptions};
    use tonk_cli::views;
    use url::Url;

    use crate::common::{self, VIEW_DECL};

    const ENDPOINT: &str = "https://access.example.test/ucan/";

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

    /// Setup mirroring `when_sharing_a_concept::shareable_site`
    /// but seeded with a `view!: &my-view …` assertion.
    async fn shareable_view_site() -> Result<common::TestSite> {
        let test = common::TestSite::new().await?;
        test.eval_inline(VIEW_DECL).await?;
        test.eval_inline(
            r#"view!: &my-view
  body: "<h1>hello</h1>"
"#,
        )
        .await?;
        wire_local_upstream(&test).await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        Ok(test)
    }

    #[dialog_common::test]
    async fn it_produces_a_launcher_url_targeting_the_view_route() -> Result<()> {
        let test = shareable_view_site().await?;
        let outcome = share::share_view(
            &test.site,
            "my-view",
            ShareOptions {
                ui_base: Some("https://ui.example.test/join".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(outcome.view_name.as_deref(), Some("my-view"));
        assert_eq!(outcome.remote_name, "origin");
        // Entity resolved through the bookmark — confirm it
        // really does have a text/html claim, so a downstream
        // host-route fetch would resolve.
        assert!(views::entity_has_text_html(&test.site, &outcome.entity).await?);

        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs.get("then").map(String::as_str),
            Some(format!("view/{}", outcome.entity).as_str()),
        );
        assert_eq!(pairs.get("remote").map(String::as_str), Some(ENDPOINT));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_accepts_a_raw_entity_uri_target() -> Result<()> {
        let test = shareable_view_site().await?;
        let entity = views::entity_for_name(&test.site, "my-view")
            .await?
            .expect("seed asserted my-view");

        let outcome =
            share::share_view(&test.site, &entity.to_string(), ShareOptions::default()).await?;
        // Direct URI input → no bookmark name surfaced.
        assert!(outcome.view_name.is_none());
        assert_eq!(outcome.entity, entity);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_the_bookmark_does_not_resolve() -> Result<()> {
        let test = shareable_view_site().await?;
        let result = share::share_view(&test.site, "missing", ShareOptions::default()).await;
        match result {
            Err(ShareError::ViewNotFound { target }) => assert_eq!(target, "missing"),
            other => panic!("expected ViewNotFound, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_the_resolved_entity_has_no_text_html_claim() -> Result<()> {
        // Seed a task fixture alongside the view fixture; sharing
        // the task bookmark should fail because the task entity
        // doesn't carry a text/html claim — even though it does
        // resolve through the same `dialog.meta/name` index.
        let test = shareable_view_site().await?;
        test.eval_inline(common::ATTRIBUTE_DECL).await?;
        test.eval_inline(common::CONCEPT_DECL).await?;
        test.eval_inline(
            r#"task!: &buy-milk
  title: "Buy milk"
  done:  false
"#,
        )
        .await?;

        let result = share::share_view(&test.site, "buy-milk", ShareOptions::default()).await;
        assert!(
            matches!(result, Err(ShareError::NotAView { .. })),
            "expected NotAView, got: {result:?}",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_no_text_html_claim_exists_under_a_did_target() -> Result<()> {
        // Pass a syntactically valid did:key that doesn't
        // correspond to any view on the branch. Entity::from_str
        // is permissive enough to accept arbitrary did:foo:bar
        // shapes, so the guard that catches the typo is the
        // text/html presence check — surface that path here so
        // a regression in the guard light up the test.
        let test = shareable_view_site().await?;
        let result = share::share_view(
            &test.site,
            "did:key:z6MkfakeEntityForViewShareTest1111111111",
            ShareOptions::default(),
        )
        .await;
        assert!(
            matches!(result, Err(ShareError::NotAView { .. })),
            "expected NotAView, got: {result:?}",
        );
        Ok(())
    }
}

mod when_sharing_a_concept {
    use anyhow::Result;
    use dialog_repository::Branch;
    use tonk_cli::remote;
    use tonk_cli::share::{self, ShareError, ShareOptions};
    use url::Url;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    const ENDPOINT: &str = "https://access.example.test/ucan/";

    /// Open a sibling branch as the dialog upstream so
    /// `tonk push` resolves locally without needing a real
    /// access service.
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

    /// Build a test site with a `task` concept defined, one
    /// registered remote, and an in-process upstream wired up —
    /// the minimum viable setup for `share_concept`.
    async fn shareable_site() -> Result<common::TestSite> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        wire_local_upstream(&test).await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        Ok(test)
    }

    #[dialog_common::test]
    async fn it_produces_a_launcher_url_with_all_expected_pieces() -> Result<()> {
        let test = shareable_site().await?;
        let outcome = share::share_concept(
            &test.site,
            "task",
            ShareOptions {
                ui_base: Some("https://ui.example.test/join".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(outcome.concept_name, "task");
        assert_eq!(outcome.space_name, share::DEFAULT_SPACE_NAME);
        assert_eq!(outcome.remote_name, "origin");
        assert_eq!(outcome.remote_endpoint, ENDPOINT);

        let url = Url::parse(&outcome.url)?;
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("ui.example.test"));
        assert_eq!(url.path(), "/join");
        // Audience-open invites carry the seed in the fragment;
        // `share` builds on `invite::mint` so this should
        // round-trip as well.
        assert!(
            url.fragment().is_some(),
            "expected ephemeral seed in URL fragment, got: {}",
            outcome.url,
        );

        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(pairs.contains_key("access"), "missing access= param");
        assert_eq!(pairs.get("remote").map(String::as_str), Some(ENDPOINT));
        assert_eq!(
            pairs.get("name").map(String::as_str),
            Some(share::DEFAULT_SPACE_NAME),
        );
        // `then` is a path suffix relative to /space/<name>/, not
        // an absolute URL — tonk-ui composes the prefix using the
        // recipient's actual local name (which may differ from
        // `name=` when they already have the subject mounted).
        assert_eq!(pairs.get("then").map(String::as_str), Some("concept/task"),);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_honours_an_explicit_space_name() -> Result<()> {
        let test = shareable_site().await?;
        let outcome = share::share_concept(
            &test.site,
            "task",
            ShareOptions {
                space_name: Some("tasks".to_string()),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(outcome.space_name, "tasks");
        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(pairs.get("name").map(String::as_str), Some("tasks"));
        // `then` is independent of the suggested `name=` — it
        // names a path under whichever local space the recipient
        // ends up in.
        assert_eq!(pairs.get("then").map(String::as_str), Some("concept/task"),);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_the_concept_is_not_defined() -> Result<()> {
        let test = shareable_site().await?;
        let result = share::share_concept(&test.site, "nope", ShareOptions::default()).await;
        match result {
            Err(ShareError::ConceptNotFound { name }) => {
                assert_eq!(name, "nope");
            }
            other => panic!("expected ConceptNotFound, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_no_upstream_is_configured() -> Result<()> {
        // Schema + remote, but no `branch.set_upstream` call.
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;

        let result = share::share_concept(&test.site, "task", ShareOptions::default()).await;
        match result {
            Err(ShareError::UpstreamNotConfigured { branch }) => {
                assert_eq!(branch, "main");
            }
            other => panic!("expected UpstreamNotConfigured, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_no_remote_is_registered() -> Result<()> {
        // Schema present but no remote at all — share refuses
        // before even checking the upstream, since a remote-less
        // share would mint a URL the recipient can't pull from.
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        wire_local_upstream(&test).await?;

        let result = share::share_concept(&test.site, "task", ShareOptions::default()).await;
        assert!(
            matches!(result, Err(ShareError::NoRemote)),
            "expected NoRemote, got: {result:?}",
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_multiple_remotes_lack_an_explicit_choice() -> Result<()> {
        let test = shareable_site().await?;
        // Add a second remote so auto-selection is ambiguous.
        remote::add(
            &test.site,
            "backup",
            "https://backup.example.test/ucan/",
            None,
        )
        .await?;

        let result = share::share_concept(&test.site, "task", ShareOptions::default()).await;
        match result {
            Err(ShareError::AmbiguousRemote(names)) => {
                // List ordering tracks the meta branch — sort to
                // make the assertion stable regardless.
                let mut split: Vec<&str> = names.split(", ").collect();
                split.sort();
                assert_eq!(split, vec!["backup", "origin"]);
            }
            other => panic!("expected AmbiguousRemote, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_picks_an_explicit_remote_among_several() -> Result<()> {
        let test = shareable_site().await?;
        remote::add(
            &test.site,
            "backup",
            "https://backup.example.test/ucan/",
            None,
        )
        .await?;

        let outcome = share::share_concept(
            &test.site,
            "task",
            ShareOptions {
                remote: Some("backup".to_string()),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(outcome.remote_name, "backup");
        assert_eq!(outcome.remote_endpoint, "https://backup.example.test/ucan/");
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_an_explicit_remote_is_unknown() -> Result<()> {
        let test = shareable_site().await?;
        let result = share::share_concept(
            &test.site,
            "task",
            ShareOptions {
                remote: Some("missing".to_string()),
                ..Default::default()
            },
        )
        .await;
        match result {
            Err(ShareError::UnknownRemote(name)) => {
                assert_eq!(name, "missing");
            }
            other => panic!("expected UnknownRemote, got: {other:?}"),
        }
        Ok(())
    }
}

mod when_sharing_a_display {
    use anyhow::Result;
    use dialog_repository::Branch;
    use tonk_cli::remote;
    use tonk_cli::share::{self, ShareError, ShareOptions};
    use tonk_cli::views;
    use url::Url;

    use crate::common::{self, ATTRIBUTE_DECL, CONCEPT_DECL};

    const ENDPOINT: &str = "https://access.example.test/ucan/";

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

    /// Site with a `task` concept, a bookmarked `task` instance,
    /// one remote, and an in-process upstream — the shape
    /// `share_display` expects.
    async fn shareable_display_site() -> Result<common::TestSite> {
        let test = common::TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?;
        test.eval_inline(CONCEPT_DECL).await?;
        test.eval_inline(
            r#"task!: &buy-milk
  title: "Buy milk"
  done:  false
"#,
        )
        .await?;
        wire_local_upstream(&test).await?;
        remote::add(&test.site, "origin", ENDPOINT, None).await?;
        Ok(test)
    }

    #[dialog_common::test]
    async fn it_produces_a_launcher_url_with_view_and_model_in_the_then_suffix() -> Result<()> {
        let test = shareable_display_site().await?;
        let outcome = share::share_display(
            &test.site,
            "buy-milk",
            Some("basic"),
            Some("task"),
            ShareOptions {
                ui_base: Some("https://ui.example.test/join".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(outcome.subject_name.as_deref(), Some("buy-milk"));
        assert_eq!(outcome.view_name.as_deref(), Some("basic"));
        assert_eq!(outcome.model.as_deref(), Some("task"));
        assert_eq!(outcome.remote_name, "origin");

        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        // `then=` carries the bookmark (not the resolved entity)
        // so the URL survives entity-URI churn.
        assert_eq!(
            pairs.get("then").map(String::as_str),
            Some("buy-milk?view=basic&model=task"),
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_omits_query_params_when_neither_view_nor_model_is_supplied() -> Result<()> {
        let test = shareable_display_site().await?;
        let outcome =
            share::share_display(&test.site, "buy-milk", None, None, ShareOptions::default())
                .await?;
        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        // No trailing `?` when neither selector is present. The
        // library `share_display` stays permissive here (it only
        // composes the URL); the CLI is where exactly-one-of is
        // enforced, so a neither-selector launcher isn't reachable
        // from `tonk share display`.
        assert_eq!(pairs.get("then").map(String::as_str), Some("buy-milk"),);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_form_encodes_query_delimiters_in_a_view_name() -> Result<()> {
        // A view name carrying `&`/`?`/`=` must be form-encoded so it
        // can't corrupt or inject into the inner query string. The
        // round-trip through `Url::query_pairs` recovers the original.
        let test = shareable_display_site().await?;
        let outcome = share::share_display(
            &test.site,
            "buy-milk",
            Some("a&b=c?d"),
            None,
            ShareOptions::default(),
        )
        .await?;
        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let then = pairs.get("then").map(String::as_str).expect("then param");
        // The view value survives intact after tonk-ui would re-parse
        // the inner query, with no extra parameters leaking in.
        let inner = then.split_once('?').expect("inner query").1;
        let inner_pairs: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(inner.as_bytes())
                .into_owned()
                .collect();
        assert_eq!(inner_pairs.get("view").map(String::as_str), Some("a&b=c?d"));
        assert!(!inner_pairs.contains_key("b"));
        Ok(())
    }

    #[dialog_common::test]
    async fn it_accepts_a_raw_entity_uri_subject() -> Result<()> {
        let test = shareable_display_site().await?;
        let entity = views::entity_for_name(&test.site, "buy-milk")
            .await?
            .expect("buy-milk should resolve");
        let outcome = share::share_display(
            &test.site,
            &entity.to_string(),
            Some("basic"),
            Some("task"),
            ShareOptions::default(),
        )
        .await?;

        assert!(outcome.subject_name.is_none());
        assert_eq!(outcome.subject_entity, entity);
        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        // URI subjects land in `then=` verbatim — no bookmark to
        // prefer over them.
        let expected = format!("{entity}?view=basic&model=task");
        assert_eq!(
            pairs.get("then").map(String::as_str),
            Some(expected.as_str())
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_the_subject_bookmark_does_not_resolve() -> Result<()> {
        let test = shareable_display_site().await?;
        let result = share::share_display(
            &test.site,
            "ghost",
            Some("basic"),
            Some("task"),
            ShareOptions::default(),
        )
        .await;
        match result {
            Err(ShareError::SubjectNotFound { target }) => assert_eq!(target, "ghost"),
            other => panic!("expected SubjectNotFound, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_errors_when_the_model_concept_is_not_defined() -> Result<()> {
        let test = shareable_display_site().await?;
        let result = share::share_display(
            &test.site,
            "buy-milk",
            Some("basic"),
            Some("nope"),
            ShareOptions::default(),
        )
        .await;
        match result {
            Err(ShareError::ConceptNotFound { name }) => assert_eq!(name, "nope"),
            other => panic!("expected ConceptNotFound, got: {other:?}"),
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_accepts_a_uri_shaped_model_without_validation() -> Result<()> {
        // A `:`-bearing model passes through without a concept
        // lookup. Mirrors the convention for `did:key:…` subjects.
        let test = shareable_display_site().await?;
        let outcome = share::share_display(
            &test.site,
            "buy-milk",
            None,
            Some("did:key:zPretendConcept"),
            ShareOptions::default(),
        )
        .await?;
        let url = Url::parse(&outcome.url)?;
        let pairs: std::collections::HashMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs.get("then").map(String::as_str),
            Some("buy-milk?model=did%3Akey%3AzPretendConcept"),
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
        let _site = TonkSite::init_with(&parent, config).await?;

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
