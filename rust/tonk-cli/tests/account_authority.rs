//! Live coverage for authorizing a spot's remote under an account.

mod common;

use anyhow::Result;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_account::backup::space_root_site;
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
/// credential, so upgrading left every spot they created with nothing under
/// that key. Authorization has to rebuild the prefix from the certificates
/// the profile already holds instead of reporting the spot as undelegated.
#[dialog_common::test]
async fn it_pushes_a_spot_whose_account_prefix_was_never_stored(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
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

/// Creating a spot retains its authority into the account space.
///
/// The retain itself is `tonk_account::delegations`, shared with the worker so
/// the two adapters cannot drift into retaining different things. This pins
/// the CLI's half of that wiring; `it_recovers_space_access_on_a_second_device`
/// proves the retained facts actually travel.
#[dialog_common::test]
async fn it_retains_a_created_spot_into_the_account_space() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("retained"),
        account_config(&fixture),
    )
    .await?;
    assert!(
        !site.repository.did().to_string().is_empty(),
        "spot creation must succeed"
    );

    let operator = fixture.pre_account_site.operator.inner();
    assert!(
        tonk_cli::account_state::retain_space_delegation(&fixture.profile, operator, &fixture.link)
            .await?,
        "a hydrated account must retain the grant"
    );
    // Content-addressed, so a second retain of the same chain writes nothing.
    assert!(
        !tonk_cli::account_state::retain_space_delegation(
            &fixture.profile,
            operator,
            &fixture.link
        )
        .await?,
        "re-retaining an identical chain must not write again"
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
    let operator = fixture.pre_account_site.operator.inner();

    // Exactly what the page mints: the account's powerline to this profile,
    // plus the descriptor that says where the account repository lives.
    let root = tonk_identity::derive::derive_root_signer(&[77; 32]).await?;
    let authorized =
        tonk_identity::ceremony::authorize_device(root.clone(), fixture.profile.did(), &remote)
            .await?;
    let payload = serde_json::json!({
        "delegationHex": authorized.delegation_hex,
        "descriptorHex": authorized.descriptor_hex,
        "credentialId": authorized.root_did,
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
    let authorization = callback.receive().await?;
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

    // Device one: an account, and a spot whose authority it retains there.
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let first = common::AccountFixture::with_account_remote(&remote).await?;
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
        tonk_account::delegations::retain_space_delegation(&account, &chain, operator).await?,
        "device one retains its space authority into the account"
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
/// each spot into the account space. A fresh profile has neither, so the run
/// must succeed and report zero rather than erroring — which is what makes it
/// safe to re-run.
#[dialog_common::test]
async fn it_migrates_delegations_idempotently() -> Result<()> {
    use dialog_storage::provider::storage::{NativeSpace, Storage};

    let fixture = common::AccountFixture::new().await?;
    let store = tonk_cli::spot::SpotStore::at(fixture.tmp.path().join("registry"));
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
    assert_eq!(second.spots, 0, "no spot may be retained twice");
    Ok(())
}

/// A spot created before the account existed chains to the device root this
/// profile held at the time and reaches no account root at all. Linking an
/// account must adopt it rather than strand it offline.
#[dialog_common::test]
async fn it_pushes_a_spot_created_before_the_account_existed(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let path = fixture.pre_account_site.root.clone();
    let site = TonkSite::open_with(&path, account_config(&fixture)).await?;
    configure_upstream(&site, &env.access_service_url).await?;

    tonk_cli::sync::push(&site).await?;

    let prefix = tonk_cli::site::account_root_prefix(&site, fixture.link.issuer()).await?;
    assert_eq!(prefix.subject(), Some(&site.repository.did()));
    assert_eq!(prefix.audience(), fixture.link.issuer());
    Ok(())
}
