//! Live coverage for authorizing a space's remote under an account.

mod common;

use anyhow::{Context, Result};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_account::prefix::space_root_site;
use tonk_cli::site::{SiteConfig, TonkSite};

/// Open sites the way a real install does, with the account boundary in
/// front of every remote fork. The shared fixture config leaves it off so
/// that tests without an account can still run.
fn account_config(fixture: &common::AccountFixture) -> SiteConfig {
    SiteConfig {
        require_account: true,
        ..fixture.config.clone()
    }
}

async fn configure_upstream(site: &TonkSite, endpoint: &str) -> Result<()> {
    tonk_cli::remote::add(site, "origin", endpoint, Some(site.repository.did())).await?;
    tonk_cli::remote::set_upstream(site, "origin").await?;
    Ok(())
}

/// Releases before the account-root prefix existed stored no such
/// credential, so upgrading left every space they created with nothing under
/// that key. Authorization has to rebuild the prefix from the certificates
/// the profile already holds instead of reporting the space as undelegated.
#[dialog_common::test]
async fn it_pushes_a_space_whose_account_prefix_was_never_stored(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    // The access service serves nothing for an account that has not
    // confirmed its email.
    fixture.activate_with(&env).await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("upgraded"),
        account_config(&fixture),
    )
    .await?;
    let prefix_site = space_root_site(&site.repository.did(), fixture.link.issuer());
    fixture
        .profile
        .credential()
        .site(prefix_site.clone())
        .save(Vec::<u8>::new())
        .perform(&site.operator)
        .await?;
    configure_upstream(&site, &env.access_service_url).await?;
    // What is under test is push authority, not provisioning; the space
    // still has to be someone's to serve.
    env.provision_subject(site.repository.did().as_str())
        .await?;

    tonk_cli::sync::push(&site).await?;

    let restored = fixture
        .profile
        .credential()
        .site(prefix_site)
        .load::<Vec<u8>>()
        .perform(&site.operator)
        .await?;
    assert!(
        !restored.is_empty(),
        "authorizing a remote must leave the recovered prefix stored"
    );
    Ok(())
}

/// Creating a space retains its authority into the account space.
///
/// The retain itself is `tonk_account::delegations`, shared with the worker so
/// the two adapters cannot drift into retaining different things. This pins
/// the CLI's half of that wiring; `it_recovers_space_access_on_a_second_device`
/// proves the retained facts actually travel.
#[dialog_common::test]
async fn it_retains_a_created_space_into_the_account_space(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    assert_eq!(
        tonk_cli::account_state::status_in(&fixture.profile, &fixture.config.account_store).await?,
        tonk_account::AccountStateStatus::Ready
    );
    let account_operator = tonk_cli::account_state::credential_operator_for_store(
        &fixture.profile,
        &fixture.config.account_store,
    )
    .await?;
    assert!(
        tonk_cli::account_state::open_account_branch_in(
            &fixture.profile,
            &account_operator,
            &fixture.config.account_store,
        )
        .await?
        .is_some()
    );
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("retained"),
        account_config(&fixture),
    )
    .await?;
    assert!(
        !site.repository.did().to_string().is_empty(),
        "space creation must succeed"
    );

    let operator = fixture.pre_account_site.operator.inner();
    // The fixture already put the root → device link into provable
    // reach, so retaining it reports already-present.
    assert!(
        !tonk_cli::account_state::retain_space_delegation(
            &fixture.profile,
            operator,
            &fixture.link
        )
        .await?,
        "the fixture's link is already retained"
    );
    // A chain the account has never seen retains once, and the
    // content-addressed store dedupes the replay.
    let (_, fresh) = fixture.space_chain(77).await?;
    assert!(
        tonk_cli::account_state::retain_space_delegation(&fixture.profile, operator, &fresh)
            .await?,
        "a hydrated account must retain a novel grant"
    );
    assert!(
        !tonk_cli::account_state::retain_space_delegation(&fixture.profile, operator, &fresh)
            .await?,
        "re-retaining an identical chain must not write again"
    );
    Ok(())
}

