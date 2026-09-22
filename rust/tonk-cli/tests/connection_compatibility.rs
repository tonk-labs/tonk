//! Run explicitly with TONK_OLD_CLI pointing to a verified released executable.
//! The regular suite does not download or execute an external binary.

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::storage::Directory;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal;

#[tokio::test]
#[ignore = "requires verified released executable in TONK_OLD_CLI"]
async fn connection_outer_layout_prevents_old_cli_ambient_authority_fallback() -> Result<()> {
    let binary = std::env::var("TONK_OLD_CLI")?;
    let temp = tempfile::tempdir()?;
    let store = tonk_cli::space::SpaceStore::at(temp.path().join("state"));
    let owner = Ed25519Signer::generate().await?;
    let recipient_seed = [84; 32];
    let recipient = Ed25519Signer::import(&recipient_seed).await?;
    let endpoint = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let scopes = tonk_invite::connection::candidate_build_scopes(&owner.did());
    let expiry = Timestamp::new(SystemTime::now() + Duration::from_secs(90 * 86400))?;
    let mut chains = Vec::new();
    for scope in &scopes {
        chains.push(DelegationChain::new(
            DelegationBuilder::new()
                .issuer(Signer::from(owner.clone()))
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .meta(tonk_invite::home_address_meta(&endpoint))
                .expiration(expiry)
                .try_build()
                .await?,
        ));
    }
    let invitation = tonk_invite::connection::AgentInvite::new(
        recipient_seed,
        chains,
        &scopes,
        &endpoint,
        Timestamp::now(),
    )
    .await?;
    let link = invitation.to_url("http://127.0.0.1:9/connect")?;
    let validated = tonk_cli::connections::validate_link(&link, &endpoint).await?;
    let outer = temp.path().join("connection");
    let binding = tonk_cli::connections::import_at(&outer, &validated, store.clone()).await?;
    tonk_cli::space::register_connection_bound(&store, "scoped", &outer, None, binding.clone())?;
    let site = tonk_cli::connections::open_bound(&outer, &binding, store.clone()).await?;
    assert_eq!(site.profile.did(), recipient.did());
    tonk_cli::eval::run_against_site(&site,
        tonk_cli::eval::Source::Inline("attribute!: &retained\n  description: Retained offline data\n  the: example.retained/value\n  as: text\n  cardinality: one\n".into()),
        tonk_cli::eval::Options::default()).await?;
    let before = site.branch().await?.handle().revision().unwrap().tree;
    assert!(!outer.join("main").exists());

    // Give the old executable's default profile broader authority to this same
    // subject. The outer layout must prevent reaching it even in permissive
    // legacy device-root mode; lack of ambient proof is not the reason for denial.
    let home = temp.path().join("old-home");
    #[cfg(target_os = "macos")]
    let profile_parent = home.join("Library/Application Support/dialog");
    #[cfg(not(target_os = "macos"))]
    let profile_parent = home.join("data/dialog");
    std::fs::create_dir_all(&profile_parent)?;
    let storage = Storage::<NativeSpace>::default();
    let profile = dialog_peer::Peer::new()
        .storage(storage.clone())
        .create(dialog_effects::storage::Location::new(
            Directory::At(profile_parent.to_string_lossy().into_owned()),
            tonk_cli::site::PROFILE_NAME,
        ))
        .await?;
    let base = home.join("legacy-data");
    std::fs::create_dir_all(&base)?;
    let peer =
        tonk_cli::peer::peer_for(&profile, Directory::At(base.to_string_lossy().into_owned()))
            .await?;
    let operator = peer.session("legacy-compatibility").build().await?;
    let ambient = DelegationBuilder::new()
        .issuer(Signer::from(owner))
        .audience(&profile.did())
        .subject(Subject::Specific(site.repository.did()))
        .command(vec!["use".into()])
        .expiration(expiry)
        .try_build()
        .await?;
    profile
        .access()
        .save(UcanDelegation(DelegationChain::new(ambient)))
        .perform(&operator)
        .await?;

    let mut old_cli = std::process::Command::new(binary);
    old_cli
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPACES_STATE", store.root())
        .env("TONK_UNSAFE_ALLOW_DEVICE_ROOT", "1")
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env_remove("TONK_SPACE");
    old_cli.arg("identity");
    let identity = old_cli.output()?;
    assert!(identity.status.success());
    assert!(
        String::from_utf8_lossy(&identity.stdout).contains(&format!("device: {}", profile.did())),
        "released CLI must use the prepared ambient profile: {}",
        String::from_utf8_lossy(&identity.stdout)
    );
    // Command has no argument reset API; clone only the verified environment.
    let output = std::process::Command::new(old_cli.get_program())
        .args(["--space", "scoped", "push"])
        .current_dir(&home)
        .envs(
            old_cli
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env_remove("TONK_SPACE")
        .output()?;
    assert!(
        !output.status.success(),
        "old CLI must refuse the outer scoped layout"
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("failed to load repository 'main'"),
        "{error}"
    );
    assert!(!outer.join("main").exists());
    let reopened = tonk_cli::connections::open_bound(&outer, &binding, store).await?;
    assert_eq!(
        reopened.branch().await?.handle().revision().unwrap().tree,
        before
    );
    Ok(())
}