/// Every account ensure repairs both halves of the account/profile union.
///
/// Login can be cancelled after its durable handoff but before these edges
/// are retained. Keeping convergence in ensure means `account status` and
/// `account sync` repair that interrupted work without minting another
/// semantically identical return edge on every retry.
#[dialog_common::test]
async fn it_converges_the_linked_account_union_during_every_ensure(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let operator = fixture.operator().await?;
    let account = fixture.account_branch().await?;
    let root = fixture.link.issuer().clone();
    let return_scope = dialog_ucan::Scope {
        subject: dialog_ucan_core::subject::Subject::Specific(fixture.profile.did()),
        command: dialog_ucan_core::command::Command::parse("/")?,
        parameters: dialog_ucan::Parameters::default(),
    };

    assert!(
        account
            .delegations()
            .prove(root.clone(), return_scope.clone())
            .perform(&operator)
            .await
            .is_err(),
        "the fixture must begin without the profile return edge"
    );

    let outcome = tonk_cli::account_state::ensure_with_operator_and_store(
        &fixture.profile,
        operator.clone(),
        fixture.store.clone(),
    )
    .await?;
    assert_eq!(outcome.status, tonk_account::AccountStateStatus::Ready);
    let account = fixture.account_branch().await?;
    assert!(
        !tonk_account::delegations::retain_space_delegation(&account, &fixture.link, &operator)
            .await?,
        "ensure must already have retained the exact account-to-profile grant"
    );
    assert!(
        account
            .delegations()
            .prove(root, return_scope)
            .perform(&operator)
            .await
            .is_ok(),
        "ensure must retain a profile-to-account return edge"
    );

    let revision = account.revision();
    tonk_cli::account_state::ensure_with_operator_and_store(
        &fixture.profile,
        operator,
        fixture.store.clone(),
    )
    .await?;
    let account = fixture.account_branch().await?;
    assert_eq!(
        account.revision(),
        revision,
        "a repeated ensure must not mint a duplicate return edge"
    );
    Ok(())
}

/// `account link --via` installs the authority a browser hands back.
///
/// The ceremony itself needs a browser, but the CONTRACT does not: the page
/// posts a base64 payload carrying a delegation and a descriptor, and
/// anything that can sign can produce one. Standing in for the browser here
/// pins the CLI's whole half — validation, descriptor persistence, the union
/// edge, and the retain — without a WebAuthn dependency. The browser's half
/// is covered by the e2e suite, which drives a real ceremony.
#[dialog_common::test]
async fn it_installs_authority_from_a_callback_authorization(
    env: AccessServiceAddress,
) -> Result<()> {
    use base64::Engine as _;
    use dialog_varsig::Principal as _;

    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    // The access service serves nothing for an account that has not
    // confirmed its email.
    fixture.activate_with(&env).await?;
    let operator = fixture.pre_account_site.operator.inner();

    // Exactly what the page mints: the account's powerline to this profile,
    // plus the descriptor that says where the account repository lives.
    let root = dialog_credentials::Ed25519Signer::import(&[77; 32]).await?;
    let authorized =
        tonk_identity::ceremony::authorize_device(root.clone(), fixture.profile.did(), &remote)
            .await?;
    let payload = serde_json::json!({
        "delegationHex": authorized.delegation_hex,
        "remote": remote,
        "credentialId": authorized.root_did,
        "attachmentId": "4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d",
    })
    .to_string();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&payload);

    // Drive the CLI's receiving half against that payload.
    let callback = tonk_cli::callback::Callback::bind().await?;
    let url = callback.url().to_owned();
    let posting = tokio::spawn(async move {
        reqwest::Client::new()
            .post(&url)
            .form(&[("authorize", encoded)])
            .send()
            .await
            .unwrap();
    });
    let authorization = callback.receive(None).await?;
    posting.await?;

    let bytes = match authorization {
        tonk_cli::callback::Authorization::Granted(bytes) => bytes,
        tonk_cli::callback::Authorization::Denied(reason) => {
            anyhow::bail!("the authorizer declined: {reason}")
        }
    };
    assert_eq!(
        String::from_utf8(bytes)?,
        payload,
        "the payload must reach the CLI byte-identical"
    );

    // Install both halves the way the CLI does, then prove the result: the
    // account's grant and this profile's return edge both readable from the
    // account space, which is what makes a later device inherit them.
    let chain = tonk_cli::account::validate_account_grant(
        &fixture.profile,
        &hex::decode(&authorized.delegation_hex)?,
    )
    .await?;
    assert_eq!(
        chain.issuer(),
        &root.did(),
        "the account root must be the issuer"
    );
    let union = tonk_account::delegations::mint_account_union(
        &fixture.profile.signer().signer().clone(),
        &root.did(),
    )
    .await?;

    let account = tonk_cli::account_state::open_account_branch(&fixture.profile, operator)
        .await?
        .expect("the fixture's account is hydrated");
    for edge in [chain, union] {
        assert!(
            tonk_account::delegations::retain_space_delegation(&account, &edge, operator).await?,
            "both halves of the union must be retained into the account"
        );
    }

    // The account can now prove it may act for this profile — the return
    // edge, which is the half nothing else in the suite covers.
    let proof = account
        .delegations()
        .prove(
            root.did(),
            dialog_ucan::Scope {
                subject: dialog_ucan_core::subject::Subject::Specific(fixture.profile.did()),
                command: dialog_ucan_core::command::Command::parse("/")?,
                parameters: dialog_ucan::Parameters::default(),
            },
        )
        .perform(operator)
        .await;
    assert!(
        proof.is_ok(),
        "the retained union must let the account act for this profile: {:?}",
        proof.err()
    );

    // Finally: exercise the capability rather than only proving it resolves.
    // `account_config` puts the account boundary in front of every remote
    // fork, so this push authorizes against a chain reaching the account
    // root and fails outright if that authority is absent.
    //
    // What this does NOT isolate: the fixture already holds an equivalent
    // `root -> profile` link, derived from the same passkey, so the push
    // would also succeed on that. Proving the callback-delivered grant is
    // separately sufficient needs a profile with no prior account, which is
    // what the browser e2e covers — it authorizes a CLI profile that never
    // linked.
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("authorized-space"),
        account_config(&fixture),
    )
    .await?;
    configure_upstream(&site, &env.access_service_url).await?;
    env.provision_subject(site.repository.did().as_str())
        .await?;
    tonk_cli::sync::push(&site)
        .await
        .context("a space must push under authority that reaches the account root")?;

    Ok(())
}

/// A profile can accept space authority before it has an account. Linking
/// later adds the profile-to-account half of the union, so the same local
/// space can authorize account-bound sync without being re-claimed or moved.
#[dialog_common::test]
async fn it_syncs_a_claimed_space_after_linking_an_account(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let account_owner = common::AccountFixture::with_account_remote(&remote).await?;
    account_owner.activate_with(&env).await?;
    let owner_operator = account_owner.pre_account_site.operator.inner();
    let owner_account =
        tonk_cli::account_state::open_account_branch(&account_owner.profile, owner_operator)
            .await?
            .expect("the account owner is hydrated");
    owner_account.push().perform(owner_operator).await?;

    let inviter = common::TestSite::new().await?;
    let invitation = tonk_cli::invite::mint(&inviter.site, None, None).await?;
    let claimer_tmp = tempfile::tempdir()?;
    let claimer_parent = claimer_tmp.path().canonicalize()?;
    let claimer_config = common::isolated_config(&claimer_parent)?;
    let joined_root = claimer_parent.join("claimed-before-link");
    let mut production_config = claimer_config.clone();
    production_config.require_account = true;
    let claimed =
        tonk_cli::invite::claim(&joined_root, &invitation.url, production_config.clone()).await?;

    let joined = TonkSite::open_with(&joined_root, production_config.clone()).await?;
    // The pre-link claim terminates at the ONBOARDING account: durable
    // before any passkey, and carried forward by the sign-in union.
    let member = tonk_cli::site::member_did(&joined).await?;
    assert_ne!(
        member,
        joined.profile.did(),
        "the pre-link invite terminates at the onboarding account, not the bare profile"
    );
    let page = common::authorizing_page(account_owner.root_signer().await?, remote.clone()).await?;
    let (announce, mut announced) = tokio::sync::mpsc::unbounded_channel();
    let approving = tokio::spawn(async move {
        if let Some(url) = announced.recv().await {
            let _ = reqwest::Client::new().get(&url).send().await;
        }
    });
    let linked = tonk_cli::account::link_with_operator(
        &joined.profile,
        joined.operator.inner(),
        &tonk_cli::account::LinkOptions {
            service_url: remote,
            device_name: "pre-link-claimer".to_owned(),
            open_browser: false,
            via: Some(page.url.clone()),
            announce: Some(announce),
            store: Some(claimer_config.account_store.clone()),
        },
    )
    .await?;
    approving.await?;
    assert_eq!(
        linked.account_state,
        tonk_account::AccountStateStatus::Ready,
        "linking must hydrate the account: {:?}",
        linked.warning
    );
    let account_root: dialog_varsig::Did = linked.root_did.parse()?;
    tonk_cli::site::load_account_root_prefix_for(
        &joined.profile,
        joined.operator.inner(),
        &claimed.subject,
        &account_root,
    )
    .await
    .context("linking must make the profile-ending invite chain reach the account root")?;

    let joined = TonkSite::open_with(&joined_root, production_config).await?;
    configure_upstream(&joined, &env.access_service_url).await?;
    env.provision_subject(claimed.subject.as_str()).await?;
    tonk_cli::sync::push(&joined)
        .await
        .context("the linked account must authorize the profile's pre-link invite")?;

    Ok(())
}

/// Discovering a space through the account: the flow that makes linking
/// worth anything.
///
/// One device creates a space and delegates it to the account, retained in
/// the account repository. A SECOND profile — which never created that space
/// and holds no authority over it — links to the same account, pulls, and can
/// then reach it. That is the whole promise of the account being the durable
/// home of delegations, and unlike the tests above it isolates the received
/// authority: the linking profile starts with nothing.
#[dialog_common::test]
async fn it_discovers_a_space_through_the_account(env: AccessServiceAddress) -> Result<()> {
    use dialog_varsig::Principal as _;

    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let owner = common::AccountFixture::with_account_remote(&remote).await?;
    // The access service serves nothing for an account that has not
    // confirmed its email.
    owner.activate_with(&env).await?;
    let owner_operator = owner.pre_account_site.operator.inner();
    let account_root = owner.link.issuer().clone();

    // The owner creates a space and puts its authority in the account.
    let site = TonkSite::init_at_with(
        &owner.tmp.path().join("shared-space"),
        account_config(&owner),
    )
    .await?;
    configure_upstream(&site, &env.access_service_url).await?;
    let subject = site.repository.did();
    let prefix = tonk_cli::site::account_root_prefix_for(
        &owner.profile,
        owner_operator,
        &subject,
        &account_root,
    )
    .await?;
    let account = tonk_cli::account_state::open_account_branch(&owner.profile, owner_operator)
        .await?
        .expect("the owner's account is hydrated");
    assert!(
        !tonk_account::delegations::retain_space_delegation(&account, &prefix, owner_operator)
            .await?,
        "creation already retained the space authority into the account"
    );
    account.push().perform(owner_operator).await?;

    // A second profile, with no account and no knowledge of that space.
    let joiner = common::TestSite::new().await?;
    let joiner_profile = joiner.site.profile.clone();
    let joiner_operator = joiner.site.operator.inner();
    let union_scope = || dialog_ucan::Scope {
        subject: dialog_ucan_core::subject::Subject::Any,
        command: dialog_ucan_core::command::Command::parse("/").expect("root command"),
        parameters: dialog_ucan::Parameters::default(),
    };
    let scope = || dialog_ucan::Scope {
        subject: dialog_ucan_core::subject::Subject::Specific(subject.clone()),
        command: dialog_ucan_core::command::Command::parse("/").expect("root command"),
        parameters: dialog_ucan::Parameters::default(),
    };
    let joiner_access = dialog_repository::Repository::from(&joiner_profile)
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(joiner_operator)
        .await?;
    assert!(
        joiner_access
            .delegations()
            .prove(joiner_profile.did(), scope())
            .perform(joiner_operator)
            .await
            .is_err(),
        "a profile that has not linked must not reach a space it never created"
    );

    // Run the REAL `account link --via` against a stand-in page: a tiny
    // server that answers the approval URL by doing exactly what the browser
    // does — mint the grant, post it to the callback. Everything after that
    // is the command's own dance: install, persist, mount, adopt, pull,
    // retain both union halves, push.
    let root = dialog_credentials::Ed25519Signer::import(&[77; 32]).await?;
    let page = common::authorizing_page(root.clone(), remote.clone()).await?;
    // Nothing opens a browser in a test, so take the approval URL off the
    // announce channel and visit it — the URL carries the callback address
    // only `link` knows, which is why a test cannot construct it.
    let (announce, mut announced) = tokio::sync::mpsc::unbounded_channel();
    let approving = tokio::spawn(async move {
        if let Some(url) = announced.recv().await {
            let _ = reqwest::Client::new().get(&url).send().await;
        }
    });
    let outcome = tonk_cli::account::link_with_operator(
        &joiner_profile,
        joiner_operator,
        &tonk_cli::account::LinkOptions {
            service_url: remote.clone(),
            device_name: "test-device".to_owned(),
            open_browser: false,
            via: Some(page.url.clone()),
            announce: Some(announce),
            store: Some(tonk_cli::space::SpaceStore::at(
                joiner.parent.join("account-state"),
            )),
        },
    )
    .await?;
    approving.await?;
    assert_eq!(
        outcome.account_state,
        tonk_account::AccountStateStatus::Ready,
        "linking must leave the account ready: {:?}",
        outcome.warning
    );
    assert_eq!(
        outcome.root_did,
        root.did().to_string(),
        "linking must record the account that authorized it"
    );

    // Re-open the access branch: linking advanced its head.
    let joiner_access = dialog_repository::Repository::from(&joiner_profile)
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(joiner_operator)
        .await?;

    let proof = joiner_access
        .delegations()
        .prove(joiner_profile.did(), scope())
        .perform(joiner_operator)
        .await;
    assert!(
        proof.is_ok(),
        "after linking and pulling, the profile must reach the space it \
         discovered through the account: {:?}",
        proof.err()
    );

    // Linking is not one-directional: the CLI writes BOTH halves of the
    // union into the account and pushes, so the account records the device
    // rather than only the device recording the account. Without this a
    // third device pulling the account would never learn this one exists.
    // `ensure` is what a linked device runs: it mounts the account, adopts
    // it as the access upstream, and syncs. Going through it here rather
    // than reaching for the branch directly means the test exercises the
    // same path `link --via` does, including the operator refresh that lets
    // the newly installed grant authorize the pull.
    tonk_cli::account_state::ensure_with_operator(&joiner_profile, joiner_operator.clone()).await?;
    let joiner_account = tonk_cli::account_state::open_account_branch_in(
        &joiner_profile,
        joiner_operator,
        &tonk_cli::space::SpaceStore::at(joiner.parent.join("account-state")),
    )
    .await?
    .expect("linking hydrates the joiner's account");
    let union = tonk_account::delegations::mint_account_union(
        &joiner_profile.signer().signer().clone(),
        &account_root,
    )
    .await?;
    assert!(
        tonk_account::delegations::retain_space_delegation(
            &joiner_account,
            &union,
            joiner_operator
        )
        .await?,
        "the profile's return edge must reach the account"
    );
    joiner_account.push().perform(joiner_operator).await?;

    // Push, then confirm the branch is durable by reading it back from the
    // OWNER, who has authority over the account and is a genuinely separate
    // view of it. A third party without that authority could not read it at
    // all, so it would prove nothing about the push.
    joiner_account.push().perform(joiner_operator).await?;
    let owner_view = tonk_cli::account_state::open_account_branch(&owner.profile, owner_operator)
        .await?
        .expect("the owner's account is hydrated");
    owner_view.pull().perform(owner_operator).await?;
    let observed = owner_view
        .delegations()
        .prove(account_root.clone(), union_scope())
        .perform(owner_operator)
        .await;
    assert!(
        observed.is_ok(),
        "the profile's return edge must be visible to the account after the \
         push, or a third device would never learn this one exists: {:?}",
        observed.err()
    );

    Ok(())
}

/// The whole loop over a live access service and S3: a space created on one
/// device is recoverable on a SECOND device that has never seen it, by pulling
/// the account.
///
/// This is the design end to end — `space -> account -> profile`. Device one
/// retains its space authority into the account and pushes. Device two is a
/// genuinely separate install (its own profile directory and storage) that
/// derives the SAME account root, which is the property a shared passkey
/// gives it. It adopts the account as its access-branch upstream, pulls, and
/// only then can prove access. No backup artifact and no chain fetch: recovery
/// is syncing a branch.
#[dialog_common::test]
async fn it_recovers_space_access_on_a_second_device(env: AccessServiceAddress) -> Result<()> {
    use dialog_ucan::{Parameters, Scope};
    use dialog_ucan_core::command::Command;
    use dialog_ucan_core::subject::Subject as UcanSubject;

    // Device one: an account, and a space whose authority it retains there.
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let first = common::AccountFixture::with_account_remote(&remote).await?;
    // The access service serves nothing for an account that has not
    // confirmed its email.
    first.activate_with(&env).await?;
    let site = TonkSite::init_at_with(&first.tmp.path().join("device-one"), account_config(&first))
        .await?;
    configure_upstream(&site, &env.access_service_url).await?;
    let subject = site.repository.did();

    let account_root = first.link.issuer().clone();
    let operator = first.pre_account_site.operator.inner();
    let account = tonk_cli::account_state::open_account_branch(&first.profile, operator)
        .await?
        .expect("device one has a hydrated account");
    // The space -> account-root prefix: the authority device two needs and
    // cannot mint for itself. `tonk space create` retains exactly this.
    let chain =
        tonk_cli::site::account_root_prefix_for(&first.profile, operator, &subject, &account_root)
            .await?;
    assert!(
        !tonk_account::delegations::retain_space_delegation(&account, &chain, operator).await?,
        "creation already retained device one's space authority into the account"
    );
    account.push().perform(operator).await?;

    // Device two: a separate install deriving the same account root. It must
    // not already prove the space — it has never seen it.
    let second = common::AccountFixture::with_account_remote(&remote).await?;
    let second_operator = second.pre_account_site.operator.inner();
    let second_account =
        tonk_cli::account_state::open_account_branch(&second.profile, second_operator)
            .await?
            .expect("device two mounts the same account");
    let scope = || Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse("/").expect("root command parses"),
        parameters: Parameters::default(),
    };

    // The operator resolves proofs from its OWN access branch, not the
    // account's, so pulling the account is not enough on its own: the access
    // branch has to adopt the account as its upstream and pull too. That is
    // what makes recovered authority usable rather than merely present.
    let second_access = dialog_repository::Repository::from(&second.profile)
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(second_operator)
        .await?;
    let second_remote = dialog_repository::Repository::from(&second.profile)
        .remote("account")
        .create(dialog_repository::SiteAddress::from(
            dialog_remote_ucan_s3::UcanAddress::new(&remote),
        ))
        .subject(account_root.clone())
        .perform(second_operator)
        .await?
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(second_operator)
        .await?;
    assert!(
        second_access
            .delegations()
            .prove(account_root.clone(), scope())
            .perform(second_operator)
            .await
            .is_err(),
        "a device that has not pulled must not already prove the space"
    );

    tonk_account::delegations::adopt_account_upstream(
        &second_access,
        &second_remote,
        second_operator,
    )
    .await?;

    // The pull IS the recovery.
    second_account.pull().perform(second_operator).await?;
    // Prove from the ACCESS branch, which is where the operator resolves
    // proofs at runtime — proving from the account branch alone would show
    // the data arrived without showing it is usable.
    let proof = second_access
        .delegations()
        .prove(account_root, scope())
        .perform(second_operator)
        .await;
    assert!(
        proof.is_ok(),
        "after pulling, device two proves the space through its access branch: {:?}",
        proof.err()
    );
    Ok(())
}

/// Migrating is safe to run with nothing to migrate, and reports so.
///
/// The command exists for profiles that predate delegations being facts: it
/// drains the legacy certificate store into the access branch and retains
/// each space into the account space. A fresh profile has neither, so the run
/// must succeed and report zero rather than erroring — which is what makes it
/// safe to re-run.
#[dialog_common::test]
async fn it_migrates_delegations_idempotently() -> Result<()> {
    use dialog_storage::provider::storage::{NativeSpace, Storage};

    let fixture = common::AccountFixture::new().await?;
    let store = tonk_cli::space::SpaceStore::at(fixture.tmp.path().join("registry"));
    // Mount the fixture's profile so migration has a provider for its
    // subject: it commits as the profile, and an unmounted one errors.
    let storage = Storage::<NativeSpace>::default();
    let profile = dialog_operator::Profile::load(&fixture.config.profile_name)
        .at(fixture.config.profile_directory.clone())
        .perform(&storage)
        .await?;

    let first = tonk_cli::account_state::migrate_delegations(
        &profile,
        fixture.pre_account_site.operator.inner(),
        &storage,
        &store,
    )
    .await?;

    // Re-running must not double-count: the certificate store was drained by
    // the first pass, and retaining is content-addressed.
    let second = tonk_cli::account_state::migrate_delegations(
        &profile,
        fixture.pre_account_site.operator.inner(),
        &storage,
        &store,
    )
    .await?;
    assert_eq!(
        second.certificates, 0,
        "a drained certificate store has nothing left to migrate; first pass moved {}",
        first.certificates
    );
    assert_eq!(second.spaces, 0, "no space may be retained twice");
    Ok(())
}

/// A space created before the account existed reaches no account root.
/// Ordinary sync must not silently turn account linking into ownership
/// adoption; `tonk space move` is the explicit boundary that does so.
#[dialog_common::test]
async fn it_denies_ordinary_sync_for_a_space_created_before_the_account_existed(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    // The access service serves nothing for an account that has not
    // confirmed its email.
    fixture.activate_with(&env).await?;
    let path = fixture.pre_account_site.root.clone();
    let site = TonkSite::open_with(&path, account_config(&fixture)).await?;
    configure_upstream(&site, &env.access_service_url).await?;
    env.provision_subject(site.repository.did().as_str())
        .await?;

    let error = tonk_cli::sync::push(&site)
        .await
        .expect_err("ordinary sync cannot adopt a local-only space");
    assert!(
        error.to_string().contains("No delegation chain proves"),
        "{error}"
    );
    Ok(())
}

/// An account-backed create seals the space's seed to the account's
/// published encryption key and records it on the account branch — the
/// copy any of the account's devices recovers the space from after a
/// ceremony opens it.
#[dialog_common::test]
async fn it_custodies_the_created_space_seed() -> Result<()> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{SecretMessage, SecretPrincipal, prelude::DidExt as _};

    let fixture = common::AccountFixture::new().await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("custodied"),
        account_config(&fixture),
    )
    .await?;
    let subject = site.repository.did();

    let account_operator =
        tonk_cli::account_state::credential_operator_for_store(&fixture.profile, &fixture.store)
            .await?;
    let account = tonk_cli::account_state::open_account_branch_in(
        &fixture.profile,
        &account_operator,
        &fixture.store,
    )
    .await?
    .context("the fixture account branch mounts")?;
    // The principal names the message; the message carries the seed.
    let principals: Vec<SecretPrincipal> = account
        .query()
        .select(Query::<SecretPrincipal> {
            this: Term::from(subject.this()),
            kind: Term::var("kind"),
            seed: Term::var("seed"),
        })
        .perform(&account_operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("read sealed principals: {error:?}"))?;
    assert_eq!(principals.len(), 1, "the created space's seed is sealed");
    assert_eq!(
        principals[0].kind.0.to_string(),
        tonk_schema::SeedKind::SPACE
    );
    let rows: Vec<SecretMessage> = account
        .query()
        .select(Query::<SecretMessage> {
            this: Term::from(principals[0].seed.0.clone()),
            to: Term::var("to"),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(&account_operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("read sealed messages: {error:?}"))?;
    assert_eq!(rows.len(), 1, "the principal names a real message");

    // The account secret opens the row and derives the space itself.
    let secret = tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new(
        fixture.root_prf,
    ));
    let sealed = tonk_identity::sealed::Sealed::decode(&rows[0].message.0)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let seed = secret
        .secret()
        .reveal(&sealed, &subject)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let signer = dialog_credentials::Ed25519Signer::import(&*seed).await?;
    use dialog_varsig::Principal as _;
    assert_eq!(signer.did(), subject, "the sealed seed derives the space");
    Ok(())
}

/// Sign-in moves custody with two passes. The shared rotation core
/// rotates whatever the onboarding account sealed (here the fixture's
/// pre-account site, created before any root existed) and retires the
/// onboarding account; the legacy walk then exports spaces that predate
/// custody rows entirely — "premade", created with a root recorded but
/// no account gate. Hosting moves in neither pass; that stays
/// `tonk space link`'s boundary.
#[dialog_common::test]
async fn it_moves_local_space_custody_at_sign_in() -> Result<()> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_cli::custody::{SpaceRotation, rotate_from_onboarding, rotate_local_spaces};
    use tonk_schema::{SecretMessage, SecretPrincipal, prelude::DidExt as _};

    let fixture = common::AccountFixture::new().await?;
    // What `tonk account login` records once the ceremony succeeds.
    let root_did_string = fixture.link.issuer().to_string();
    fixture
        .store
        .set_account(Some(tonk_cli::space::AccountRecord::new(root_did_string)))?;
    let mut local_config = fixture.config.clone();
    local_config.require_account = false;
    let created =
        tonk_cli::space::create(&fixture.store, "premade", None, None, local_config).await?;
    let subject: dialog_varsig::Did = created.did.parse()?;

    // The login sequence: the shared core rotates the onboarding-sealed
    // seeds, then the legacy walk covers anything without a custody row.
    // The fixture's attach recorded a root before this create, so
    // "premade" is the LEGACY shape: root-delegated, no custody row.
    // The rotation pass moves the genuinely onboarding-custodied seed
    // (the fixture's pre-account site) and the walk exports this one.
    let failures = rotate_from_onboarding(&fixture.store, &fixture.config).await?;
    assert!(failures.is_empty(), "rotation completes: {failures:?}");
    let outcomes = rotate_local_spaces(&fixture.store, &fixture.config).await?;
    assert!(
        outcomes
            .iter()
            .any(|(name, outcome)| name == "premade" && matches!(outcome, SpaceRotation::Moved)),
        "the walk moves the legacy space: {outcomes:?}"
    );

    let account_operator =
        tonk_cli::account_state::credential_operator_for_store(&fixture.profile, &fixture.store)
            .await?;
    let account = tonk_cli::account_state::open_account_branch_in(
        &fixture.profile,
        &account_operator,
        &fixture.store,
    )
    .await?
    .context("the fixture account branch mounts")?;
    let principals: Vec<SecretPrincipal> = account
        .query()
        .select(Query::<SecretPrincipal> {
            this: Term::from(subject.this()),
            kind: Term::var("kind"),
            seed: Term::var("seed"),
        })
        .perform(&account_operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("read sealed principals: {error:?}"))?;
    assert_eq!(principals.len(), 1, "the moved space's seed is sealed");
    let rows: Vec<SecretMessage> = account
        .query()
        .select(Query::<SecretMessage> {
            this: Term::from(principals[0].seed.0.clone()),
            to: Term::var("to"),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(&account_operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("read sealed messages: {error:?}"))?;
    assert_eq!(rows.len(), 1, "the principal names a real message");

    let secret = tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new(
        fixture.root_prf,
    ));
    let sealed = tonk_identity::sealed::Sealed::decode(&rows[0].message.0)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let seed = secret
        .secret()
        .reveal(&sealed, &subject)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let signer = dialog_credentials::Ed25519Signer::import(&*seed).await?;
    use dialog_varsig::Principal as _;
    assert_eq!(signer.did(), subject, "the sealed seed derives the space");

    // Running again converges: the onboarding account is retired, so
    // the rotation finds nothing, and the walk still reports the row.
    let failures = rotate_from_onboarding(&fixture.store, &fixture.config).await?;
    assert!(
        failures.is_empty(),
        "a second rotation is a no-op: {failures:?}"
    );
    let again = rotate_local_spaces(&fixture.store, &fixture.config).await?;
    assert!(
        again
            .iter()
            .any(|(name, outcome)| name == "premade" && matches!(outcome, SpaceRotation::Already)),
        "a second sign-in finds custody already moved: {again:?}"
    );
    Ok(())
}
